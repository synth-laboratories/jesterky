//! The quality scan run end-to-end on the committed `examples/quality_scan.json`
//! topology, driven by a stub model (deterministic, no network). Plus unit tests
//! of the two pure programs, and an `#[ignore]`d live codex scan.

use jesterky_actor::{MemArtifactStore, MemCheckpointStore, MemEventSink, ReplayClock};
use jesterky_contract::{CallKind, ProcessNode, RunStopReason, WorkflowSpec};
use jesterky_core::ledger::Ledger;
use jesterky_core::{Actor, Runner};
use jesterky_model::{CodexModel, ModelActor, ModelError, ModelRequest, StubModel};
use jesterky_quality::{programs, roles, DIMENSIONS, SCANNER_ACTOR, SUMMARY_ACTOR};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn scan_spec() -> WorkflowSpec {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("quality_scan.json");
    let text = std::fs::read_to_string(&path).expect("read quality_scan.json");
    serde_json::from_str(&text).expect("parse quality_scan.json")
}

fn runner(actor: Arc<dyn Actor>) -> Runner {
    Runner {
        programs: programs(),
        actor,
        resource: None,
        sink: Arc::new(MemEventSink::new()),
        clock: Arc::new(ReplayClock::default()),
        store: Arc::new(MemArtifactStore::new()),
        checkpoints: Some(Arc::new(MemCheckpointStore::new())),
    }
}

/// Apply the scan's roles to a ModelActor.
fn with_roles<M: jesterky_model::Model>(mut actor: ModelActor<M>) -> ModelActor<M> {
    for (name, prompt) in roles() {
        actor = actor.with_role(name, prompt);
    }
    actor
}

#[tokio::test]
async fn scan_aggregates_stubbed_verdicts_into_a_failing_report() {
    // A stub model: FAIL on `security` and `tests`, PASS elsewhere; the report
    // actor echoes the aggregated summary.
    let model = StubModel::new(|req: &ModelRequest| -> Result<String, ModelError> {
        match req.actor.as_str() {
            SCANNER_ACTOR => {
                let dimension = req
                    .inputs
                    .pointer("/job/dimension")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let verdict = if matches!(dimension, "security" | "tests") {
                    "fail"
                } else {
                    "pass"
                };
                Ok(json!({
                    "dimension": dimension,
                    "verdict": verdict,
                    "severity": if verdict == "fail" { "high" } else { "none" },
                    "rationale": "stub",
                })
                .to_string())
            }
            SUMMARY_ACTOR => {
                let summary = req.inputs.get("summary").cloned().unwrap_or(json!({}));
                Ok(summary.to_string())
            }
            other => Ok(json!({ "actor": other }).to_string()),
        }
    });

    let actor = with_roles(ModelActor::new(model));
    let manifest = runner(Arc::new(actor))
        .run(&scan_spec(), "quality-scan-test".to_string(), json!({}))
        .await
        .expect("scan runs");

    // One recorded scanner verdict per dimension.
    let verdicts: Vec<_> = manifest
        .recorded
        .iter()
        .filter(|r| matches!(&r.call, CallKind::Actor { actor } if actor == SCANNER_ACTOR))
        .collect();
    assert_eq!(
        verdicts.len(),
        DIMENSIONS.len(),
        "one verdict per dimension"
    );
    let failed = verdicts
        .iter()
        .filter(|r| r.outputs["verdict"] == json!("fail"))
        .count();
    assert_eq!(failed, 2, "security + tests failed");

    // The reduce fed a report actor with the correct aggregate.
    let summary = manifest
        .recorded
        .iter()
        .find(|r| matches!(&r.call, CallKind::Actor { actor } if actor == SUMMARY_ACTOR))
        .expect("summary was recorded");
    assert_eq!(summary.outputs["verdict"], json!("fail"));
    assert_eq!(summary.outputs["passed"], json!(6));
    assert_eq!(summary.outputs["failed"], json!(2));
}

/// A stub model that passes every dimension (so the scan completes cleanly).
fn passing_scan_manifest() -> impl std::future::Future<Output = jesterky_contract::RunManifest> {
    let model = StubModel::new(|req: &ModelRequest| -> Result<String, ModelError> {
        match req.actor.as_str() {
            SCANNER_ACTOR => {
                let dimension = req
                    .inputs
                    .pointer("/job/dimension")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                Ok(
                    json!({ "dimension": dimension, "verdict": "pass", "severity": "none" })
                        .to_string(),
                )
            }
            SUMMARY_ACTOR => Ok(req
                .inputs
                .get("summary")
                .cloned()
                .unwrap_or(json!({}))
                .to_string()),
            other => Ok(json!({ "actor": other }).to_string()),
        }
    });
    let actor = with_roles(ModelActor::new(model));
    async move {
        runner(Arc::new(actor))
            .run(&scan_spec(), "process-tree-io".to_string(), json!({}))
            .await
            .expect("scan runs")
    }
}

fn leaves<'a>(node: &'a ProcessNode, out: &mut Vec<&'a ProcessNode>) {
    if node.children.is_empty() {
        out.push(node);
    }
    for child in &node.children {
        leaves(child, out);
    }
}

