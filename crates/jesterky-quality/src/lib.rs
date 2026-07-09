//! `jesterky-quality` — the reference **M2 workload**: a structured code quality
//! scan, expressed as a jesterky topology that runs on a real host actor.
//!
//! Shape (the committed `examples/quality_scan.json` topology):
//! `expand` fans a target into one audit **job per dimension** → a `map` runs a
//! `quality_scanner` **actor** per job (a model verdict) → a `reduce` aggregates
//! the verdicts into a report → a `summary_recorder` actor emits it.
//!
//! This crate owns the two pure programs and the actor **roles** (system
//! prompts). It is a *workload on* the substrate, not part of it — mloky's V2
//! quality scan ported to jesterky. Swap `FakeActor` for a
//! `jesterky_model::ModelActor` + [`roles`] and the same topology runs on a real
//! model and replays byte-for-byte.
//!
//! The dimensions are the "8 verdicts" of the mloky scan.

pub mod blog;
pub mod docs;
pub mod dungeongrid;
pub mod host;
pub mod obliq;
pub mod trace;

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{json, Value};
use std::sync::Arc;

pub use dungeongrid::{DungeonGridActor, DungeonGridState};

/// The audit dimensions — one actor verdict is produced per dimension.
pub const DIMENSIONS: [&str; 8] = [
    "correctness",
    "security",
    "performance",
    "error_handling",
    "tests",
    "docs",
    "style",
    "api_design",
];

/// The topology's scanner actor name (produces a per-dimension verdict).
pub const SCANNER_ACTOR: &str = "quality_scanner";
/// The topology's report actor name (emits the aggregated report).
pub const SUMMARY_ACTOR: &str = "summary_recorder";

pub use host::host_config;

/// The pure programs the scan topology needs: `quality.expand` and
/// `quality.aggregate`. Register these on any [`Runner`](jesterky_core::Runner)
/// that runs the scan.
///
/// DungeonGrid programs share a process-local env handle so the CLI can also
/// mount a matching [`DungeonGridActor`]. Prefer [`programs_with_dungeon`] when
/// you need to pair them yourself.
pub fn programs() -> ProgramRegistry {
    programs_with_dungeon(DungeonGridState::new())
}

/// Like [`programs`], but DungeonGrid reset/finalize close over `state` so a
/// host can hand the same handle to [`DungeonGridActor`].
pub fn programs_with_dungeon(state: DungeonGridState) -> ProgramRegistry {
    let mut programs = ProgramRegistry::new();
    programs.register("quality.expand", Arc::new(expand));
    programs.register("quality.aggregate", Arc::new(aggregate));
    blog::register(&mut programs);
    docs::register(&mut programs);
    trace::register(&mut programs);
    obliq::register(&mut programs);
    dungeongrid::register(&mut programs, state);
    programs
}

/// `quality.expand` — fan the target into one job per [`DIMENSIONS`] entry.
/// Resolves the audit target from (in order) its resolved `inputs.target`, then
/// the seeded `ledger.target` (from run args — `--args '{"target":"…"}'`), then a
/// generic default, so the program is total whether or not a target was given.
fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let target = inputs
        .get("target")
        .and_then(Value::as_str)
        .or_else(|| ledger.get("target").and_then(Value::as_str))
        .unwrap_or("the target codebase")
        .to_string();
    let jobs: Vec<Value> = DIMENSIONS
        .iter()
        .map(|dimension| json!({ "dimension": dimension, "target": target }))
        .collect();
    Ok(json!({ "jobs": jobs }))
}

/// `quality.aggregate` — fold the per-dimension verdicts into a report. Robust to
/// a missing `verdict` field (a fake/echo actor produces none): anything not
/// explicitly `"fail"` counts as a pass, so the program never errors on shape.
fn aggregate(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let scans = inputs
        .get("scans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failing_dimensions = Vec::new();
    for scan in &scans {
        let is_fail = scan
            .get("verdict")
            .and_then(Value::as_str)
            .is_some_and(|verdict| verdict.eq_ignore_ascii_case("fail"));
        if is_fail {
            failed += 1;
            if let Some(dimension) = scan.get("dimension").and_then(Value::as_str) {
                failing_dimensions.push(dimension.to_string());
            }
        } else {
            passed += 1;
        }
    }
    let verdict = if failed == 0 { "pass" } else { "fail" };
    Ok(json!({
        "summary": {
            "total": scans.len(),
            "passed": passed,
            "failed": failed,
            "failing_dimensions": failing_dimensions,
            "verdict": verdict,
        }
    }))
}

/// System prompts for the scan's actors, as `(actor_name, system_prompt)`. Apply
/// them to a `jesterky_model::ModelActor` with `with_role` so a real model plays
/// each role. (Roles are host-side, like programs — the locked `NodeKind::Actor`
/// carries only a name, not a prompt.)
pub fn roles() -> [(&'static str, &'static str); 2] {
    [
        (SCANNER_ACTOR, SCANNER_SYSTEM_PROMPT),
        (SUMMARY_ACTOR, SUMMARY_SYSTEM_PROMPT),
    ]
}

/// System prompt for the per-dimension auditor.
pub const SCANNER_SYSTEM_PROMPT: &str = "\
You are a fast code-quality auditor. You receive one audit `job` (`dimension`, \
`target`). Judge ONLY that dimension. STRICT: do NOT run shell commands, do NOT \
read files, do NOT explore the repo — one bounded judgment from the dimension \
name and target label. Your entire reply is ONE JSON object with exactly: \
`dimension` (echo it), `verdict` (\"pass\" or \"fail\"), `severity` \
(\"none\"|\"low\"|\"medium\"|\"high\"), `rationale` (one sentence). Stop \
immediately after the JSON.";

/// System prompt for the report actor.
pub const SUMMARY_SYSTEM_PROMPT: &str = "\
You receive a `summary` object with aggregated audit verdicts. Do NOT run tools. \
Reply with ONE JSON object: echo `verdict`, `passed`, `failed` from the summary, \
plus `headline` (one sentence on overall quality). Nothing else.";
