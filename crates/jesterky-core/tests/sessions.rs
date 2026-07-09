//! Session tests — the "poll under a center logic" joint (DungeonGrid).
//!
//! `session_group_serializes_turns_under_permit_one` proves the load-bearing
//! property: a `session_group` whose `limit` has `permits = 1` never lets two
//! sessions hold the turn at once, so concurrent sessions take strict turns
//! through a shared resource. `resume_session_rehydrates_checkpoint` proves the
//! resume path binds saved state into the body.

use jesterky_actor::{FakeActor, MemArtifactStore, MemCheckpointStore, MemEventSink, ReplayClock};
use jesterky_contract::{
    Bindings, CallKind, EventKind, Limit, Node, NodeKind, ProcessNode, Ref, RunPlan, WorkflowSpec,
};
use jesterky_core::{CheckpointStore, ProgramRegistry, Runner};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

#[tokio::test]
async fn session_group_serializes_turns_under_permit_one() {
    let spec = session_group_spec();
    let manifest = runner(None)
        .run(&spec, "session-run".to_string(), json!({}))
        .await
        .expect("session run completes");

    // One SessionStarted per session.
    let started = manifest
        .events
        .iter()
        .filter(|e| e.kind == EventKind::SessionStarted)
        .count();
    assert_eq!(started, 3, "one SessionStarted per session");

    // The central invariant: walking acquire/release in emit order, at most one
    // permit is ever held — sessions never overlap under permits = 1.
    let mut held = 0i32;
    let mut max_held = 0i32;
    for e in &manifest.events {
        match e.kind {
            EventKind::SemaphoreAcquired => {
                held += 1;
                max_held = max_held.max(held);
            }
            EventKind::SemaphoreReleased => held -= 1,
            _ => {}
        }
    }
    assert_eq!(held, 0, "every acquire is released");
    assert_eq!(
        max_held, 1,
        "permits=1 serializes: two sessions never hold the turn at once"
    );

    // Each session's actor ran → three recorded `hero` calls, each at a distinct
    // session node_path (so replay can disambiguate them).
    let hero_calls: Vec<_> = manifest
        .recorded
        .iter()
        .filter(|r| matches!(&r.call, CallKind::Actor { actor } if actor == "hero"))
        .collect();
    assert_eq!(hero_calls.len(), 3, "one hero call per session");
    let distinct_paths: std::collections::HashSet<_> = hero_calls
        .iter()
        .map(|r| r.addr.node_path.clone())
        .collect();
    assert_eq!(
        distinct_paths.len(),
        3,
        "each session records under its own path"
    );

    // The trace tree exists and carries one `actor:hero` leaf per session.
    let trace = manifest.trace.expect("trace tree built");
    assert_eq!(trace.label, "workflow:sessions");
    assert_eq!(count_leaves(&trace, "actor:hero"), 3);
}

#[tokio::test]
async fn resume_session_rehydrates_checkpoint() {
    // Pre-seed a checkpoint the resumed session should read back.
    let store = Arc::new(MemCheckpointStore::new());
    store
        .save("sess1", json!({ "turn": 7 }))
        .await
        .expect("checkpoint saved");

    let spec = resume_spec();
    let manifest = runner(Some(store))
        .run(&spec, "resume-run".to_string(), json!({}))
        .await
        .expect("resume run completes");

    // The resume event fired for the right session.
    let resumed = manifest
        .events
        .iter()
        .find(|e| e.kind == EventKind::SessionResumed)
        .expect("a SessionResumed event");
    assert_eq!(resumed.payload["session"], json!("sess1"));

    // The body actor saw the rehydrated state (`session.turn` → 7), proving the
    // checkpoint was loaded and bound, not just that the arm ran.
    let recall = manifest
        .recorded
        .iter()
        .find(|r| matches!(&r.call, CallKind::Actor { actor } if actor == "recall"))
        .expect("recall actor ran");
    assert_eq!(recall.outputs["turn"], json!(7));
}

fn runner(checkpoints: Option<Arc<MemCheckpointStore>>) -> Runner {
    Runner {
        programs: programs(),
        actor: Arc::new(FakeActor),
        resource: None,
        sink: Arc::new(MemEventSink::new()),
        clock: Arc::new(ReplayClock::default()),
        store: Arc::new(MemArtifactStore::new()),
        checkpoints: checkpoints.map(|s| s as Arc<dyn CheckpointStore>),
    }
}

fn programs() -> ProgramRegistry {
    let mut programs = ProgramRegistry::new();
    programs.register(
        "sessions.seed",
        Arc::new(|_, _| {
            Ok(json!({
                "sessions": [
                    { "id": "alpha" },
                    { "id": "beta" },
                    { "id": "gamma" }
                ]
            }))
        }),
    );
    programs
}

fn session_group_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "seed".to_string(),
        Node {
            kind: NodeKind::Program {
                op: "sessions.seed".to_string(),
            },
            inputs: Bindings::new(),
            outputs: bindings([("sessions", "ledger.sessions")]),
        },
    );
    nodes.insert(
        "play".to_string(),
        Node {
            kind: NodeKind::SessionGroup {
                sessions: Ref("ledger.sessions".to_string()),
                actor: "hero".to_string(),
                body: Box::new(Node {
                    kind: NodeKind::Actor {
                        actor: "hero".to_string(),
                    },
                    inputs: bindings([("id", "item.id")]),
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
        name: "sessions".to_string(),
        entrypoint: vec!["seed".to_string(), "play".to_string()],
        nodes,
        runplan: RunPlan {
            limits: BTreeMap::from([("turn".to_string(), 1)]),
            ..RunPlan::default()
        },
        host: None,
    }
}

fn resume_spec() -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert(
        "resume".to_string(),
        Node {
            kind: NodeKind::ResumeSession {
                // A JSON-literal ref resolves to the session id string.
                session: Ref("\"sess1\"".to_string()),
                body: Box::new(Node {
                    kind: NodeKind::Actor {
                        actor: "recall".to_string(),
                    },
                    // Reads the rehydrated state bound as the `session` item.
                    inputs: bindings([("turn", "session.turn")]),
                    outputs: Bindings::new(),
                }),
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );

    WorkflowSpec {
        name: "resume".to_string(),
        entrypoint: vec!["resume".to_string()],
        nodes,
        runplan: RunPlan::default(),
        host: None,
    }
}

/// Count leaves with the given label anywhere in the trace tree.
fn count_leaves(node: &ProcessNode, label: &str) -> usize {
    let here = usize::from(node.label == label);
    here + node
        .children
        .iter()
        .map(|c| count_leaves(c, label))
        .sum::<usize>()
}

fn bindings<const N: usize>(pairs: [(&str, &str); N]) -> Bindings {
    pairs
        .into_iter()
        .map(|(name, r)| (name.to_string(), Ref(r.to_string())))
        .collect()
}