fn find_label<'a>(node: &'a ProcessNode, label: &str) -> Option<&'a ProcessNode> {
    if node.label == label {
        return Some(node);
    }
    node.children.iter().find_map(|c| find_label(c, label))
}

// Regression guard against a hollow trace (Tier-1 "process tree optimizer-starved").
// The optimizer needs (inputs -> outputs [-> score]) per call; these assert the
// runner fills them instead of leaving `null`.

#[tokio::test]
async fn process_tree_carries_leaf_inputs() {
    let manifest = passing_scan_manifest().await;
    let trace = manifest.trace.as_ref().expect("trace present");
    let mut ls = Vec::new();
    leaves(trace, &mut ls);
    let scanner_leaves: Vec<_> = ls
        .iter()
        .filter(|n| n.label == format!("actor:{SCANNER_ACTOR}"))
        .collect();
    assert_eq!(
        scanner_leaves.len(),
        DIMENSIONS.len(),
        "one scanner leaf per dim"
    );
    for leaf in &scanner_leaves {
        assert!(
            leaf.inputs.get("job").is_some(),
            "scanner leaf must carry its bound `job` input, got {:?}",
            leaf.inputs
        );
    }
}

#[tokio::test]
async fn process_tree_root_outputs_are_the_ledger() {
    let manifest = passing_scan_manifest().await;
    let trace = manifest.trace.as_ref().expect("trace present");
    // Root outputs are the settled ledger: the aggregate's `summary` slot is there.
    assert!(
        trace.outputs.get("summary").is_some(),
        "root outputs should snapshot the ledger (summary slot), got {:?}",
        trace.outputs
    );
    assert_eq!(trace.outputs["summary"]["verdict"], json!("pass"));
}

#[tokio::test]
async fn reduce_node_carries_a_score() {
    let manifest = passing_scan_manifest().await;
    let trace = manifest.trace.as_ref().expect("trace present");
    let aggregate = find_label(trace, "aggregate").expect("aggregate reduce node in trace");
    assert!(
        aggregate.score.is_some(),
        "reduce node should surface a score, got None"
    );
}

#[tokio::test]
async fn clean_run_has_completed_stop_reason() {
    let manifest = passing_scan_manifest().await;
    assert_eq!(manifest.stop_reason, RunStopReason::Completed);
}

#[tokio::test]
async fn manifest_invariants_all_pass_on_a_clean_scan() {
    let manifest = passing_scan_manifest().await;
    let report = manifest
        .invariants
        .as_ref()
        .expect("invariant report attached");
    assert!(
        report.all_ok,
        "expected all invariants to pass, failures: {:?}",
        report.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>()
    );
    // The map fan node was actually checked (not silently skipped).
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name.contains("scan_jobs") && c.name.contains("over_matches_total")),
        "map fan/gate invariant should be present, got: {:?}",
        report.checks.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    assert!(report
        .checks
        .iter()
        .any(|c| c.name == "no_orphaned_records"));
    assert!(report.checks.iter().any(|c| c.name == "unique_event_addrs"));
}

#[test]
fn expand_produces_one_job_per_dimension() {
    let reg = programs();
    let expand = reg.get("quality.expand").expect("expand registered");
    let out = expand(&Ledger::new(), &json!({ "target": "crates/foo" })).expect("expand ok");
    let jobs = out["jobs"].as_array().expect("jobs array");
    assert_eq!(jobs.len(), DIMENSIONS.len());
    assert_eq!(jobs[0]["target"], json!("crates/foo"));
    assert_eq!(jobs[1]["dimension"], json!(DIMENSIONS[1]));
}

#[test]
fn aggregate_counts_and_verdicts() {
    let reg = programs();
    let aggregate = reg.get("quality.aggregate").expect("aggregate registered");
    let scans = json!([
        { "dimension": "correctness", "verdict": "pass" },
        { "dimension": "security", "verdict": "fail" },
        { "dimension": "tests", "verdict": "fail" },
        { "dimension": "docs", "verdict": "pass" },
    ]);
    let out = aggregate(&Ledger::new(), &json!({ "scans": scans })).expect("aggregate ok");
    let summary = &out["summary"];
    assert_eq!(summary["total"], json!(4));
    assert_eq!(summary["passed"], json!(2));
    assert_eq!(summary["failed"], json!(2));
    assert_eq!(summary["verdict"], json!("fail"));
    assert_eq!(summary["failing_dimensions"], json!(["security", "tests"]));
}

/// Real codex scan of the jesterky repo. Ignored — needs the codex CLI +
/// ChatGPT-bundle auth + network. Run manually:
///   cargo test -p jesterky-quality -- --ignored codex_live_scan
#[tokio::test]
#[ignore = "requires codex CLI + ChatGPT-bundle auth + network"]
async fn codex_live_scan() {
    let actor = with_roles(ModelActor::new(CodexModel::gpt55()));
    let manifest = runner(Arc::new(actor))
        .run(&scan_spec(), "quality-scan-live".to_string(), json!({}))
        .await
        .expect("live scan runs");
    let verdicts = manifest
        .recorded
        .iter()
        .filter(|r| matches!(&r.call, CallKind::Actor { actor } if actor == SCANNER_ACTOR))
        .count();
    assert_eq!(verdicts, DIMENSIONS.len(), "one real verdict per dimension");
}
