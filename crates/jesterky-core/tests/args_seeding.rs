//! Run args seed the ledger, so a spec parameterizes itself via `ledger.<key>`.

use jesterky_actor::{FakeActor, MemArtifactStore, MemEventSink, ReplayClock};
use jesterky_contract::{Bindings, Node, NodeKind, Ref, RunPlan, WorkflowSpec};
use jesterky_core::{Actor, ProgramRegistry, Runner};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[tokio::test]
async fn run_args_are_addressable_as_ledger_keys() {
    // A single actor whose input is bound to `ledger.target` — which is only set
    // if the run args seeded it. FakeActor echoes inputs, so the recorded output
    // proves the arg reached the binding.
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "probe".to_string(),
        Node {
            kind: NodeKind::Actor {
                actor: "prober".to_string(),
            },
            inputs: [("target".to_string(), Ref("ledger.target".to_string()))]
                .into_iter()
                .collect::<Bindings>(),
            outputs: Bindings::new(),
        },
    );
    let spec = WorkflowSpec {
        name: "args_probe".to_string(),
        entrypoint: vec!["probe".to_string()],
        nodes,
        runplan: RunPlan::default(),
    };

    let runner = Runner {
        programs: ProgramRegistry::new(),
        actor: Arc::new(FakeActor) as Arc<dyn Actor>,
        resource: None,
        sink: Arc::new(MemEventSink::new()),
        clock: Arc::new(ReplayClock::default()),
        store: Arc::new(MemArtifactStore::new()),
        checkpoints: None,
    };

    let manifest = runner
        .run(&spec, "args-run".to_string(), json!({ "target": "crates/foo" }))
        .await
        .expect("run completes");

    let probe = manifest.recorded.first().expect("prober recorded");
    assert_eq!(probe.outputs["target"], json!("crates/foo"));
}
