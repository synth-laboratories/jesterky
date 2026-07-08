use jesterky_contract::{
    manifest_schema_json, workflow_schema_json, Addr, Artifact, ArtifactRef, Bindings, CallKind,
    Checkpoint, Event, EventKind, Limit, Node, NodeKind, NodePath, PathSeg, ProcessNode,
    RecordedOutput, Ref, RunManifest, RunPlan, RunStatus, WorkflowSpec,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn workflow_and_manifest_round_trip_through_json() {
    let spec = workflow_spec();
    let manifest = run_manifest(&spec);

    assert_json_roundtrip(&spec);
    assert_json_roundtrip(&manifest);
}

#[test]
fn schemas_emit_parseable_json_with_node_kind_variants() {
    let workflow_schema: serde_json::Value =
        serde_json::from_str(&workflow_schema_json()).expect("workflow schema parses");
    let manifest_schema: serde_json::Value =
        serde_json::from_str(&manifest_schema_json()).expect("manifest schema parses");

    assert_eq!(workflow_schema["title"], json!("WorkflowSpec"));
    assert_eq!(manifest_schema["title"], json!("RunManifest"));

    let workflow_schema_text = workflow_schema.to_string();
    for variant in [
        "program",
        "reduce",
        "actor",
        "map",
        "for_each",
        "while",
        "branch",
        "session_group",
        "resume_session",
    ] {
        assert!(
            workflow_schema_text.contains(variant),
            "workflow schema contains NodeKind variant `{variant}`"
        );
    }
}

fn assert_json_roundtrip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let serialized = serde_json::to_string(value).expect("value serializes");
    let deserialized: T = serde_json::from_str(&serialized).expect("value deserializes");
    assert_eq!(
        serde_json::to_value(value).expect("original converts to JSON"),
        serde_json::to_value(deserialized).expect("round-tripped converts to JSON")
    );
}

fn workflow_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "seed".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "quality.seed".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([
                ("jobs", "ledger.jobs"),
                ("keep_going", "ledger.keep_going"),
                ("sessions", "ledger.sessions"),
            ]),
        },
    );
    nodes.insert(
        "scan".to_string(),
        Node {
            kind: NodeKind::Map {
                over: Ref("ledger.jobs".to_string()),
                item_as: "item".to_string(),
                concurrency: Some(2),
                min_success: 0.5,
                body: Box::new(Node {
                    kind: NodeKind::Actor {
                        actor: "quality_scanner".to_string(),
                    },
                    inputs: bindings([("job", "item")]),
                    outputs: bindings([("scan", "ledger.scan")]),
                }),
            },
            inputs: Bindings::new(),
            outputs: bindings([("scan", "ledger.scans")]),
        },
    );
    nodes.insert(
        "refine".to_string(),
        Node {
            kind: NodeKind::While {
                cond: Ref("ledger.keep_going".to_string()),
                body: Box::new(Node {
                    kind: NodeKind::Program {
                        op: "quality.refine".to_string(),
                    },
                    inputs: bindings([("scans", "ledger.scans")]),
                    outputs: bindings([("keep_going", "ledger.keep_going")]),
                }),
                max_iters: 3,
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );
    nodes.insert(
        "sessions".to_string(),
        Node {
            kind: NodeKind::SessionGroup {
                sessions: Ref("ledger.sessions".to_string()),
                actor: "session_worker".to_string(),
                body: Box::new(Node {
                    kind: NodeKind::Actor {
                        actor: "session_worker".to_string(),
                    },
                    inputs: bindings([("session", "item")]),
                    outputs: Bindings::new(),
                }),
                limit: Some(Limit {
                    name: "turn".to_string(),
                    permits: 1,
                }),
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );

    WorkflowSpec {
        name: "roundtrip_quality".to_string(),
        entrypoint: vec![
            "seed".to_string(),
            "scan".to_string(),
            "refine".to_string(),
            "sessions".to_string(),
        ],
        nodes,
        runplan: RunPlan {
            limits: BTreeMap::from([("turn".to_string(), 1)]),
            map_concurrency: Some(2),
            ..RunPlan::default()
        },
    }
}

fn run_manifest(spec: &WorkflowSpec) -> RunManifest {
    let root = Addr {
        run_id: "roundtrip-run".to_string(),
        node_path: NodePath::root(),
        iteration: 0,
        local_seq: 0,
    };
    let actor_addr = Addr {
        run_id: "roundtrip-run".to_string(),
        node_path: NodePath(vec![PathSeg::Node("scan".to_string()), PathSeg::Index(0)]),
        iteration: 0,
        local_seq: 1,
    };
    let artifact = ArtifactRef {
        key: "blob/scan-0".to_string(),
        size_bytes: 17,
        content_type: "application/json".to_string(),
    };

    RunManifest {
        run_id: "roundtrip-run".to_string(),
        workflow_name: spec.name.clone(),
        spec_hash: spec.validate_and_hash().expect("spec validates"),
        args: json!({ "seed": 1 }),
        events: vec![
            Event {
                addr: root.clone(),
                kind: EventKind::WorkflowStarted,
                payload: json!({ "seed": 1 }),
                wall_ms: 0,
            },
            Event {
                addr: actor_addr.clone(),
                kind: EventKind::ActorInvoked,
                payload: json!({ "actor": "quality_scanner" }),
                wall_ms: 1,
            },
        ],
        recorded: vec![RecordedOutput {
            addr: actor_addr.clone(),
            call: CallKind::Actor {
                actor: "quality_scanner".to_string(),
            },
            outputs: json!({ "scan": { "ok": true } }),
            score: Some(0.75),
            signal: Some(json!({ "verifier": "pass" })),
            artifacts: vec![artifact.clone()],
        }],
        checkpoints: vec![Checkpoint {
            session: "alpha".to_string(),
            addr: root.clone(),
            state: Artifact::Inline(json!({ "turn": 1 })),
        }],
        trace: Some(ProcessNode {
            addr: root,
            label: "workflow:roundtrip_quality".to_string(),
            inputs: json!({}),
            outputs: json!({ "ok": true }),
            score: Some(0.75),
            signal: Some(json!({ "summary": "ok" })),
            artifacts: vec![artifact],
            children: Vec::new(),
        }),
        status: RunStatus::Completed,
    }
}

fn bindings<const N: usize>(pairs: [(&str, &str); N]) -> Bindings {
    pairs
        .into_iter()
        .map(|(name, r)| (name.to_string(), Ref(r.to_string())))
        .collect()
}
