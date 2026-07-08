//! Terminal rendering for jesterky runs.
//!
//! Two renderers:
//! - [`render_tree`] — the minimal deterministic `ProcessNode` indent (kept for
//!   `print_manifest`).
//! - [`render_run_view`] — a **btop-style** panel: a rounded border with an inset
//!   title, gradient progress bars per map phase, and a colored item tree.
//!   [`adapt_manifest`] folds a finished [`RunManifest`] into the [`RunView`] it
//!   draws (post-hoc). The renderer is pure `view -> String` and IO-free; ANSI is
//!   opt-in via [`RenderOpts::color`] (the CLI disables it off a TTY / `NO_COLOR`).

use jesterky_contract::{
    CallKind, EventKind, NodePath, PathSeg, ProcessNode, RunManifest, WorkflowSpec,
};

// ─────────────────────────── minimal tree (unchanged) ───────────────────────

/// Render a process tree into deterministic, indented terminal text.
pub fn render_tree(root: &ProcessNode) -> String {
    let mut out = String::new();
    render_node(root, 0, &mut out);
    out
}

fn render_node(node: &ProcessNode, depth: usize, out: &mut String) {
    out.push_str(&"  ".repeat(depth));
    out.push_str(&node.label);
    if let Some(score) = node.score {
        out.push_str(&format!(" score={score:.3}"));
    }
    out.push_str(&format!(" artifacts={}", node.artifacts.len()));
    out.push('\n');

    for child in &node.children {
        render_node(child, depth + 1, out);
    }
}

// ───────────────────────────────── view model ───────────────────────────────

/// A run reduced to what the panel draws: a header, one phase per map node, and
/// a result line.
#[derive(Debug, Clone, PartialEq)]
pub struct RunView {
    pub title: String,
    pub agents: usize,
    pub concurrency: Option<u32>,
    pub model: Option<String>,
    pub phases: Vec<PhaseView>,
    pub outcome: Outcome,
    pub result_note: String,
}

/// One map collection: its rollup and its items.
#[derive(Debug, Clone, PartialEq)]
pub struct PhaseView {
    pub label: String,
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub live: usize,
    pub items: Vec<ItemView>,
}

/// One map item (a shard / agent).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub index: u32,
    pub status: ItemStatus,
    /// Compact right-hand detail (e.g. `pass  security`).
    pub label: String,
    pub tone: Tone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Queued,
    Running,
    Done,
    Failed,
}

/// Semantic color of an item's label, independent of execution status (a scan
/// item can *complete* yet carry a `fail` verdict).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Bad,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Running,
    Completed,
    Failed,
}

// ───────────────────────────────── adapter ──────────────────────────────────

