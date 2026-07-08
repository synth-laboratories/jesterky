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

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{json, Value};
use std::sync::Arc;

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

/// The pure programs the scan topology needs: `quality.expand` and
/// `quality.aggregate`. Register these on any [`Runner`](jesterky_core::Runner)
/// that runs the scan.
pub fn programs() -> ProgramRegistry {
    let mut programs = ProgramRegistry::new();
    programs.register("quality.expand", Arc::new(expand));
    programs.register("quality.aggregate", Arc::new(aggregate));
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
You are a rigorous code-quality auditor. You are given one audit `job` with a \
`dimension` (e.g. security, tests, correctness) and a `target`. Judge the target \
ONLY along that one dimension. Reply with a JSON object with exactly these \
fields: `dimension` (echo it), `verdict` (\"pass\" or \"fail\"), `severity` \
(\"none\" | \"low\" | \"medium\" | \"high\"), and `rationale` (one sentence). \
Be decisive.";

/// System prompt for the report actor.
pub const SUMMARY_SYSTEM_PROMPT: &str = "\
You are given a `summary` object aggregating per-dimension audit verdicts. Reply \
with a JSON object echoing the summary's `verdict`, `passed`, and `failed`, plus \
a `headline` field: one sentence stating the overall quality outcome.";
