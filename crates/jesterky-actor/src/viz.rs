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
    BudgetSnapshot, BudgetState, CallKind, EventKind, NodeKind, NodePath, PathSeg, ProcessNode,
    RunManifest, ShardProgress, WorkflowSpec,
};
use std::collections::HashMap;

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
    /// Formal resource budgets (progress + ETA) when the run declared them.
    pub budgets: Option<BudgetSnapshot>,
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
    /// Live usage folded from the shard's [`ShardProgress`] (`0` when unknown /
    /// post-hoc from a manifest that carries no live progress).
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub steps: u32,
    /// Wall-clock seconds the shard has been (or was) running, if known.
    pub elapsed_secs: Option<f64>,
    /// The shard's latest action while running (`reading runner.rs`).
    pub detail: String,
}

impl ItemView {
    pub fn tokens(&self) -> u64 {
        self.tokens_in + self.tokens_out
    }
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
///
/// When `item_labels` is set, empty item rows are filled from that slice by index
/// (used during live `--follow` before recorded outputs land).
///
/// When `progress` is set (live follow / final frame), each item's tokens, steps,
/// elapsed, and latest action are folded from the shard's [`ShardProgress`].
pub fn adapt_manifest(
    manifest: &RunManifest,
    spec: Option<&WorkflowSpec>,
    item_labels: Option<&[String]>,
    progress: Option<&HashMap<NodePath, ShardProgress>>,
) -> RunView {
    // DungeonGrid is one workflow over a party: fold turn-map shards into one
    // lane per hero (round-robin), not one row per scheduled turn.
    if manifest.workflow_name.starts_with("dungeongrid") {
        return adapt_dungeongrid_party(manifest, spec, item_labels, progress);
    }

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
        // On failure, carry a concise reason from the event payload so the row
        // says *why* it died instead of a bare ✗.
        let fail_reason = if event.kind == EventKind::MapItemFailed {
            event
                .payload
                .get("error")
                .and_then(|v| v.as_str())
                .map(fail_annotation)
        } else {
            None
        };
        match phase.items.iter_mut().find(|item| item.index == index) {
            Some(item) => {
                item.status = status;
                if let Some(reason) = &fail_reason {
                    item.detail = reason.clone();
                    item.tone = Tone::Bad;
                }
            }
            None => phase.items.push(ItemView {
                index,
                status,
                label: String::new(),
                tone: if fail_reason.is_some() {
                    Tone::Bad
                } else {
                    Tone::Neutral
                },
                tokens_in: 0,
                tokens_out: 0,
                steps: 0,
                elapsed_secs: None,
                detail: fail_reason.unwrap_or_default(),
            }),
        }
    }

    preseed_items(&mut phases, spec, item_labels);

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
            if let Some(progress) = progress {
                fold_progress(item, progress, &item_path);
            }
        }
        phase.items.sort_by_key(|item| item.index);
        if let Some(labels) = item_labels {
            for item in &mut phase.items {
                if item.label.is_empty() {
                    if let Some(label) = labels.get(item.index as usize) {
                        item.label = label.clone();
                    }
                }
            }
        }
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
        budgets: manifest.budgets.clone(),
    }
}

fn count(phase: &PhaseView, status: ItemStatus) -> usize {
    phase.items.iter().filter(|i| i.status == status).count()
}