/// Fold a finished manifest (+ optional spec, for concurrency) into a [`RunView`].
pub fn adapt_manifest(manifest: &RunManifest, spec: Option<&WorkflowSpec>) -> RunView {
    let mut phases: Vec<(Vec<PathSeg>, PhaseView)> = Vec::new();

    for event in &manifest.events {
        let Some((parent, index)) = split_item(&event.addr.node_path) else {
            continue;
        };
        if !matches!(
            event.kind,
            EventKind::MapItemStarted | EventKind::MapItemCompleted | EventKind::MapItemFailed
        ) {
            continue;
        }
        let phase = match phases.iter_mut().find(|(key, _)| *key == parent) {
            Some((_, phase)) => phase,
            None => {
                phases.push((
                    parent.clone(),
                    PhaseView {
                        label: seg_label(parent.last()),
                        total: 0,
                        done: 0,
                        failed: 0,
                        live: 0,
                        items: Vec::new(),
                    },
                ));
                &mut phases.last_mut().unwrap().1
            }
        };
        let status = match event.kind {
            EventKind::MapItemCompleted => ItemStatus::Done,
            EventKind::MapItemFailed => ItemStatus::Failed,
            _ => ItemStatus::Running,
        };
        match phase.items.iter_mut().find(|item| item.index == index) {
            Some(item) => item.status = status,
            None => phase.items.push(ItemView {
                index,
                status,
                label: String::new(),
                tone: Tone::Neutral,
            }),
        }
    }

    // Attach a compact label from the recorded actor output at each item path,
    // and tally the rollup.
    let mut agents = 0usize;
    for (parent, phase) in &mut phases {
        for item in &mut phase.items {
            let mut item_path = parent.clone();
            item_path.push(PathSeg::Index(item.index));
            if let Some((label, tone)) = item_detail(manifest, &item_path) {
                item.label = label;
                item.tone = tone;
            }
        }
        phase.items.sort_by_key(|item| item.index);
        phase.total = phase.items.len();
        phase.done = count(phase, ItemStatus::Done);
        phase.failed = count(phase, ItemStatus::Failed);
        phase.live = count(phase, ItemStatus::Running);
        agents += phase.total;
    }

    let outcome = if manifest
        .events
        .iter()
        .any(|e| e.kind == EventKind::WorkflowFailed)
    {
        Outcome::Failed
    } else if manifest
        .events
        .iter()
        .any(|e| e.kind == EventKind::WorkflowCompleted)
    {
        Outcome::Completed
    } else {
        Outcome::Running
    };

    RunView {
        title: manifest.workflow_name.clone(),
        agents,
        concurrency: spec.and_then(|s| s.runplan.map_concurrency),
        model: None,
        phases: phases.into_iter().map(|(_, phase)| phase).collect(),
        outcome,
        result_note: result_note(manifest),
    }
}

fn count(phase: &PhaseView, status: ItemStatus) -> usize {
    phase.items.iter().filter(|i| i.status == status).count()
}

/// A compact label + tone from the recorded actor output at (or under) an item.
fn item_detail(manifest: &RunManifest, item_path: &[PathSeg]) -> Option<(String, Tone)> {
    let recorded = manifest.recorded.iter().find(|r| {
        matches!(r.call, CallKind::Actor { .. }) && starts_with(&r.addr.node_path, item_path)
    })?;
    let outputs = &recorded.outputs;
    if let Some(verdict) = outputs.get("verdict").and_then(|v| v.as_str()) {
        let tone = match verdict {
            "fail" => Tone::Bad,
            "pass" => Tone::Good,
            _ => Tone::Neutral,
        };
        let mut label = verdict.to_string();
        if let Some(dim) = outputs.get("dimension").and_then(|v| v.as_str()) {
            label = format!("{label:<5} {dim}");
        }
        return Some((label, tone));
    }
    Some(("ok".to_string(), Tone::Neutral))
}

/// The result line: prefer a `summary_recorder`-style report, else empty.
fn result_note(manifest: &RunManifest) -> String {
    for r in &manifest.recorded {
        let o = &r.outputs;
        if let (Some(passed), Some(failed)) = (
            o.get("passed").and_then(|v| v.as_u64()),
            o.get("failed").and_then(|v| v.as_u64()),
        ) {
            return format!("pass {passed} · fail {failed}");
        }
    }
    String::new()
}

fn split_item(path: &NodePath) -> Option<(Vec<PathSeg>, u32)> {
    match path.0.last() {
        Some(PathSeg::Index(i)) => Some((path.0[..path.0.len() - 1].to_vec(), *i)),
        _ => None,
    }
}

fn starts_with(path: &NodePath, prefix: &[PathSeg]) -> bool {
    path.0.len() >= prefix.len() && path.0[..prefix.len()] == *prefix
}

fn seg_label(seg: Option<&PathSeg>) -> String {
    match seg {
        Some(PathSeg::Node(name)) => name.clone(),
        Some(PathSeg::Index(i)) => format!("[{i}]"),
        None => "map".to_string(),
    }
}

// ────────────────────────────── styled renderer ─────────────────────────────

#[derive(Debug, Clone, Copy)]
struct Rgb(u8, u8, u8);

