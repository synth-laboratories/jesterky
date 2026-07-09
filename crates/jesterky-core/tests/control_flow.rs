use jesterky_actor::{FakeActor, MemArtifactStore, MemEventSink, ReplayClock};
use jesterky_contract::{
    Bindings, EventKind, Node, NodeKind, NodePath, Ref, RunPlan, WorkflowSpec,
};
use jesterky_core::{Actor, ProgramRegistry, Runner};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[tokio::test]
async fn while_iterations_feed_branch_and_stamp_body_events() {
    let spec = control_flow_spec();
    let manifest = runner(Arc::new(FakeActor))
        .run(&spec, "control-flow-run".to_string(), json!({}))
        .await
        .expect("control-flow run completes");

    let final_record = manifest
        .recorded
        .iter()
        .find(|record| matches!(&record.call, jesterky_contract::CallKind::Actor { actor } if actor == "final_recorder"))
        .expect("final recorder was invoked");
    assert_eq!(final_record.outputs["final"], json!(3));
    assert_eq!(final_record.outputs["branch"], json!("then"));

    let while_body_path = NodePath::root().child("loop").child("body");
    let body_iterations = manifest
        .events
        .iter()
        .filter(|event| {
            event.kind == EventKind::NodeStarted && event.addr.node_path == while_body_path
        })
        .map(|event| event.addr.iteration)
        .collect::<Vec<_>>();
    assert_eq!(body_iterations, vec![0, 1, 2]);
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
        "control.init",
        Arc::new(|_, _| {
            Ok(json!({
                "counter": 0,
                "keep_going": true,
                "threshold": 3
            }))
        }),
    );
    programs.register(
        "control.increment",
        Arc::new(|_, inputs| {
            let counter = inputs["counter"].as_i64().expect("counter is an integer") + 1;
            let threshold = inputs["threshold"]
                .as_i64()
                .expect("threshold is an integer");
            Ok(json!({
                "counter": counter,
                "keep_going": counter < threshold
            }))
        }),
    );
    programs.register(
        "control.then",
        Arc::new(|_, inputs| {
            Ok(json!({
                "final": inputs["counter"].clone(),
                "branch": "then"
            }))
        }),
    );
    programs.register(
        "control.otherwise",
        Arc::new(|_, inputs| {
            Ok(json!({
                "final": inputs["counter"].clone(),
                "branch": "otherwise"
            }))
        }),
    );
    programs
}

fn control_flow_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "init".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "control.init".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([
                ("counter", "ledger.counter"),
                ("keep_going", "ledger.keep_going"),
                ("threshold", "ledger.threshold"),
            ]),
        },
    );
    nodes.insert(
        "loop".to_string(),
        Node {
            kind: NodeKind::While {
                cond: Ref("ledger.keep_going".to_string()),
                body: Box::new(Node {
                    kind: NodeKind::Program {
                        op: "control.increment".to_string(),
                    },
                    inputs: bindings([
                        ("counter", "ledger.counter"),
                        ("threshold", "ledger.threshold"),
                    ]),
                    outputs: bindings([
                        ("counter", "ledger.counter"),
                        ("keep_going", "ledger.keep_going"),
                    ]),
                }),
                max_iters: 10,
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );
    nodes.insert(
        "choose".to_string(),
        Node {
            kind: NodeKind::Branch {
                cond: Ref("ledger.counter".to_string()),
                then: "then_program".to_string(),
                otherwise: Some("otherwise_program".to_string()),
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );
    nodes.insert(
        "then_program".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "control.then".to_string(),
            },
            inputs: bindings([("counter", "ledger.counter")]),
            outputs: bindings([("final", "ledger.final"), ("branch", "ledger.branch")]),
        },
    );
    nodes.insert(
        "otherwise_program".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "control.otherwise".to_string(),
            },
            inputs: bindings([("counter", "ledger.counter")]),
            outputs: bindings([("final", "ledger.final"), ("branch", "ledger.branch")]),
        },
    );
    nodes.insert(
        "record_final".to_string(),
        Node {
            kind: NodeKind::Actor {
                actor: "final_recorder".to_string(),
            },
            inputs: bindings([("final", "ledger.final"), ("branch", "ledger.branch")]),
            outputs: bindings([
                ("final", "ledger.recorded_final"),
                ("branch", "ledger.recorded_branch"),
            ]),
        },
    );

    WorkflowSpec {
        name: "control_flow".to_string(),
        entrypoint: vec![
            "init".to_string(),
            "loop".to_string(),
            "choose".to_string(),
            "record_final".to_string(),
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