/// DungeonGrid party view: **one lane per hero**, not per turn slot.
///
/// The runtime still schedules turns as a serial map (round-robin over heroes);
/// the panel collapses those shards into hero rows so a 4-hero / 8-turn run shows
/// `4 heroes · round-robin` with live status on the active hero.
fn adapt_dungeongrid_party(
    manifest: &RunManifest,
    spec: Option<&WorkflowSpec>,
    item_labels: Option<&[String]>,
    progress: Option<&HashMap<NodePath, ShardProgress>>,
) -> RunView {
    let heroes = party_hero_ids(manifest, item_labels);
    let n = heroes.len().max(1);
    let map_parent = party_map_parent(spec);

    // Per-hero accumulators keyed by hero index.
    let mut statuses = vec![ItemStatus::Queued; n];
    let mut labels = vec![String::new(); n];
    let mut tones = vec![Tone::Neutral; n];
    let mut details = vec![String::new(); n];
    let mut turns_done = vec![0u32; n];
    let mut tokens_in = vec![0u64; n];
    let mut tokens_out = vec![0u64; n];
    let mut steps = vec![0u32; n];
    let mut elapsed = vec![None::<f64>; n];

    // Walk map-item events in order; map turn index → hero via round-robin, then
    // refine with recorded `hero_id` when available.
    for event in &manifest.events {
        let Some((parent, index)) = split_item(&event.addr.node_path) else {
            continue;
        };
        if parent != map_parent
            && !matches!(parent.last(), Some(PathSeg::Node(name)) if name == "play_turns")
        {
            // Accept any map parent whose last segment is play_turns, or matches
            // the preseeded map path.
            if !parent
                .last()
                .map(|s| matches!(s, PathSeg::Node(n) if n == "play_turns" || n == "map"))
                .unwrap_or(false)
            {
                continue;
            }
        }
        if !matches!(
            event.kind,
            EventKind::MapItemStarted | EventKind::MapItemCompleted | EventKind::MapItemFailed
        ) {
            continue;
        }

        let mut hero_idx = (index as usize) % n;
        // Prefer recorded hero_id for this turn path when present.
        let mut turn_path = parent.clone();
        turn_path.push(PathSeg::Index(index));
        if let Some(hid) = recorded_hero_id(manifest, &turn_path) {
            if let Some(i) = heroes.iter().position(|h| h == &hid) {
                hero_idx = i;
            }
        } else if let Some(labels) = item_labels {
            // Live preseed labels are hero ids when dungeongrid — but turn labels
            // may still be t00:hero_1 from older configs.
            if let Some(lab) = labels.get(index as usize) {
                if let Some(i) = heroes
                    .iter()
                    .position(|h| h == lab || lab.ends_with(h.as_str()))
                {
                    hero_idx = i;
                }
            }
        }

        let status = match event.kind {
            EventKind::MapItemCompleted => ItemStatus::Done,
            EventKind::MapItemFailed => ItemStatus::Failed,
            _ => ItemStatus::Running,
        };
        match status {
            ItemStatus::Running => statuses[hero_idx] = ItemStatus::Running,
            ItemStatus::Failed => {
                statuses[hero_idx] = ItemStatus::Failed;
                if let Some(err) = event.payload.get("error").and_then(|v| v.as_str()) {
                    details[hero_idx] = fail_annotation(err);
                    tones[hero_idx] = Tone::Bad;
                }
            }
            ItemStatus::Done => {
                if statuses[hero_idx] != ItemStatus::Running
                    && statuses[hero_idx] != ItemStatus::Failed
                {
                    statuses[hero_idx] = ItemStatus::Done;
                }
                // Count completions (re-walk is fine — we recount below).
            }
            ItemStatus::Queued => {}
        }

        // Fold live progress for this turn shard into the hero lane.
        if let Some(progress) = progress {
            let path = NodePath(turn_path.clone());
            if let Some(shard) = progress.get(&path) {
                // Don't double-count across event re-walks: progress is absolute
                // per shard; we re-sum from scratch each adapt call below.
                let _ = shard;
            }
        }
    }

    // Re-sum progress + recorded labels per hero from all turn shards.
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
        let mut turn_path = parent.clone();
        turn_path.push(PathSeg::Index(index));
        let hero_idx = {
            let mut hi = (index as usize) % n;
            if let Some(hid) = recorded_hero_id(manifest, &turn_path) {
                if let Some(i) = heroes.iter().position(|h| h == &hid) {
                    hi = i;
                }
            }
            hi
        };
        if event.kind == EventKind::MapItemCompleted {
            turns_done[hero_idx] = turns_done[hero_idx].saturating_add(1);
        }
        if let Some((label, tone)) = item_detail(manifest, &turn_path) {
            // Latest completed/recorded turn wins the right-hand label.
            if event.kind == EventKind::MapItemCompleted || !labels[hero_idx].is_empty() {
                labels[hero_idx] = label;
                tones[hero_idx] = tone;
            } else if labels[hero_idx].is_empty() {
                labels[hero_idx] = label;
                tones[hero_idx] = tone;
            }
        }
        if let Some(progress) = progress {
            let path = NodePath(turn_path);
            if let Some(shard) = progress.get(&path) {
                tokens_in[hero_idx] = tokens_in[hero_idx].saturating_add(shard.tokens_in);
                tokens_out[hero_idx] = tokens_out[hero_idx].saturating_add(shard.tokens_out);
                steps[hero_idx] = steps[hero_idx].saturating_add(shard.steps);
                let secs = shard.elapsed_secs();
                elapsed[hero_idx] = Some(elapsed[hero_idx].map_or(secs, |c| c.max(secs)));
                if statuses[hero_idx] == ItemStatus::Running && !shard.last_action.is_empty() {
                    details[hero_idx] = shard.last_action.clone();
                }
            }
        }
    }

    // After a full pass, clear Running if no MapItemStarted without complete for
    // that hero remains — recompute live set.
    let mut live_heroes = vec![false; n];
    let mut started = vec![0u32; n];
    let mut finished = vec![0u32; n];
    let mut failed = vec![0u32; n];
    for event in &manifest.events {
        let Some((_parent, index)) = split_item(&event.addr.node_path) else {
            continue;
        };
        let hero_idx = (index as usize) % n;
        // refine with recorded when possible
        let hero_idx = {
            let mut path = _parent;
            path.push(PathSeg::Index(index));
            recorded_hero_id(manifest, &path)
                .and_then(|hid| heroes.iter().position(|h| h == &hid))
                .unwrap_or(hero_idx)
        };
        match event.kind {
            EventKind::MapItemStarted => started[hero_idx] += 1,
            EventKind::MapItemCompleted => finished[hero_idx] += 1,
            EventKind::MapItemFailed => failed[hero_idx] += 1,
            _ => {}
        }
    }
    for i in 0..n {
        if failed[i] > 0 {
            statuses[i] = ItemStatus::Failed;
        } else if started[i] > finished[i] {
            statuses[i] = ItemStatus::Running;
            live_heroes[i] = true;
        } else if finished[i] > 0 {
            statuses[i] = ItemStatus::Done;
        } else {
            statuses[i] = ItemStatus::Queued;
        }
        turns_done[i] = finished[i];
        if details[i].is_empty() {
            if turns_done[i] > 0 {
                details[i] = format!(
                    "{} turn{}",
                    turns_done[i],
                    if turns_done[i] == 1 { "" } else { "s" }
                );
            }
        } else if statuses[i] == ItemStatus::Running {
            // keep live action; also show turn count
            details[i] = format!(
                "{} · {} turn{}",
                details[i],
                turns_done[i],
                if turns_done[i] == 1 { "" } else { "s" }
            );
        } else if turns_done[i] > 0 && !details[i].contains("turn") {
            details[i] = format!(
                "{} turn{}",
                turns_done[i],
                if turns_done[i] == 1 { "" } else { "s" }
            );
        }
        // Always prefix the hero id so the party lane is identifiable.
        let action = labels[i].clone();
        labels[i] = if action.is_empty() || action == heroes[i] {
            heroes[i].clone()
        } else if action.starts_with(&heroes[i]) {
            action
        } else {
            format!("{:<8} {action}", heroes[i])
        };
    }

    let items: Vec<ItemView> = (0..n)
        .map(|i| ItemView {
            index: i as u32,
            status: statuses[i],
            label: labels[i].clone(),
            tone: tones[i],
            tokens_in: tokens_in[i],
            tokens_out: tokens_out[i],
            steps: steps[i].max(turns_done[i]),
            elapsed_secs: elapsed[i],
            detail: details[i].clone(),
        })
        .collect();

    let phase = PhaseView {
        label: "party".to_string(),
        total: n,
        done: items
            .iter()
            .filter(|i| i.status == ItemStatus::Done)
            .count(),
        failed: items
            .iter()
            .filter(|i| i.status == ItemStatus::Failed)
            .count(),
        live: items
            .iter()
            .filter(|i| i.status == ItemStatus::Running)
            .count(),
        items,
    };

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

    let result = party_result_note(manifest);
    RunView {
        title: manifest.workflow_name.clone(),
        agents: n,
        // Round-robin is serial by design; surface that in the header.
        concurrency: Some(1),
        model: None,
        phases: vec![phase],
        outcome,
        result_note: result,
        budgets: manifest.budgets.clone(),
    }
}

fn party_hero_ids(manifest: &RunManifest, item_labels: Option<&[String]>) -> Vec<String> {
    // 1. Live preseed: CLI passes hero ids for dungeongrid.
    if let Some(labels) = item_labels {
        let heroes: Vec<String> = labels
            .iter()
            .filter(|l| l.starts_with("hero_") || !l.contains(':'))
            .cloned()
            .collect();
        // If labels look like t00:hero_1, extract unique hero suffixes.
        if heroes.len() >= 2 && heroes.iter().all(|h| h.starts_with("hero_")) {
            let mut uniq = Vec::new();
            for h in heroes {
                if !uniq.contains(&h) {
                    uniq.push(h);
                }
            }
            if !uniq.is_empty() {
                return uniq;
            }
        }
        let mut from_turn = Vec::new();
        for lab in labels {
            if let Some(h) = lab.split(':').last() {
                if h.starts_with("hero_") && !from_turn.iter().any(|x: &String| x == h) {
                    from_turn.push(h.to_string());
                }
            }
        }
        if !from_turn.is_empty() {
            return from_turn;
        }
        if !labels.is_empty() && labels.iter().all(|l| !l.contains(':') && !l.is_empty()) {
            let mut uniq = Vec::new();
            for l in labels {
                if !uniq.contains(l) {
                    uniq.push(l.clone());
                }
            }
            return uniq;
        }
    }
    // 2. From recorded hero outputs.
    let mut from_rec = Vec::new();
    for r in &manifest.recorded {
        if let Some(h) = r.outputs.get("hero_id").and_then(|v| v.as_str()) {
            if !from_rec.iter().any(|x: &String| x == h) {
                from_rec.push(h.to_string());
            }
        }
    }
    if !from_rec.is_empty() {
        return from_rec;
    }
    // 3. Default 4-party.
    vec![
        "hero_1".into(),
        "hero_2".into(),
        "hero_3".into(),
        "hero_4".into(),
    ]
}

fn party_map_parent(spec: Option<&WorkflowSpec>) -> Vec<PathSeg> {
    let name = preseed_map_node(spec);
    vec![PathSeg::Node(name)]
}

