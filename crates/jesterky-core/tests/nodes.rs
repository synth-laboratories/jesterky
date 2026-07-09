use jesterky_actor::{FakeActor, MemArtifactStore, MemEventSink, ReplayClock};
use jesterky_contract::{
    Bindings, CallKind, EventKind, Node, NodeKind, NodePath, PathSeg, Ref, RunPlan, RunStatus,
    WorkflowSpec,
};
use jesterky_core::{Actor, CoreError, ProgramRegistry, Runner};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[tokio::test]
async fn map_min_success_gate_allows_partial_success_above_threshold() {
    let manifest = runner(Arc::new(FakeActor))
        .run(
            &map_min_success_spec(0.5),
            "map-min-success-pass".to_string(),
            json!({}),
        )
        .await
        .expect("map passes min_success");

    assert_eq!(manifest.status, RunStatus::Completed);
    let completed = manifest
        .events
        .iter()
        .find(|event| event.kind == EventKind::MapCompleted)
        .expect("map completion event emitted");
    assert_eq!(completed.payload["successes"], json!(2));
    assert_eq!(completed.payload["total"], json!(3));

    let recorded = actor_record(&manifest, "recorder");
    assert_eq!(recorded.outputs["values"], json!([1, null, 3]));
}

#[tokio::test]
async fn map_min_success_gate_fails_below_threshold() {
    // A tripped gate STOPS the pipeline but still finalizes a `Failed` manifest
    // (the run's events + recorded outputs survive to inspect) — `run()` returns
    // `Ok` with `status: Failed`, and the reason rides a `WorkflowFailed` event.
    let manifest = runner(Arc::new(FakeActor))
        .run(
            &map_min_success_spec(0.75),
            "map-min-success-fail".to_string(),
            json!({}),
        )
        .await
        .expect("run finalizes a failed manifest, not a bare error");

    assert_eq!(manifest.status, RunStatus::Failed);
    let reason = manifest
        .events
        .iter()
        .find(|e| matches!(e.kind, EventKind::WorkflowFailed))
        .and_then(|e| e.payload.get("error").and_then(|v| v.as_str()))
        .expect("WorkflowFailed event carries the reason");
    assert!(
        reason.contains("min_success gate failed") && reason.contains("2/3"),
        "unexpected reason: {reason}"
    );
}

#[tokio::test]
async fn for_each_side_effects_are_visible_across_items() {
    let manifest = runner(Arc::new(FakeActor))
        .run(&for_each_spec(), "for-each-run".to_string(), json!({}))
        .await
        .expect("for_each run completes");

    let recorded = actor_record(&manifest, "recorder");
    assert_eq!(recorded.outputs["total"], json!(6));

    let body_paths = manifest
        .events
        .iter()
        .filter(|event| event.kind == EventKind::NodeStarted)
        .map(|event| event.addr.node_path.clone())
        .filter(|path| {
            path.0
                .first()
                .is_some_and(|seg| *seg == PathSeg::Node("accumulate".to_string()))
                && path.0.iter().any(|seg| matches!(seg, PathSeg::Index(_)))
        })
        .collect::<Vec<_>>();
    assert_eq!(body_paths.len(), 3);
}

#[tokio::test]
async fn nested_map_records_nested_addr_paths() {
    let manifest = runner(Arc::new(FakeActor))
        .run(&nested_map_spec(), "nested-map-run".to_string(), json!({}))
        .await
        .expect("nested map run completes");

    let recorded = actor_record(&manifest, "recorder");
    assert_eq!(recorded.outputs["groups"], json!([[1, 2], [3]]));

    let actor_paths = manifest
        .recorded
        .iter()
        .filter(|record| matches!(&record.call, CallKind::Actor { actor } if actor == "leaf"))
        .map(|record| record.addr.node_path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        actor_paths,
        vec![
            NodePath(vec![
                PathSeg::Node("outer".to_string()),
                PathSeg::Index(0),
                PathSeg::Index(0),
            ]),
            NodePath(vec![
                PathSeg::Node("outer".to_string()),
                PathSeg::Index(0),
                PathSeg::Index(1),
            ]),
            NodePath(vec![
                PathSeg::Node("outer".to_string()),
                PathSeg::Index(1),
                PathSeg::Index(0),
            ]),
        ]
    );
}