// A tokyo-night / btop-ish dark palette.
const FG: Rgb = Rgb(0xc0, 0xca, 0xf5);
const DIM: Rgb = Rgb(0x56, 0x5f, 0x89);
const BORDER: Rgb = Rgb(0x3b, 0x42, 0x61);
const CYAN: Rgb = Rgb(0x7d, 0xcf, 0xff);
const GREEN: Rgb = Rgb(0x9e, 0xce, 0x6a);
const YELLOW: Rgb = Rgb(0xe0, 0xaf, 0x68);
const RED: Rgb = Rgb(0xf7, 0x76, 0x8e);
// Progress-bar gradient (fills cyan → green, btop-style).
const GRAD_LO: Rgb = Rgb(0x2a, 0xc3, 0xde);
const GRAD_HI: Rgb = Rgb(0x9e, 0xce, 0x6a);

#[derive(Debug, Clone, Copy, Default)]
struct Style {
    fg: Option<Rgb>,
    bold: bool,
    dim: bool,
}

impl Style {
    fn fg(rgb: Rgb) -> Self {
        Self {
            fg: Some(rgb),
            ..Self::default()
        }
    }
    fn bold(rgb: Rgb) -> Self {
        Self {
            fg: Some(rgb),
            bold: true,
            dim: false,
        }
    }
    fn faint() -> Self {
        Self {
            fg: Some(DIM),
            bold: false,
            dim: true,
        }
    }
}

struct Span {
    text: String,
    style: Style,
}

/// A line as styled spans; knows its visible width so the panel can pad it.
#[derive(Default)]
struct Line(Vec<Span>);

impl Line {
    fn push(&mut self, text: impl Into<String>, style: Style) -> &mut Self {
        self.0.push(Span {
            text: text.into(),
            style,
        });
        self
    }
    fn width(&self) -> usize {
        self.0.iter().map(|s| s.text.chars().count()).sum()
    }
    fn render(&self, color: bool) -> String {
        let mut out = String::new();
        for span in &self.0 {
            if color {
                let code = ansi(span.style);
                if code.is_empty() {
                    out.push_str(&span.text);
                } else {
                    out.push_str(&code);
                    out.push_str(&span.text);
                    out.push_str("\x1b[0m");
                }
            } else {
                out.push_str(&span.text);
            }
        }
        out
    }
}

fn ansi(style: Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    if style.bold {
        parts.push("1".to_string());
    }
    if style.dim {
        parts.push("2".to_string());
    }
    if let Some(Rgb(r, g, b)) = style.fg {
        parts.push(format!("38;2;{r};{g};{b}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", parts.join(";"))
    }
}

fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    Rgb(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

const FRAC: [&str; 8] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];

/// A gradient completion bar with a fractional trailing cell (btop-style).
fn gradient_bar(ratio: f64, width: usize) -> Line {
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = ratio * width as f64;
    let full = filled.floor() as usize;
    let frac = filled - full as f64;
    let mut line = Line::default();
    for i in 0..width {
        let t = if width > 1 {
            i as f64 / (width - 1) as f64
        } else {
            0.0
        };
        if i < full {
            line.push("█", Style::fg(lerp(GRAD_LO, GRAD_HI, t)));
        } else if i == full && frac > 0.06 {
            let idx = ((frac * 8.0).round() as usize).clamp(1, 7);
            line.push(FRAC[idx], Style::fg(lerp(GRAD_LO, GRAD_HI, t)));
        } else {
            line.push("█", Style::fg(BORDER));
        }
    }
    line
}

/// How wide the panel renders, and whether to emit ANSI color.
#[derive(Debug, Clone, Copy)]
pub struct RenderOpts {
    pub width: usize,
    pub color: bool,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            width: 76,
            color: true,
        }
    }
}