fn recorded_hero_id(manifest: &RunManifest, item_path: &[PathSeg]) -> Option<String> {
    manifest.recorded.iter().find_map(|r| {
        if starts_with(&r.addr.node_path, item_path) {
            r.outputs
                .get("hero_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        } else {
            None
        }
    })
}

fn party_result_note(manifest: &RunManifest) -> String {
    // Prefer last turn's episode-ish fields; fall back to score/turns from any
    // recorded hero output.
    for r in manifest.recorded.iter().rev() {
        if let (Some(score), Some(turns)) = (
            // episode is program-only; surface aggregate from last scores
            r.score,
            r.outputs.get("turn").and_then(|v| v.as_u64()),
        ) {
            let action = r
                .outputs
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            return format!("last {action} · turn {turns} · reward {score:.2}");
        }
    }
    result_note(manifest)
}

/// Pre-create the full item grid before map events arrive (live follow).
fn preseed_items(
    phases: &mut Vec<(Vec<PathSeg>, PhaseView)>,
    spec: Option<&WorkflowSpec>,
    item_labels: Option<&[String]>,
) {
    let Some(labels) = item_labels else {
        return;
    };
    if phases.is_empty() {
        let phase_label = preseed_map_node(spec);
        phases.push((
            vec![PathSeg::Node(phase_label.clone())],
            PhaseView {
                label: phase_label,
                total: 0,
                done: 0,
                failed: 0,
                live: 0,
                items: Vec::new(),
            },
        ));
    }
    for (_, phase) in phases.iter_mut() {
        for (index, label) in labels.iter().enumerate() {
            let index = index as u32;
            if phase.items.iter().any(|item| item.index == index) {
                continue;
            }
            phase.items.push(ItemView {
                index,
                status: ItemStatus::Queued,
                label: label.clone(),
                tone: Tone::Neutral,
                tokens_in: 0,
                tokens_out: 0,
                steps: 0,
                elapsed_secs: None,
                detail: String::new(),
            });
        }
    }
}

fn preseed_map_node(spec: Option<&WorkflowSpec>) -> String {
    if let Some(node) = spec
        .and_then(|s| s.host.as_ref())
        .and_then(|h| h.viz.as_ref())
        .and_then(|v| v.map_node.as_deref())
    {
        return node.to_string();
    }
    if let Some(spec) = spec {
        for id in &spec.entrypoint {
            if matches!(
                spec.nodes.get(id).map(|n| &n.kind),
                Some(NodeKind::Map { .. })
            ) {
                return id.clone();
            }
        }
    }
    "map".to_string()
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
    // DungeonGrid hero turns: action + reward + position.
    if let Some(action) = outputs.get("action").and_then(|v| v.as_str()) {
        if let Some(prebuilt) = outputs.get("label").and_then(|v| v.as_str()) {
            let tone = dungeon_tone(outputs);
            return Some((prebuilt.to_string(), tone));
        }
        let reward = outputs
            .get("reward")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let mut label = action.to_string();
        if reward != 0.0 {
            label = format!("{label} {reward:+.2}");
        }
        if let Some(pos) = outputs.get("pos").and_then(|v| v.as_array()) {
            if pos.len() == 2 {
                label = format!(
                    "{label}  [{},{}]",
                    pos[0].as_i64().unwrap_or(0),
                    pos[1].as_i64().unwrap_or(0)
                );
            }
        }
        return Some((label, dungeon_tone(outputs)));
    }
    if let Some(score) = outputs.get("score").and_then(|v| v.as_f64()) {
        let mut label = format!("{score:.0}/10");
        if let Some(summary) = violation_summary(outputs) {
            label = format!("{label}  {summary}");
        }
        let tone = if score >= 7.0 {
            Tone::Good
        } else if score < 5.0 {
            Tone::Bad
        } else {
            Tone::Neutral
        };
        return Some((label, tone));
    }
    Some(("ok".to_string(), Tone::Neutral))
}

fn dungeon_tone(outputs: &serde_json::Value) -> Tone {
    if outputs
        .get("done")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Tone::Good;
    }
    let reward = outputs
        .get("reward")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if reward > 0.1 {
        Tone::Good
    } else if reward < 0.0 {
        Tone::Bad
    } else {
        Tone::Neutral
    }
}

/// Compact violation rollup for a scored output: the total count plus a severity
/// histogram, highest-first (`3 viol · 1 crit · 2 low`). Generic over any output
/// carrying a `violations` array of `{severity}` objects (or a `violation_codes`
/// / string list, where only the count is known). Returns `None` when there are
/// no violations, so a clean pass shows just its score — and it is *bounded*
/// (at most one term per severity level), so it never overflows the row the way
/// a raw code dump does.
fn violation_summary(outputs: &serde_json::Value) -> Option<String> {
    // Highest → lowest; `none` is tracked but not shown (a non-violation).
    const LEVELS: [&str; 5] = ["critical", "high", "medium", "low", "none"];
    let raw = outputs
        .get("violations")
        .or_else(|| outputs.get("violation_codes"))?;
    let mut counts = [0u32; LEVELS.len()];
    let count = match raw {
        serde_json::Value::Array(items) => {
            for it in items {
                if let Some(sev) = it.get("severity").and_then(|v| v.as_str()) {
                    // Match case-insensitively: models don't reliably honor a
                    // lowercase enum (real runs emit `HIGH`/`MEDIUM`), and dropping
                    // those would under-count severity exactly when it's highest.
                    let sev = sev.to_ascii_lowercase();
                    if let Some(i) = LEVELS.iter().position(|l| *l == sev) {
                        counts[i] += 1;
                    }
                }
            }
            items.len()
        }
        serde_json::Value::String(s) => s.split(',').filter(|t| !t.trim().is_empty()).count(),
        _ => return None,
    };
    if count == 0 {
        return None;
    }
    let mut out = format!("{count} viol");
    for (i, level) in LEVELS.iter().enumerate() {
        if *level == "none" || counts[i] == 0 {
            continue;
        }
        out.push_str(&format!(" · {} {}", counts[i], severity_abbrev(level)));
    }
    Some(out)
}

/// Short severity tags that fit the row (`critical`→`crit`, `medium`→`med`).
fn severity_abbrev(level: &str) -> &str {
    match level {
        "critical" => "crit",
        "medium" => "med",
        other => other,
    }
}

/// Fold every live [`ShardProgress`] at (or under) an item path into its view —
/// tokens/steps sum, elapsed takes the longest, detail the latest action. Prefix
/// match (not exact) so a nested actor body still attributes to its shard.
fn fold_progress(
    item: &mut ItemView,
    progress: &HashMap<NodePath, ShardProgress>,
    item_path: &[PathSeg],
) {
    for (path, shard) in progress {
        if !starts_with(path, item_path) {
            continue;
        }
        item.tokens_in += shard.tokens_in;
        item.tokens_out += shard.tokens_out;
        item.steps += shard.steps;
        let secs = shard.elapsed_secs();
        item.elapsed_secs = Some(item.elapsed_secs.map_or(secs, |cur| cur.max(secs)));
        // A failed row already holds its failure reason — keep that, don't
        // overwrite it with the last live action.
        if !shard.last_action.is_empty() && item.status != ItemStatus::Failed {
            item.detail = shard.last_action.clone();
        }
    }
}

