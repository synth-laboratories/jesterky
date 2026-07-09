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
pub mod oss_code;
pub mod trace;

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
const DEFAULT_QUALITY_TARGET: &str = "the target codebase";

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
    oss_code::register(&mut programs);
    trace::register(&mut programs);
    obliq::register(&mut programs);
    dungeongrid::register(&mut programs, state);
    programs
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ScanVerdict {
    Pass,
    Fail,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ScanSeverity {
    None,
    Low,
    Medium,
    High,
}

#[derive(Debug, Serialize)]
struct QualityJob {
    dimension: &'static str,
    target: String,
}

#[derive(Debug, Serialize)]
struct QualityExpandOutput {
    jobs: Vec<QualityJob>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualityExpandInput {
    target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScanRecord {
    dimension: String,
    verdict: Option<ScanVerdict>,
    severity: Option<ScanSeverity>,
    rationale: Option<String>,
    target: Option<String>,
}

#[derive(Debug)]
enum ScanRecord {
    Verdict {
        dimension: String,
        verdict: ScanVerdict,
    },
    FakeEcho,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QualityAggregateInput {
    scans: Vec<RawScanRecord>,
}

#[derive(Debug, Serialize)]
struct QualityAggregateOutput {
    summary: QualityAggregateSummary,
}

#[derive(Debug, Serialize)]
struct QualityAggregateSummary {
    total: usize,
    passed: usize,
    failed: usize,
    failing_dimensions: Vec<String>,
    verdict: ScanVerdict,
}

/// `quality.expand` — fan the target into one job per [`DIMENSIONS`] entry.
/// Resolves the audit target from its resolved `inputs.target` or the seeded
/// `ledger.target` (from run args — `--args '{"target":"…"}'`), or the explicit
/// demo label used by the deterministic quickstart when no repository is named.
fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let QualityExpandInput { target } = serde_json::from_value(inputs.clone())
        .map_err(|err| CoreError::Config(format!("invalid quality.expand input: {err}")))?;
    let target = match target {
        Some(target) if !target.trim().is_empty() => target,
        Some(_) => {
            return Err(CoreError::Config(
                "quality.expand `target` must be non-empty".to_string(),
            ));
        }
        None => ledger
            .get("target")
            .and_then(Value::as_str)
            .filter(|target| !target.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| DEFAULT_QUALITY_TARGET.to_string()),
    };
    let jobs = DIMENSIONS
        .iter()
        .map(|dimension| QualityJob {
            dimension: *dimension,
            target: target.clone(),
        })
        .collect();
    quality_value(QualityExpandOutput { jobs })
}

/// `quality.aggregate` — fold the per-dimension verdicts into a report. Every
/// scan must carry an explicit `dimension` and `verdict`; malformed verdicts are
/// config errors, never implicit passes.
fn aggregate(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let QualityAggregateInput { scans } = serde_json::from_value(inputs.clone())
        .map_err(|err| CoreError::Config(format!("invalid quality.aggregate input: {err}")))?;
    let records = scans
        .into_iter()
        .map(ScanRecord::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failing_dimensions = Vec::new();
    for record in records {
        match record {
            ScanRecord::Verdict {
                verdict: ScanVerdict::Pass,
                ..
            }
            | ScanRecord::FakeEcho => passed += 1,
            ScanRecord::Verdict {
                dimension,
                verdict: ScanVerdict::Fail,
            } => {
                failed += 1;
                failing_dimensions.push(dimension);
            }
        }
    }
    let verdict = if failed == 0 {
        ScanVerdict::Pass
    } else {
        ScanVerdict::Fail
    };
    quality_value(QualityAggregateOutput {
        summary: QualityAggregateSummary {
            total: passed + failed,
            passed,
            failed,
            failing_dimensions,
            verdict,
        },
    })
}

impl TryFrom<RawScanRecord> for ScanRecord {
    type Error = CoreError;

    fn try_from(raw: RawScanRecord) -> Result<Self, Self::Error> {
        match (raw.verdict, raw.severity, raw.rationale, raw.target) {
            (Some(verdict), Some(_severity), Some(rationale), None)
                if !raw.dimension.trim().is_empty() && !rationale.trim().is_empty() =>
            {
                Ok(Self::Verdict {
                    dimension: raw.dimension,
                    verdict,
                })
            }
            (None, None, None, Some(target))
                if !raw.dimension.trim().is_empty() && !target.trim().is_empty() =>
            {
                Ok(Self::FakeEcho)
            }
            _ => Err(CoreError::Config(format!(
                "quality aggregate record for dimension {:?} must be a complete verdict or a typed fake-actor echo",
                raw.dimension
            ))),
        }
    }
}

fn quality_value(output: impl Serialize) -> Result<Value, CoreError> {
    serde_json::to_value(output)
        .map_err(|err| CoreError::Config(format!("quality program output is not JSON: {err}")))
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