/// Render a [`RunView`] as a btop-style panel.
pub fn render_run_view(view: &RunView, opts: &RenderOpts) -> String {
    let width = opts.width.max(40);
    let inner = width - 4; // content between "│ " and " │"

    let mut body: Vec<Line> = Vec::new();
    body.push(Line::default());

    for phase in &view.phases {
        // Phase row: glyph · label · gradient bar · rollup.
        let ratio = if phase.total > 0 {
            phase.done as f64 / phase.total as f64
        } else {
            0.0
        };
        let mut row = Line::default();
        row.push("◆ ", Style::fg(CYAN));
        row.push(format!("{:<10} ", truncate(&phase.label, 10)), Style::bold(FG));
        for span in gradient_bar(ratio, 22).0 {
            row.0.push(span);
        }
        row.push(format!("  {}/{}", phase.done, phase.total), Style::fg(FG));
        if phase.live > 0 {
            row.push(format!(" · {} live", phase.live), Style::fg(YELLOW));
        }
        if phase.failed > 0 {
            row.push(format!(" · {} fail", phase.failed), Style::fg(RED));
        }
        body.push(row);

        // Item rows.
        for (n, item) in phase.items.iter().enumerate() {
            let connector = if n + 1 == phase.items.len() {
                "  ╰ "
            } else {
                "  ├ "
            };
            let (glyph, gstyle) = status_glyph(item.status);
            let tone = match item.tone {
                Tone::Good => GREEN,
                Tone::Bad => RED,
                Tone::Neutral => FG,
            };
            let mut row = Line::default();
            row.push(connector, Style::fg(BORDER));
            row.push(format!("[{:>2}] ", item.index), Style::faint());
            row.push(format!("{glyph} "), gstyle);
            row.push(item.label.clone(), Style::fg(tone));
            body.push(row);
        }
        body.push(Line::default());
    }

    // Result row.
    let mut result = Line::default();
    let (word, style) = match view.outcome {
        Outcome::Completed => ("completed", Style::bold(GREEN)),
        Outcome::Failed => ("failed", Style::bold(RED)),
        Outcome::Running => ("running", Style::bold(YELLOW)),
    };
    result.push("result  ", Style::faint());
    result.push(word, style);
    if !view.result_note.is_empty() {
        result.push("  ·  ", Style::faint());
        result.push(view.result_note.clone(), Style::fg(FG));
    }
    body.push(result);

    // Assemble the panel.
    let meta = header_meta(view);
    let mut out = String::new();
    out.push_str(&top_border(&view.title, &meta, width, opts.color));
    out.push('\n');
    for line in body {
        let mut framed = Line::default();
        framed.push("│ ", Style::fg(BORDER));
        let content_width = line.width();
        for span in line.0 {
            framed.0.push(span);
        }
        let pad = inner.saturating_sub(content_width);
        if pad > 0 {
            framed.push(" ".repeat(pad), Style::default());
        }
        framed.push(" │", Style::fg(BORDER));
        out.push_str(&framed.render(opts.color));
        out.push('\n');
    }
    out.push_str(&bottom_border(width, opts.color));
    out.push('\n');
    out
}

fn header_meta(view: &RunView) -> String {
    let mut parts = vec![format!("{} agents", view.agents)];
    if let Some(c) = view.concurrency {
        parts.push(format!("{c}-wide"));
    }
    if let Some(model) = &view.model {
        parts.push(model.clone());
    }
    parts.join(" · ")
}

fn status_glyph(status: ItemStatus) -> (&'static str, Style) {
    match status {
        ItemStatus::Done => ("✓", Style::fg(GREEN)),
        ItemStatus::Failed => ("✗", Style::fg(RED)),
        ItemStatus::Running => ("●", Style::fg(YELLOW)),
        ItemStatus::Queued => ("○", Style::faint()),
    }
}

fn top_border(title: &str, meta: &str, width: usize, color: bool) -> String {
    let mut line = Line::default();
    line.push("╭─┤ ", Style::fg(BORDER));
    line.push(title.to_string(), Style::bold(CYAN));
    line.push(" ├", Style::fg(BORDER));
    // Visible so far: "╭─┤ " (4) + title + " ├" (2). Reserve the right meta block
    // "┤ {meta} ├─" (meta+5) and the closing "╮" (1).
    let right = if meta.is_empty() { 0 } else { meta.chars().count() + 5 };
    let used = 4 + title.chars().count() + 2 + right + 1;
    let dashes = width.saturating_sub(used);
    line.push("─".repeat(dashes), Style::fg(BORDER));
    if !meta.is_empty() {
        line.push("┤ ", Style::fg(BORDER));
        line.push(meta.to_string(), Style::faint());
        line.push(" ├─", Style::fg(BORDER));
    }
    line.push("╮", Style::fg(BORDER));
    line.render(color)
}