/// Condense a shard's raw failure string into a compact row annotation. Strips
/// the `actor \`name\`:` prefix, collapses whitespace, and classifies the common
/// cases so the row reads `✗ slug · parse: extra data …` rather than a wall of text.
fn fail_annotation(err: &str) -> String {
    let msg = err.trim();
    // Drop a leading "actor `blog_auditor`: " prefix if present.
    let msg = match (msg.find(": "), msg.starts_with("actor ")) {
        (Some(i), true) => msg[i + 2..].trim(),
        _ => msg,
    };
    let lower = msg.to_lowercase();
    let class = if lower.contains("not a json object") || lower.contains("did not parse") {
        "parse"
    } else if lower.contains("quota") || lower.contains("rate") || lower.contains("429") {
        "rate-limit"
    } else if lower.contains("auth") || lower.contains("unauthorized") {
        "auth"
    } else if lower.contains("no space") || lower.contains("enospc") {
        "disk full"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else {
        "error"
    };
    let one_line = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{class}: {one_line}")
}

/// The result line: prefer a `summary_recorder`-style report, else empty.
fn result_note(manifest: &RunManifest) -> String {
    for r in &manifest.recorded {
        let o = &r.outputs;
        if let Some(report) = o.get("matrix_report").and_then(|v| v.as_str()) {
            return report.to_string();
        }
    }
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

/// How wide the panel renders, whether to emit ANSI color, and live-follow
/// animation state.
#[derive(Debug, Clone, Copy)]
pub struct RenderOpts {
    pub width: usize,
    pub color: bool,
    /// Spinner frame for running rows (incremented by the follow loop).
    pub tick: u32,
    /// Elapsed wall time for the live status line.
    pub elapsed_secs: Option<f64>,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            width: 76,
            color: true,
            tick: 0,
            elapsed_secs: None,
        }
    }
}

