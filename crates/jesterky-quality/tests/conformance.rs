//! mloky parity gate.
//!
//! mloky is the reference runtime jesterky descends from. The two do NOT share an
//! event vocabulary — mloky emits domain lifecycle events (`run_started`,
//! `agent_started`, `agent_completed`, `run_completed`) while jesterky emits the
//! pinned contract stream keyed by `Addr` (ADR #5). Byte-for-byte event equality
//! is therefore the wrong thing to assert.
//!
//! What DOES transfer — and what any workflow substrate must guarantee — are two
//! observable properties of a map-over-jobs-then-reduce run:
//!
//!   * **Conservation.** Every job that starts completes, and the count is
//!     conserved end to end: `jobs_started == jobs_completed == jobs_in_report`.
//!     No work is silently dropped between fan-out and fan-in.
//!   * **Termination.** The run reaches a terminal `completed` status with every
//!     job ok.
//!
//! This gate projects BOTH runtimes onto that canonical `RunOutcome` and asserts
//! each satisfies conservation + termination. The mloky projection is read from a
//! real recorded run (`fixtures/mloky_scan_reference.jsonl`); the jesterky
//! projection is computed from a fresh deterministic scan. Passing means jesterky
//! reproduces the reference runtime's contract on the scan topology.

use jesterky_actor::{MemArtifactStore, MemCheckpointStore, MemEventSink, ReplayClock};
use jesterky_contract::{CallKind, EventKind, RunManifest, RunStatus, WorkflowSpec};
use jesterky_core::{Actor, Runner};
use jesterky_model::{ModelActor, ModelError, ModelRequest, StubModel};
use jesterky_quality::{programs, roles, DIMENSIONS, SCANNER_ACTOR, SUMMARY_ACTOR};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// The runtime-agnostic projection both systems are compared on.
#[derive(Debug, PartialEq, Eq)]
struct RunOutcome {
    jobs_started: usize,
    jobs_completed: usize,
    jobs_in_report: usize,
    terminal_completed: bool,
    all_jobs_ok: bool,
}

impl RunOutcome {
    /// Conservation + termination — the two properties that transfer across
    /// runtimes regardless of event vocabulary.
    fn is_faithful(&self) -> bool {
        self.jobs_started > 0
            && self.jobs_started == self.jobs_completed
            && self.jobs_completed == self.jobs_in_report
            && self.terminal_completed
            && self.all_jobs_ok
    }
}

/// Project a recorded mloky run (its `.jsonl` event log) onto `RunOutcome`.
fn mloky_outcome(log: &str) -> RunOutcome {
    let rows: Vec<Value> = log
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("mloky event is json"))
        .collect();
    let kind = |r: &Value| r["kind"].as_str().unwrap_or("").to_string();

    let jobs_declared = rows
        .iter()
        .find(|r| kind(r) == "run_started")
        .and_then(|r| r["jobs"].as_array())
        .map(|a| a.len())
        .expect("run_started declares jobs");
    let jobs_started = rows.iter().filter(|r| kind(r) == "agent_started").count();
    let completed: Vec<&Value> = rows.iter().filter(|r| kind(r) == "agent_completed").collect();
    let all_jobs_ok = !completed.is_empty()
        && completed.iter().all(|r| r["ok"].as_bool() == Some(true));
    let terminal_completed = rows
        .iter()
        .find(|r| kind(r) == "run_completed")
        .map(|r| r["status"].as_str() == Some("completed") && r["ok"].as_bool() == Some(true))
        .unwrap_or(false);

    RunOutcome {
        // In mloky the reduce consumes exactly the completed agents; the report
        // is over the same set, so `jobs_in_report` == completed count.
        jobs_started: jobs_declared.max(jobs_started),
        jobs_completed: completed.len(),
        jobs_in_report: completed.len(),
        terminal_completed,
        all_jobs_ok,
    }
}