fn bottom_border(width: usize, color: bool) -> String {
    let mut line = Line::default();
    line.push("╰", Style::fg(BORDER));
    line.push("─".repeat(width.saturating_sub(2)), Style::fg(BORDER));
    line.push("╯", Style::fg(BORDER));
    line.render(color)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jesterky_contract::{Addr, ArtifactRef, NodePath, ProcessNode};
    use serde_json::json;

    #[test]
    fn render_tree_is_deterministic_indented_text() {
        let root = ProcessNode {
            addr: addr(0),
            label: "workflow:quality".to_string(),
            inputs: json!({}),
            outputs: json!({}),
            score: Some(0.875),
            signal: None,
            artifacts: vec![artifact("blob/root")],
            children: vec![
                ProcessNode {
                    addr: addr(1),
                    label: "expand".to_string(),
                    inputs: json!({}),
                    outputs: json!({}),
                    score: None,
                    signal: None,
                    artifacts: Vec::new(),
                    children: Vec::new(),
                },
                ProcessNode {
                    addr: addr(2),
                    label: "actor:scanner".to_string(),
                    inputs: json!({}),
                    outputs: json!({ "ok": true }),
                    score: Some(0.5),
                    signal: None,
                    artifacts: vec![artifact("blob/scan")],
                    children: Vec::new(),
                },
            ],
        };

        const EXPECTED: &str = "\
workflow:quality score=0.875 artifacts=1
  expand artifacts=0
  actor:scanner score=0.500 artifacts=1
";

        assert_eq!(render_tree(&root), EXPECTED);
    }

    #[test]
    fn run_view_panel_is_stable_without_color() {
        let view = RunView {
            title: "quality_scan".to_string(),
            agents: 4,
            concurrency: Some(2),
            model: Some("deepseek".to_string()),
            phases: vec![PhaseView {
                label: "scan_jobs".to_string(),
                total: 4,
                done: 3,
                failed: 0,
                live: 1,
                items: vec![
                    ItemView { index: 0, status: ItemStatus::Done, label: "pass  correctness".to_string(), tone: Tone::Good },
                    ItemView { index: 1, status: ItemStatus::Done, label: "fail  security".to_string(), tone: Tone::Bad },
                    ItemView { index: 2, status: ItemStatus::Done, label: "pass  tests".to_string(), tone: Tone::Good },
                    ItemView { index: 3, status: ItemStatus::Running, label: String::new(), tone: Tone::Neutral },
                ],
            }],
            outcome: Outcome::Running,
            result_note: "pass 2 · fail 1".to_string(),
        };
        let out = render_run_view(&view, &RenderOpts { width: 60, color: false });
        assert!(out.contains("╭─┤ quality_scan ├"));
        assert!(out.contains("4 agents · 2-wide · deepseek"));
        assert!(out.contains("◆ scan_jobs"));
        // Item 1 *completed* (glyph ✓) but its verdict is `fail` (red label) —
        // execution status and verdict are distinct.
        assert!(out.contains("├ [ 1] ✓ fail  security"));
        assert!(out.contains("╰ [ 3] ●"));
        assert!(out.contains("result  running  ·  pass 2 · fail 1"));
        assert!(out.contains("╰─"));
        // No ANSI escapes when color is off.
        assert!(!out.contains('\x1b'));
        // Every framed line is the same visible width.
        let widths: Vec<usize> = out
            .lines()
            .filter(|l| l.starts_with('│'))
            .map(|l| l.chars().count())
            .collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged panel: {widths:?}"
        );
    }

    fn addr(local_seq: u32) -> Addr {
        Addr {
            run_id: "render-run".to_string(),
            node_path: NodePath::root(),
            iteration: 0,
            local_seq,
        }
    }

    fn artifact(key: &str) -> ArtifactRef {
        ArtifactRef {
            key: key.to_string(),
            size_bytes: 10,
            content_type: "application/json".to_string(),
        }
    }
}