/// Render a [`RunView`] as a btop-style panel.
pub fn render_run_view(view: &RunView, opts: &RenderOpts) -> String {
    let width = opts.width.max(40);
    let inner = width - 4; // content between "│ " and " │"

    let mut body: Vec<Line> = Vec::new();
    body.push(Line::default());

    // Identity line: which model is running and how much it has spent so far.
    // Full body width, so the live usage never clips (unlike the border meta).
    if let Some(model) = &view.model {
        let short = model.rsplit('/').next().unwrap_or(model);
        let mut row = Line::default();
        row.push(fit(short, inner), Style::fg(CYAN));
        let (tok_in, tok_out) = token_totals(view);
        let total = tok_in + tok_out;
        if total > 0 {
            let tok = format!("   {} tok", fmt_tokens(total));
            let breakdown = format!(" ({} in · {} out)", fmt_tokens(tok_in), fmt_tokens(tok_out));
            // Throughput we're getting through the model: aggregate tokens/sec and
            // tokens/min over the run so far. `tps` = how fast it's going; `tpm`
            // maps to provider rate-limit budgets. Needs a non-trivial elapsed.
            let thr = opts
                .elapsed_secs
                .filter(|s| *s >= 1.0)
                .map(|secs| {
                    let tps = (total as f64 / secs).round() as u64;
                    format!(" · {} tps · {} tpm", fmt_tokens(tps), fmt_tokens(tps * 60))
                })
                .unwrap_or_default();
            // Fit by preference: full → drop breakdown (keep throughput) → drop
            // throughput → bare total. Throughput outranks the in/out split.
            let candidates = [
                format!("{tok}{breakdown}{thr}"),
                format!("{tok}{thr}"),
                format!("{tok}{breakdown}"),
                tok.clone(),
            ];
            for c in candidates {
                if row.width() + c.chars().count() <= inner {
                    row.push(c, Style::faint());
                    break;
                }
            }
        }
        body.push(row);
        body.push(Line::default());
    }

    // Budget progress + ETA (formal limits / limit engine).
    if let Some(budgets) = &view.budgets {
        for line in budget_lines(budgets, inner) {
            body.push(line);
        }
        if !budgets.items.is_empty() {
            body.push(Line::default());
        }
    }

    if view.outcome == Outcome::Running {
        let total_done: usize = view.phases.iter().map(|phase| phase.done).sum();
        let total_live: usize = view.phases.iter().map(|phase| phase.live).sum();
        let total: usize = view
            .phases
            .iter()
            .map(|phase| phase.total)
            .sum::<usize>()
            .max(view.agents);
        let (spin, spin_style) = status_glyph(ItemStatus::Running, opts.tick);
        let mut row = Line::default();
        row.push(format!("{spin} "), spin_style);
        row.push(
            format!("{}  ", format_elapsed(opts.elapsed_secs)),
            Style::faint(),
        );
        row.push(
            format!("{total_live} live · {total_done}/{total} done"),
            Style::fg(FG),
        );
        // Usage lives on the identity line above; keep this line pure progress.
        if total_live > 0 && row.width() + 12 <= inner {
            row.push("   ", Style::default());
            row.push("scanning…", Style::fg(YELLOW));
        }
        body.push(row);
        body.push(Line::default());
    }

    for phase in &view.phases {
        // Phase row: glyph · label · gradient bar · rollup.
        let ratio = if phase.total > 0 {
            phase.done as f64 / phase.total as f64
        } else {
            0.0
        };
        let mut row = Line::default();
        row.push("◆ ", Style::fg(CYAN));
        row.push(
            format!("{:<10} ", truncate(&phase.label, 10)),
            Style::bold(FG),
        );
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

        // Item rows — windowed. A large map (69 pages) would otherwise spam the
        // whole terminal with mostly-pending rows; instead show at most
        // MAX_ITEM_ROWS that FOLLOW the live frontier, so the window slides
        // forward as shards stream and complete. `⋮ N above/below` marks the
        // hidden items so nothing looks lost.
        let (start, end) = window_bounds(&phase.items);
        if start > 0 {
            body.push(ellipsis_row(start, "done above"));
        }
        for (n, item) in phase.items[start..end].iter().enumerate() {
            // ╰ caps the tree only when the window reaches the true last item.
            let is_last_row = start + n + 1 == end;
            let connector = if is_last_row && end == phase.items.len() {
                "  ╰ "
            } else {
                "  ├ "
            };
            let (glyph, gstyle) = status_glyph(item.status, opts.tick);
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
            let suffix = item_suffix(item);
            if !suffix.is_empty() {
                // Keep the row inside the frame: trim the live suffix to the room
                // left after the label so the right border never shifts.
                let room = inner.saturating_sub(row.width());
                row.push(fit(&suffix, room), Style::faint());
            }
            body.push(row);
        }
        if end < phase.items.len() {
            body.push(ellipsis_row(phase.items.len() - end, "more below"));
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
    // `result_note` may be a multi-line table (the docs/blog matrix). Rendering it
    // as one span leaks embedded newlines past the border AND makes the panel
    // taller than the terminal — which breaks redraw-in-place (the cursor can't
    // move above the viewport, so every frame appends: the "repeating" bug). So
    // render each note line as its OWN bordered, width-clipped row, capped in
    // count; the full table still prints in full below the panel.
    let note_lines: Vec<&str> = view
        .result_note
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if let Some(first) = note_lines.first() {
        result.push("  ·  ", Style::faint());
        result.push(
            fit(first, inner.saturating_sub(result.width() + 2)),
            Style::fg(FG),
        );
    }
    body.push(result);
    const MAX_NOTE_LINES: usize = 12;
    for line in note_lines.iter().skip(1).take(MAX_NOTE_LINES) {
        let mut row = Line::default();
        row.push(
            format!("  {}", fit(line, inner.saturating_sub(2))),
            Style::faint(),
        );
        body.push(row);
    }
    if note_lines.len() > MAX_NOTE_LINES + 1 {
        let hidden = note_lines.len() - MAX_NOTE_LINES - 1;
        let mut row = Line::default();
        row.push(
            format!("  … {hidden} more rows (full table printed below)"),
            Style::faint(),
        );
        body.push(row);
    }

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

/// Render a panel and split it into redraw-friendly lines (no trailing blank).
pub fn render_run_view_lines(view: &RunView, opts: &RenderOpts) -> Vec<String> {
    render_run_view(view, opts)
        .lines()
        .map(str::to_string)
        .collect()
}

/// Overwrite the previous frame on a TTY: move up, clear each line, rewrite.
pub fn redraw_lines<W: std::io::Write>(
    previous_line_count: &mut usize,
    lines: &[String],
    out: &mut W,
) -> std::io::Result<()> {
    if *previous_line_count > 0 {
        write!(out, "\x1b[{}A", previous_line_count)?;
    }
    for line in lines {
        write!(out, "\x1b[2K\r{line}")?;
        writeln!(out)?;
    }
    *previous_line_count = lines.len();
    out.flush()
}

/// Progress + ETA lines (config from `snap.plan.viz` / `snap.plan.eta`).
///
/// ```text
/// budget  calls 4/64 (6%) · tok 20k/500k (4%) · wall 16s/15m (2%)
/// ETA     calls 3m24s · tok 5m42s · wall 14m44s   nearest calls
/// ```
fn budget_lines(snap: &BudgetSnapshot, inner: usize) -> Vec<Line> {
    use jesterky_contract::BudgetEtaMode;

    if snap.items.is_empty() || !snap.plan.viz.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    let viz = &snap.plan.viz;
    let eta_cfg = &snap.plan.eta;

    // ── progress ────────────────────────────────────────────────────────
    if viz.show_progress {
        let mut progress = Line::default();
        let state_style = match snap.state {
            BudgetState::Exhausted => Style::bold(RED),
            BudgetState::Warning => Style::bold(YELLOW),
            BudgetState::Ok => Style::fg(GREEN),
            _ => Style::faint(),
        };
        progress.push("budget  ", state_style);
        let progress_body = snap
            .items
            .iter()
            .filter(|item| item.show_progress)
            .map(|item| {
                let spent = fmt_budget_amount(item.spent, item.kind.unit());
                let max = fmt_budget_amount(item.max, item.kind.unit());
                let name = if viz.short_labels && item.label == item.kind.as_str() {
                    item.kind.short_str().to_string()
                } else {
                    item.label.clone()
                };
                format!("{name} {spent}/{max} ({:.0}%)", item.used_percent)
            })
            .collect::<Vec<_>>()
            .join(" · ");
        progress.push(
            fit(&progress_body, inner.saturating_sub(progress.width())),
            Style::faint(),
        );
        out.push(progress);
    }

    // ── ETA ─────────────────────────────────────────────────────────────
    let eta_on = viz.show_eta && eta_cfg.enabled && !matches!(eta_cfg.mode, BudgetEtaMode::Off);
    if eta_on {
        let mut etas: Vec<(&jesterky_contract::BudgetStatus, f64, bool)> = snap
            .items
            .iter()
            .filter(|item| item.show_eta)
            .filter_map(|item| {
                item.forecast.seconds_to_limit.map(|secs| {
                    let is_nearest = snap
                        .nearest
                        .as_ref()
                        .map(|n| n.kind == item.kind)
                        .unwrap_or(false);
                    (item, secs, is_nearest)
                })
            })
            .collect();
        if matches!(eta_cfg.mode, BudgetEtaMode::NearestOnly) {
            etas.retain(|(_, _, is_near)| *is_near);
        }
        if !etas.is_empty() {
            let mut eta_row = Line::default();
            eta_row.push("ETA     ", Style::bold(YELLOW));
            let mut first = true;
            for (item, secs, is_nearest) in &etas {
                if !first {
                    eta_row.push(" · ", Style::faint());
                }
                first = false;
                let name = if viz.short_labels && item.label == item.kind.as_str() {
                    item.kind.short_str().to_string()
                } else {
                    item.label.clone()
                };
                let piece = format!("{name} {}", format_eta_secs(*secs));
                let style = if *is_nearest {
                    Style::bold(CYAN)
                } else {
                    Style::faint()
                };
                if eta_row.width() + piece.chars().count() > inner {
                    break;
                }
                eta_row.push(piece, style);
            }
            if viz.show_nearest_tag {
                if let Some(near) = &snap.nearest {
                    let nlabel = if viz.short_labels && near.label == near.kind.as_str() {
                        near.kind.short_str().to_string()
                    } else {
                        near.label.clone()
                    };
                    let tag = format!("  nearest {nlabel}");
                    if eta_row.width() + tag.chars().count() <= inner {
                        eta_row.push(tag, Style::fg(YELLOW));
                    }
                }
            }
            out.push(eta_row);
        } else if snap.wall_secs < eta_cfg.min_wall_secs {
            let mut eta_row = Line::default();
            eta_row.push("ETA     ", Style::faint());
            eta_row.push(
                format!("— (need >{:.0}s wall for burn-rate)", eta_cfg.min_wall_secs),
                Style::faint(),
            );
            out.push(eta_row);
        }
    }

    out
}

fn fmt_budget_amount(value: f64, unit: &str) -> String {
    if unit == "seconds" {
        return format_eta_secs(value);
    }
    if value >= 1000.0 {
        fmt_tokens(value.round() as u64)
    } else if (value - value.round()).abs() < 1e-6 {
        format!("{}", value.round() as u64)
    } else {
        format!("{value:.1}")
    }
}

fn format_eta_secs(secs: f64) -> String {
    let secs = secs.max(0.0);
    if secs < 60.0 {
        return format!("{secs:.0}s");
    }
    let m = (secs / 60.0).floor() as u64;
    let s = (secs % 60.0).round() as u64;
    if m < 60 {
        return format!("{m}m{s:02}s");
    }
    let h = m / 60;
    let m = m % 60;
    format!("{h}h{m:02}m")
}

fn header_meta(view: &RunView) -> String {
    // The border carries the run shape only; the model + live usage get their own
    // full-width identity line at the top of the body (never clipped).
    if view.title.starts_with("dungeongrid") {
        return format!("{} heroes · round-robin", view.agents);
    }
    let mut parts = vec![format!("{} agents", view.agents)];
    if let Some(c) = view.concurrency {
        parts.push(format!("{c}-wide"));
    }
    parts.join(" · ")
}

/// Most map item rows to show at once. A large map (e.g. 69 docs pages) would
/// otherwise render a row per item and flood the terminal.
const MAX_ITEM_ROWS: usize = 10;
/// Rows of just-settled context kept above the live frontier for continuity.
const FRONTIER_LEAD: usize = 2;

/// Pick the `[start, end)` window of items to render: the whole list when it
/// fits, otherwise a `MAX_ITEM_ROWS`-tall window anchored on the live frontier
/// (the first running, else first queued, item). As shards complete the frontier
/// advances and the window slides forward — so it tracks the ones streaming
/// updates instead of pinning to the top. Keeps `FRONTIER_LEAD` settled rows
/// above the frontier for context, and never runs past the end.
fn window_bounds(items: &[ItemView]) -> (usize, usize) {
    let len = items.len();
    if len <= MAX_ITEM_ROWS {
        return (0, len);
    }
    let frontier = items
        .iter()
        .position(|i| i.status == ItemStatus::Running)
        .or_else(|| items.iter().position(|i| i.status == ItemStatus::Queued))
        .unwrap_or(len); // all settled → show the tail
    let start = frontier
        .saturating_sub(FRONTIER_LEAD)
        .min(len - MAX_ITEM_ROWS);
    (start, start + MAX_ITEM_ROWS)
}

/// A faint `⋮ N <what>` summary standing in for items hidden by the window.
fn ellipsis_row(n: usize, what: &str) -> Line {
    let mut row = Line::default();
    row.push(format!("  ⋮ {n} {what}"), Style::faint());
    row
}

fn status_glyph(status: ItemStatus, tick: u32) -> (&'static str, Style) {
    const SPIN: [&str; 4] = ["◐", "◓", "◑", "◒"];
    match status {
        ItemStatus::Done => ("✓", Style::fg(GREEN)),
        ItemStatus::Failed => ("✗", Style::fg(RED)),
        ItemStatus::Running => (SPIN[tick as usize % SPIN.len()], Style::fg(YELLOW)),
        ItemStatus::Queued => ("○", Style::faint()),
    }
}

/// Run-level `(input, output)` token totals summed across every shard.
fn token_totals(view: &RunView) -> (u64, u64) {
    let mut tin = 0u64;
    let mut tout = 0u64;
    for phase in &view.phases {
        for item in &phase.items {
            tin += item.tokens_in;
            tout += item.tokens_out;
        }
    }
    (tin, tout)
}

/// The live suffix for an item row: `· elapsed · N steps · Nk tok · action` while
/// running; `· elapsed · Nk tok` once done. Empty when nothing is known yet.
fn item_suffix(item: &ItemView) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(secs) = item.elapsed_secs {
        parts.push(format_elapsed(Some(secs)));
    }
    match item.status {
        ItemStatus::Running => {
            if item.steps > 0 {
                let unit = if item.steps == 1 { "step" } else { "steps" };
                parts.push(format!("{} {unit}", item.steps));
            }
            if item.tokens() > 0 {
                parts.push(format!("{} tok", fmt_tokens(item.tokens())));
            }
            if !item.detail.is_empty() {
                parts.push(item.detail.clone());
            }
        }
        ItemStatus::Done => {
            if item.tokens() > 0 {
                parts.push(format!("{} tok", fmt_tokens(item.tokens())));
            }
        }
        ItemStatus::Failed => {
            if item.tokens() > 0 {
                parts.push(format!("{} tok", fmt_tokens(item.tokens())));
            }
            // The failure reason — why this shard died, not just ✗.
            if !item.detail.is_empty() {
                parts.push(item.detail.clone());
            }
        }
        ItemStatus::Queued => {}
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" · {}", parts.join(" · "))
    }
}

/// Compact token count: `3.2k` / `18k` / `2.1M`, plain `840` under a thousand.
fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        let k = n as f64 / 1_000.0;
        if k < 10.0 {
            format!("{k:.1}k")
        } else {
            format!("{}k", k.round() as u64)
        }
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Trim `text` to at most `room` visible chars (ellipsis when clipped).
fn fit(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        text.to_string()
    } else if room == 0 {
        String::new()
    } else {
        text.chars()
            .take(room.saturating_sub(1))
            .collect::<String>()
            + "…"
    }
}