/// Project a jesterky `RunManifest` onto the same `RunOutcome`.
fn jesterky_outcome(manifest: &RunManifest) -> RunOutcome {
    let count = |k: EventKind| manifest.events.iter().filter(|e| e.kind == k).count();
    let jobs_started = count(EventKind::MapItemStarted);
    let jobs_completed = count(EventKind::MapItemCompleted);

    // The report the reduce fed to the summary actor: passed + failed.
    let summary = manifest
        .recorded
        .iter()
        .find(|r| matches!(&r.call, CallKind::Actor { actor } if actor == SUMMARY_ACTOR))
        .expect("summary recorded");
    let jobs_in_report = summary.outputs["passed"].as_u64().unwrap_or(0) as usize
        + summary.outputs["failed"].as_u64().unwrap_or(0) as usize;

    // Every scanner verdict is a well-formed pass/fail (a job that "ok"-completed
    // its judgment), and no map item failed.
    let verdicts_ok = manifest
        .recorded
        .iter()
        .filter(|r| matches!(&r.call, CallKind::Actor { actor } if actor == SCANNER_ACTOR))
        .all(|r| matches!(r.outputs["verdict"].as_str(), Some("pass") | Some("fail")));
    let all_jobs_ok = verdicts_ok && count(EventKind::MapItemFailed) == 0;

    RunOutcome {
        jobs_started,
        jobs_completed,
        jobs_in_report,
        terminal_completed: manifest.status == RunStatus::Completed,
        all_jobs_ok,
    }
}

fn scan_spec() -> WorkflowSpec {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/quality_scan.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("read scan spec")).expect("parse")
}

/// Run the quality scan deterministically (stub model, no network).
async fn run_jesterky_scan() -> RunManifest {
    let model = StubModel::new(|req: &ModelRequest| -> Result<String, ModelError> {
        match req.actor.as_str() {
            SCANNER_ACTOR => {
                let dim = req
                    .inputs
                    .pointer("/job/dimension")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let verdict = if matches!(dim, "security" | "tests") { "fail" } else { "pass" };
                Ok(json!({
                    "dimension": dim,
                    "verdict": verdict,
                    "severity": if verdict == "fail" { "high" } else { "none" },
                    "rationale": "stub",
                })
                .to_string())
            }
            SUMMARY_ACTOR => Ok(req.inputs.get("summary").cloned().unwrap_or(json!({})).to_string()),
            other => Ok(json!({ "actor": other }).to_string()),
        }
    });
    let mut actor = ModelActor::new(model);
    for (name, prompt) in roles() {
        actor = actor.with_role(name, prompt);
    }
    let runner = Runner {
        programs: programs(),
        actor: Arc::new(actor) as Arc<dyn Actor>,
        resource: None,
        sink: Arc::new(MemEventSink::new()),
        clock: Arc::new(ReplayClock::default()),
        store: Arc::new(MemArtifactStore::new()),
        checkpoints: Some(Arc::new(MemCheckpointStore::new())),
    };
    runner
        .run(&scan_spec(), "conformance-scan".to_string(), json!({}))
        .await
        .expect("scan runs")
}

/// The oracle itself must satisfy the properties we compare against — otherwise
/// the fixture is meaningless.
#[test]
fn mloky_reference_run_is_faithful() {
    let log = include_str!("../fixtures/mloky_scan_reference.jsonl");
    let outcome = mloky_outcome(log);
    assert!(
        outcome.is_faithful(),
        "mloky reference log violates conservation/termination: {outcome:?}"
    );
    assert_eq!(outcome.jobs_completed, 8, "reference scan fanned out to 8 jobs");
}

/// The gate: jesterky reproduces the reference runtime's contract on the scan
/// topology — conservation + termination hold, just as they do for mloky.
#[tokio::test]
async fn jesterky_scan_matches_mloky_contract() {
    let mloky = mloky_outcome(include_str!("../fixtures/mloky_scan_reference.jsonl"));
    let jesterky = jesterky_outcome(&run_jesterky_scan().await);

    assert!(mloky.is_faithful(), "oracle unfaithful: {mloky:?}");
    assert!(
        jesterky.is_faithful(),
        "jesterky violates the reference contract: {jesterky:?}"
    );

    // Conservation is exact within jesterky: started == completed == report.
    assert_eq!(jesterky.jobs_started, DIMENSIONS.len());
    assert_eq!(jesterky.jobs_started, jesterky.jobs_completed);
    assert_eq!(jesterky.jobs_completed, jesterky.jobs_in_report);
}