#[tokio::test]
async fn reduce_folds_map_outputs() {
    let manifest = runner(Arc::new(FakeActor))
        .run(&reduce_spec(), "reduce-run".to_string(), json!({}))
        .await
        .expect("reduce run completes");

    let recorded = actor_record(&manifest, "recorder");
    assert_eq!(recorded.outputs["sum"], json!(12));
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
        "nodes.seed_partial",
        Arc::new(|_, _| {
            Ok(json!({
                "items": [
                    { "value": 1, "fail": false },
                    { "value": 2, "fail": true },
                    { "value": 3, "fail": false }
                ]
            }))
        }),
    );
    programs.register(
        "nodes.maybe",
        Arc::new(|_, inputs| {
            if inputs["item"]["fail"].as_bool().unwrap_or(false) {
                return Err(CoreError::UnknownProgram(
                    "planned item failure".to_string(),
                ));
            }
            Ok(json!({ "value": inputs["item"]["value"].clone() }))
        }),
    );
    programs.register(
        "nodes.seed_deltas",
        Arc::new(|_, _| {
            Ok(json!({
                "total": 0,
                "deltas": [
                    { "amount": 1 },
                    { "amount": 2 },
                    { "amount": 3 }
                ]
            }))
        }),
    );
    programs.register(
        "nodes.add_delta",
        Arc::new(|_, inputs| {
            let total = inputs["total"].as_i64().expect("total is an integer");
            let amount = inputs["delta"]["amount"]
                .as_i64()
                .expect("amount is an integer");
            Ok(json!({ "total": total + amount }))
        }),
    );
    programs.register(
        "nodes.seed_groups",
        Arc::new(|_, _| {
            Ok(json!({
                "groups": [
                    { "items": [{ "value": 1 }, { "value": 2 }] },
                    { "items": [{ "value": 3 }] }
                ]
            }))
        }),
    );
    programs.register(
        "nodes.seed_numbers",
        Arc::new(|_, _| Ok(json!({ "numbers": [1, 2, 3] }))),
    );
    programs.register(
        "nodes.double",
        Arc::new(|_, inputs| {
            let value = inputs["value"].as_i64().expect("value is an integer");
            Ok(json!({ "value": value * 2 }))
        }),
    );
    programs.register(
        "nodes.sum",
        Arc::new(|_, inputs| {
            let sum = inputs["values"]
                .as_array()
                .expect("values is an array")
                .iter()
                .map(|value| value.as_i64().expect("value is an integer"))
                .sum::<i64>();
            Ok(json!({ "sum": sum }))
        }),
    );
    programs
}

fn map_min_success_spec(min_success: f64) -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "seed".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "nodes.seed_partial".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([("items", "ledger.items")]),
        },
    );
    nodes.insert(
        "scan".to_string(),
        Node {
            kind: NodeKind::Map {
                over: Ref("ledger.items".to_string()),
                item_as: "item".to_string(),
                concurrency: Some(2),
                min_success,
                body: Box::new(Node {
                    kind: NodeKind::Program {
                        op: "nodes.maybe".to_string(),
                    },
                    inputs: bindings([("item", "item")]),
                    outputs: bindings([("value", "ledger.value")]),
                }),
            },
            inputs: Bindings::new(),
            outputs: bindings([("value", "ledger.values")]),
        },
    );
    nodes.insert(
        "record".to_string(),
        record_node([("values", "ledger.values")]),
    );

    WorkflowSpec {
        name: "map_min_success".to_string(),
        entrypoint: vec!["seed".to_string(), "scan".to_string(), "record".to_string()],
        nodes,
        runplan: RunPlan::default(),
        host: None,
    }
}