fn format_elapsed(secs: Option<f64>) -> String {
    let Some(secs) = secs else {
        return "--:--".to_string();
    };
    let total = secs.max(0.0) as u64;
    let minutes = total / 60;
    let seconds = total % 60;
    if minutes > 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("0:{seconds:02}")
    }
}

fn top_border(title: &str, meta: &str, width: usize, color: bool) -> String {
    let mut line = Line::default();
    line.push("╭─┤ ", Style::fg(BORDER));
    line.push(title.to_string(), Style::bold(CYAN));
    line.push(" ├", Style::fg(BORDER));
    // Fixed (non-dash) chars around a meta block: "╭─┤ " (4) + title + " ├" (2) +
    // "┤ " + " ├─" (5) + "╮" (1) = 12 + title. Clamp meta so the border always
    // fits inside `width` with at least one dash — a growing live-usage string
    // never pushes the corner past the frame.
    let fixed = 12 + title.chars().count();
    let meta = if meta.is_empty() {
        String::new()
    } else {
        fit(meta, width.saturating_sub(fixed + 1))
    };
    let right = if meta.is_empty() {
        0
    } else {
        meta.chars().count() + 5
    };
    let used = 4 + title.chars().count() + 2 + right + 1;
    let dashes = width.saturating_sub(used).max(1);
    line.push("─".repeat(dashes), Style::fg(BORDER));
    if !meta.is_empty() {
        line.push("┤ ", Style::fg(BORDER));
        line.push(meta, Style::faint());
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
                    ItemView {
                        index: 0,
                        status: ItemStatus::Done,
                        label: "pass  correctness".to_string(),
                        tone: Tone::Good,
                        tokens_in: 12_000,
                        tokens_out: 3_200,
                        steps: 14,
                        elapsed_secs: Some(31.0),
                        detail: String::new(),
                    },
                    ItemView {
                        index: 1,
                        status: ItemStatus::Done,
                        label: "fail  security".to_string(),
                        tone: Tone::Bad,
                        tokens_in: 8_000,
                        tokens_out: 2_000,
                        steps: 9,
                        elapsed_secs: Some(22.0),
                        detail: String::new(),
                    },
                    ItemView {
                        index: 2,
                        status: ItemStatus::Done,
                        label: "pass  tests".to_string(),
                        tone: Tone::Good,
                        tokens_in: 0,
                        tokens_out: 0,
                        steps: 0,
                        elapsed_secs: None,
                        detail: String::new(),
                    },
                    ItemView {
                        index: 3,
                        status: ItemStatus::Running,
                        label: String::new(),
                        tone: Tone::Neutral,
                        tokens_in: 2_100,
                        tokens_out: 400,
                        steps: 6,
                        elapsed_secs: Some(14.0),
                        detail: "reading runner.rs".to_string(),
                    },
                ],
            }],
            outcome: Outcome::Running,
            result_note: "pass 2 · fail 1".to_string(),
            budgets: None,
        };
        let out = render_run_view(
            &view,
            &RenderOpts {
                width: 76,
                color: false,
                tick: 0,
                elapsed_secs: None,
            },
        );
        assert!(out.contains("╭─┤ quality_scan ├"));
        // Border carries run shape; model + live usage sit on the identity line.
        assert!(out.contains("4 agents · 2-wide"));
        assert!(out.contains("deepseek   28k tok (22k in · 5.6k out)"));
        assert!(out.contains("◆ scan_jobs"));
        // Item 1 *completed* (glyph ✓) but its verdict is `fail` (red label) —
        // execution status and verdict are distinct. Done rows carry usage.
        assert!(out.contains("├ [ 1] ✓ fail  security · 0:22 · 10k tok"));
        // Running row shows live steps / tokens / latest action.
        assert!(out.contains("╰ [ 3] ◐"));
        assert!(out.contains("6 steps · 2.5k tok · reading runner.rs"));
        // Status line stays pure progress; usage is on the identity line above.
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

    #[test]
    fn narrow_panel_never_overflows_its_frame() {
        // A running view with a large token rollup on a cramped width: the status
        // line's optional tail must collapse/drop so every framed line stays the
        // same visible width (no right-border shift).
        let view = RunView {
            title: "quality_scan".to_string(),
            agents: 8,
            concurrency: Some(4),
            model: Some("deepseek/deepseek-v4-pro-direct".to_string()),
            phases: vec![PhaseView {
                label: "scan_jobs".to_string(),
                total: 8,
                done: 2,
                failed: 0,
                live: 4,
                items: vec![ItemView {
                    index: 0,
                    status: ItemStatus::Running,
                    label: String::new(),
                    tone: Tone::Neutral,
                    tokens_in: 120_000,
                    tokens_out: 40_000,
                    steps: 42,
                    elapsed_secs: Some(95.0),
                    detail: "reading a/very/long/path/to/runner.rs".to_string(),
                }],
            }],
            outcome: Outcome::Running,
            result_note: String::new(),
            budgets: None,
        };
        // The fixed 22-column gradient bar sets a ~56-col floor for the phase
        // row; below that the bar itself can't fit. Test at and above it — this
        // is the range where the token rollup used to overflow the status line.
        for width in [56usize, 60, 66, 72] {
            let out = render_run_view(
                &view,
                &RenderOpts {
                    width,
                    color: false,
                    tick: 1,
                    elapsed_secs: Some(95.0),
                },
            );
            let widths: Vec<usize> = out
                .lines()
                .filter(|l| l.starts_with('│'))
                .map(|l| l.chars().count())
                .collect();
            assert!(
                widths.windows(2).all(|w| w[0] == w[1]),
                "ragged panel at width {width}: {widths:?}"
            );
            // The top/bottom borders (incl. the live-usage header meta) must also
            // stay exactly `width` — a growing token string never blows the frame.
            for edge in out
                .lines()
                .filter(|l| l.starts_with('╭') || l.starts_with('╰'))
            {
                assert_eq!(
                    edge.chars().count(),
                    width,
                    "border overflow at {width}: {edge}"
                );
            }
        }
    }

    #[test]
    fn identity_line_shows_model_and_live_usage() {
        // A realistic run shows the (compacted) model id and its running usage on
        // a full-width identity line at the top of the panel body — no clipping.
        let view = RunView {
            title: "quality_scan".to_string(),
            agents: 8,
            concurrency: Some(4),
            model: Some("deepseek/deepseek-v4-pro-direct".to_string()),
            phases: vec![PhaseView {
                label: "scan_jobs".to_string(),
                total: 8,
                done: 8,
                failed: 0,
                live: 0,
                items: (0..8)
                    .map(|i| ItemView {
                        index: i,
                        status: ItemStatus::Done,
                        label: "pass  dim".to_string(),
                        tone: Tone::Good,
                        tokens_in: 3_600,
                        tokens_out: 400,
                        steps: 2,
                        elapsed_secs: Some(30.0),
                        detail: String::new(),
                    })
                    .collect(),
            }],
            outcome: Outcome::Completed,
            result_note: "pass 8 · fail 0".to_string(),
            budgets: None,
        };
        let out = render_run_view(
            &view,
            &RenderOpts {
                width: 76,
                color: false,
                tick: 0,
                elapsed_secs: None,
            },
        );
        // The identity line carries the model and its full usage breakdown.
        let identity = out
            .lines()
            .find(|l| l.contains("deepseek-v4-pro-direct"))
            .expect("identity line present");
        assert!(
            identity.contains("32k tok (29k in · 3.2k out)"),
            "usage missing: {identity}"
        );
        // Full model id shows uncompacted only on the launch command, not here;
        // and every framed line (identity included) holds the frame width.
        for l in out
            .lines()
            .filter(|l| l.starts_with('│') || l.starts_with('╭') || l.starts_with('╰'))
        {
            assert_eq!(l.chars().count(), 76, "frame width broken: {l}");
        }
    }

    #[test]
    fn identity_line_shows_token_throughput_when_elapsed_known() {
        // 32k total tokens over 20s → 1.6k tps · 96k tpm on the identity line.
        let view = RunView {
            title: "quality_scan".to_string(),
            agents: 8,
            concurrency: Some(4),
            model: Some("deepseek/deepseek-v4-pro-direct".to_string()),
            phases: vec![PhaseView {
                label: "scan_jobs".to_string(),
                total: 8,
                done: 8,
                failed: 0,
                live: 0,
                items: (0..8)
                    .map(|i| ItemView {
                        index: i,
                        status: ItemStatus::Done,
                        label: "pass".to_string(),
                        tone: Tone::Good,
                        tokens_in: 3_600,
                        tokens_out: 400,
                        steps: 2,
                        elapsed_secs: Some(20.0),
                        detail: String::new(),
                    })
                    .collect(),
            }],
            outcome: Outcome::Completed,
            result_note: String::new(),
            budgets: None,
        };
        // Wide panel so both breakdown and throughput fit.
        let out = render_run_view(
            &view,
            &RenderOpts {
                width: 100,
                color: false,
                tick: 0,
                elapsed_secs: Some(20.0),
            },
        );
        let identity = out
            .lines()
            .find(|l| l.contains("deepseek-v4-pro-direct"))
            .expect("identity line present");
        assert!(identity.contains("1.6k tps"), "tps missing: {identity}");
        assert!(identity.contains("96k tpm"), "tpm missing: {identity}");
        // Throughput outranks the breakdown: at a width where the full line won't
        // fit but `tok + throughput` does, the (in · out) split is dropped and
        // tps/tpm survive.
        let narrow = render_run_view(
            &view,
            &RenderOpts {
                width: 64,
                color: false,
                tick: 0,
                elapsed_secs: Some(20.0),
            },
        );
        let narrow_id = narrow
            .lines()
            .find(|l| l.contains("deepseek-v4-pro-direct"))
            .expect("identity line present");
        assert!(
            narrow_id.contains("tps"),
            "throughput dropped under pressure: {narrow_id}"
        );
        assert!(
            !narrow_id.contains("in ·"),
            "breakdown should drop first: {narrow_id}"
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

    #[test]
    fn redraw_lines_tracks_previous_frame_height() {
        let mut buf = Vec::new();
        let mut prev = 0usize;
        redraw_lines(&mut prev, &["one".to_string(), "two".to_string()], &mut buf)
            .expect("first redraw");
        assert_eq!(prev, 2);
        redraw_lines(&mut prev, &["solo".to_string()], &mut buf).expect("second redraw");
        assert_eq!(prev, 1);
        let rendered = String::from_utf8(buf).expect("utf8");
        assert!(rendered.contains("\x1b[2A"));
        assert!(rendered.contains("\x1b[2K\rsolo"));
    }

    #[test]
    fn fail_annotation_classifies_and_strips_actor_prefix() {
        assert_eq!(
            fail_annotation("actor `blog_auditor`: model reply was not a JSON object: extra data"),
            "parse: model reply was not a JSON object: extra data"
        );
        assert!(fail_annotation("quota: 429 too many requests").starts_with("rate-limit:"));
        assert!(fail_annotation("No space left on device").starts_with("disk full:"));
    }

    #[test]
    fn failed_item_row_shows_its_reason() {
        let view = RunView {
            title: "quality_scan_blogs".to_string(),
            agents: 1,
            concurrency: Some(8),
            model: None,
            phases: vec![PhaseView {
                label: "audit_posts".to_string(),
                total: 1,
                done: 0,
                failed: 1,
                live: 0,
                items: vec![ItemView {
                    index: 0,
                    status: ItemStatus::Failed,
                    label: "managed-research".to_string(),
                    tone: Tone::Bad,
                    tokens_in: 14_000,
                    tokens_out: 1_600,
                    steps: 3,
                    elapsed_secs: Some(20.0),
                    detail: "parse: a brace span did not parse: Extra data".to_string(),
                }],
            }],
            outcome: Outcome::Failed,
            result_note: String::new(),
            budgets: None,
        };
        let out = render_run_view(
            &view,
            &RenderOpts {
                width: 96,
                color: false,
                tick: 0,
                elapsed_secs: None,
            },
        );
        assert!(out.contains("✗ managed-research"));
        assert!(out.contains("parse:"), "failure reason missing:\n{out}");
    }

    #[test]
    fn window_bounds_follows_the_live_frontier() {
        // 69 items: 0..28 done, 28..36 running, rest queued. The frontier is the
        // first running (28); the window keeps FRONTIER_LEAD above it and is
        // exactly MAX_ITEM_ROWS tall.
        let mut items = Vec::new();
        for i in 0..69u32 {
            let status = if i < 28 {
                ItemStatus::Done
            } else if i < 36 {
                ItemStatus::Running
            } else {
                ItemStatus::Queued
            };
            items.push(ItemView {
                index: i,
                status,
                label: format!("page-{i}"),
                tone: Tone::Neutral,
                tokens_in: 0,
                tokens_out: 0,
                steps: 0,
                elapsed_secs: None,
                detail: String::new(),
            });
        }
        let (start, end) = window_bounds(&items);
        assert_eq!(
            (start, end),
            (26, 36),
            "window follows frontier 28 with lead 2"
        );
        assert_eq!(end - start, MAX_ITEM_ROWS);

        // A short list is shown whole (no window).
        assert_eq!(window_bounds(&items[..8]), (0, 8));

        // All settled → window pins to the tail, never past the end.
        let mut done = items.clone();
        for it in &mut done {
            it.status = ItemStatus::Done;
        }
        assert_eq!(window_bounds(&done), (59, 69));
    }

    #[test]
    fn large_map_panel_windows_to_ten_rows_with_ellipses() {
        let mut items = Vec::new();
        for i in 0..69u32 {
            let status = if i < 28 {
                ItemStatus::Done
            } else if i < 36 {
                ItemStatus::Running
            } else {
                ItemStatus::Queued
            };
            items.push(ItemView {
                index: i,
                status,
                label: format!("page-{i}"),
                tone: Tone::Neutral,
                tokens_in: 0,
                tokens_out: 0,
                steps: 0,
                elapsed_secs: None,
                detail: String::new(),
            });
        }
        let view = RunView {
            title: "quality_scan_docs".to_string(),
            agents: 69,
            concurrency: Some(8),
            model: None,
            phases: vec![PhaseView {
                label: "audit_pages".to_string(),
                total: 69,
                done: 28,
                failed: 0,
                live: 8,
                items,
            }],
            outcome: Outcome::Running,
            result_note: String::new(),
            budgets: None,
        };
        let out = render_run_view(
            &view,
            &RenderOpts {
                width: 96,
                color: false,
                tick: 0,
                elapsed_secs: None,
            },
        );
        let item_rows = out.lines().filter(|l| l.contains("] ")).count();
        assert_eq!(item_rows, MAX_ITEM_ROWS, "exactly ten item rows:\n{out}");
        assert!(
            out.contains("⋮ 26 done above"),
            "top summary missing:\n{out}"
        );
        assert!(
            out.contains("⋮ 33 more below"),
            "bottom summary missing:\n{out}"
        );
        // The live frontier is visible; the far ends are not.
        assert!(out.contains("[28] "), "frontier row shown:\n{out}");
        assert!(!out.contains("[ 0] "), "top item hidden:\n{out}");
        assert!(!out.contains("[68] "), "tail item hidden:\n{out}");
    }

    #[test]
    fn large_multiline_result_note_stays_framed_and_bounded() {
        // A 40-row matrix in result_note must NOT leak past the border or make the
        // panel taller than a terminal (the redraw "repeating" bug). Every content
        // line stays framed, the note is capped, and a pointer names the overflow.
        let matrix = (0..40)
            .map(|i| {
                format!(
                    "row-{i:02}  score {}  a very wide column of text here",
                    i % 10
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let view = RunView {
            title: "quality_scan_docs".to_string(),
            agents: 69,
            concurrency: Some(20),
            model: None,
            phases: vec![PhaseView {
                label: "audit_pages".to_string(),
                total: 1,
                done: 1,
                failed: 0,
                live: 0,
                items: vec![ItemView {
                    index: 0,
                    status: ItemStatus::Done,
                    label: "page".to_string(),
                    tone: Tone::Good,
                    tokens_in: 0,
                    tokens_out: 0,
                    steps: 0,
                    elapsed_secs: None,
                    detail: String::new(),
                }],
            }],
            outcome: Outcome::Completed,
            result_note: matrix,
            budgets: None,
        };
        let out = render_run_view(
            &view,
            &RenderOpts {
                width: 60,
                color: false,
                tick: 0,
                elapsed_secs: None,
            },
        );
        let lines: Vec<&str> = out.lines().collect();
        // No content line escapes the frame: every line is a border or is framed
        // by `│ … │`, and none exceeds the panel width.
        for l in &lines {
            let is_border = l.starts_with('╭') || l.starts_with('╰');
            assert!(
                is_border || (l.starts_with('│') && l.ends_with('│')),
                "line escaped the frame: {l:?}"
            );
            assert!(l.chars().count() <= 60, "line wider than panel: {l:?}");
        }
        // Bounded height: the 40-row matrix is capped, not fully inlined.
        assert!(lines.len() < 30, "panel too tall ({} lines)", lines.len());
        assert!(
            out.contains("more rows (full table printed below)"),
            "overflow pointer missing:\n{out}"
        );
    }

    #[test]
    fn violation_summary_counts_and_bins_by_severity() {
        // Array of {severity} objects → count + a highest-first histogram,
        // bounded (one term per level) so it never overflows the row.
        let outputs = serde_json::json!({
            "score": 5,
            "violations": [
                { "code": "V1", "severity": "high" },
                { "code": "V2", "severity": "low" },
                { "code": "V3", "severity": "low" },
                { "code": "V4", "severity": "none" },
            ],
        });
        assert_eq!(
            violation_summary(&outputs).as_deref(),
            Some("4 viol · 1 high · 2 low")
        );
        // Models don't honor the lowercase enum — `HIGH`/`MEDIUM` must still bin,
        // or the histogram silently drops the highest severities (real-run bug).
        let uppercased = serde_json::json!({
            "score": 5,
            "violations": [
                { "code": "V3", "severity": "HIGH" },
                { "code": "M", "severity": "MEDIUM" },
            ],
        });
        assert_eq!(
            violation_summary(&uppercased).as_deref(),
            Some("2 viol · 1 high · 1 med")
        );
        // A clean pass (no violations) shows just its score — no summary.
        assert_eq!(
            violation_summary(&serde_json::json!({ "score": 9, "violations": [] })),
            None
        );
        // Legacy string form: count only (no per-item severity to bin).
        assert_eq!(
            violation_summary(&serde_json::json!({ "violation_codes": "A,B,C" })).as_deref(),
            Some("3 viol")
        );
    }
}
