use jesterky_actor::{FakeActor, MemArtifactStore, MemEventSink, ReplayActor, ReplayClock};
use jesterky_contract::{Bindings, Event, Node, NodeKind, Ref, RunPlan, RunStatus, WorkflowSpec};
use jesterky_core::{Actor, ProgramRegistry, Runner};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[tokio::test]
async fn map_reduce_replay_has_byte_identical_addr_sorted_events() {
    let spec = quality_scan_spec();

    let live = runner(Arc::new(FakeActor));
    let manifest = live
        .run(&spec, "replay-fidelity-run".to_string(), json!({}))
        .await
        .expect("live run completes");
    assert_eq!(manifest.status, RunStatus::Completed);
    assert!(
        !manifest.events.is_empty(),
        "event report should be non-empty"
    );
    assert!(
        !manifest.recorded.is_empty(),
        "actor recording should be non-empty"
    );

    let replay = runner(Arc::new(ReplayActor::from_manifest(&manifest)));
    let replay_manifest = replay
        .run(&spec, "replay-fidelity-run".to_string(), json!({}))
        .await
        .expect("replay run completes");

    assert_eq!(
        sorted_events_json(&manifest.events),
        sorted_events_json(&replay_manifest.events)
    );
}

fn runner(actor: Arc<dyn Actor>) -> Runner {
    Runner {
        programs: programs(),
        actor,
        resource: None,
        sink: Arc::new(MemEventSink::new()),
        clock: Arc::new(ReplayClock::default()),
        store: Arc::new(MemArtifactStore::new()),
        checkpoints: None,
    }
}

fn programs() -> ProgramRegistry {
    let mut programs = ProgramRegistry::new();
    programs.register(
        "quality.expand",
        Arc::new(|_, _| {
            Ok(json!({
                "jobs": [
                    { "id": 0, "target": "alpha" },
                    { "id": 1, "target": "beta" },
                    { "id": 2, "target": "gamma" }
                ]
            }))
        }),
    );
    programs.register(
        "quality.aggregate",
        Arc::new(|_, inputs| {
            let scans = inputs
                .get("scans")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            Ok(json!({
                "summary": {
                    "count": scans.len(),
                    "first": scans.first().cloned()
                }
            }))
        }),
    );
    programs
}

fn quality_scan_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "expand_jobs".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "quality.expand".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([("jobs", "ledger.jobs")]),
        },
    );
    nodes.insert(
        "scan_jobs".to_string(),
        Node {
            kind: NodeKind::Map {
                over: Ref("ledger.jobs".to_string()),
                item_as: "item".to_string(),
                concurrency: Some(2),
                min_success: 1.0,
                body: Box::new(Node {
                    kind: NodeKind::Actor {
                        actor: "quality_scanner".to_string(),
                    },
                    inputs: bindings([("job", "item")]),
                    outputs: bindings([("job", "ledger.scan")]),
                }),
            },
            inputs: Bindings::new(),
            outputs: bindings([("job", "ledger.scans")]),
        },
    );
    nodes.insert(
        "aggregate".to_string(),
        Node {
            kind: NodeKind::Reduce {
                op: "quality.aggregate".to_string(),
            },
            inputs: bindings([("scans", "ledger.scans")]),
            outputs: bindings([("summary", "ledger.summary")]),
        },
    );

    WorkflowSpec {
        name: "quality_scan".to_string(),
        entrypoint: vec![
            "expand_jobs".to_string(),
            "scan_jobs".to_string(),
            "aggregate".to_string(),
        ],
        nodes,
        runplan: RunPlan::default(),
        host: None,
    }
}

fn bindings<const N: usize>(pairs: [(&str, &str); N]) -> Bindings {
    pairs
        .into_iter()
        .map(|(name, r)| (name.to_string(), Ref(r.to_string())))
        .collect()
}

fn sorted_events_json(events: &[Event]) -> String {
    let mut events = events.to_vec();
    events.sort_by(|a, b| a.addr.cmp(&b.addr));
    serde_json::to_string(&events).expect("events serialize")
}