fn for_each_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "seed".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "nodes.seed_deltas".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([("total", "ledger.total"), ("deltas", "ledger.deltas")]),
        },
    );
    nodes.insert(
        "accumulate".to_string(),
        Node {
            kind: NodeKind::ForEach {
                over: Ref("ledger.deltas".to_string()),
                item_as: "delta".to_string(),
                body: Box::new(Node {
                    kind: NodeKind::Program {
                        op: "nodes.add_delta".to_string(),
                    },
                    inputs: bindings([("total", "ledger.total"), ("delta", "delta")]),
                    outputs: bindings([("total", "ledger.total")]),
                }),
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );
    nodes.insert(
        "record".to_string(),
        record_node([("total", "ledger.total")]),
    );

    WorkflowSpec {
        name: "for_each_side_effects".to_string(),
        entrypoint: vec![
            "seed".to_string(),
            "accumulate".to_string(),
            "record".to_string(),
        ],
        nodes,
        runplan: RunPlan::default(),
        host: None,
    }
}

fn nested_map_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "seed".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "nodes.seed_groups".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([("groups", "ledger.groups")]),
        },
    );
    nodes.insert(
        "outer".to_string(),
        Node {
            kind: NodeKind::Map {
                over: Ref("ledger.groups".to_string()),
                item_as: "group".to_string(),
                concurrency: None,
                min_success: 1.0,
                body: Box::new(Node {
                    kind: NodeKind::Map {
                        over: Ref("group.items".to_string()),
                        item_as: "leaf".to_string(),
                        concurrency: None,
                        min_success: 1.0,
                        body: Box::new(Node {
                            kind: NodeKind::Actor {
                                actor: "leaf".to_string(),
                            },
                            inputs: bindings([("value", "leaf.value")]),
                            outputs: bindings([("value", "ledger.value")]),
                        }),
                    },
                    inputs: Bindings::new(),
                    outputs: bindings([("value", "ledger.values")]),
                }),
            },
            inputs: Bindings::new(),
            outputs: bindings([("value", "ledger.groups")]),
        },
    );
    nodes.insert(
        "record".to_string(),
        record_node([("groups", "ledger.groups")]),
    );

    WorkflowSpec {
        name: "nested_map".to_string(),
        entrypoint: vec![
            "seed".to_string(),
            "outer".to_string(),
            "record".to_string(),
        ],
        nodes,
        runplan: RunPlan::default(),
        host: None,
    }
}

fn reduce_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "seed".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "nodes.seed_numbers".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([("numbers", "ledger.numbers")]),
        },
    );
    nodes.insert(
        "double".to_string(),
        Node {
            kind: NodeKind::Map {
                over: Ref("ledger.numbers".to_string()),
                item_as: "number".to_string(),
                concurrency: Some(2),
                min_success: 1.0,
                body: Box::new(Node {
                    kind: NodeKind::Program {
                        op: "nodes.double".to_string(),
                    },
                    inputs: bindings([("value", "number")]),
                    outputs: bindings([("value", "ledger.value")]),
                }),
            },
            inputs: Bindings::new(),
            outputs: bindings([("value", "ledger.values")]),
        },
    );
    nodes.insert(
        "sum".to_string(),
        Node {
            kind: NodeKind::Reduce {
                op: "nodes.sum".to_string(),
            },
            inputs: bindings([("values", "ledger.values")]),
            outputs: bindings([("sum", "ledger.sum")]),
        },
    );
    nodes.insert("record".to_string(), record_node([("sum", "ledger.sum")]));

    WorkflowSpec {
        name: "reduce".to_string(),
        entrypoint: vec![
            "seed".to_string(),
            "double".to_string(),
            "sum".to_string(),
            "record".to_string(),
        ],
        nodes,
        runplan: RunPlan::default(),
        host: None,
    }
}

fn record_node<const N: usize>(inputs: [(&str, &str); N]) -> Node {
    Node {
        kind: NodeKind::Actor {
            actor: "recorder".to_string(),
        },
        inputs: bindings(inputs),
        outputs: Bindings::new(),
    }
}

fn actor_record<'a>(
    manifest: &'a jesterky_contract::RunManifest,
    actor_name: &str,
) -> &'a jesterky_contract::RecordedOutput {
    manifest
        .recorded
        .iter()
        .find(|record| matches!(&record.call, CallKind::Actor { actor } if actor == actor_name))
        .expect("actor was recorded")
}

fn bindings<const N: usize>(pairs: [(&str, &str); N]) -> Bindings {
    pairs
        .into_iter()
        .map(|(name, r)| (name.to_string(), Ref(r.to_string())))
        .collect()
}
