//! Goals (semantic termination) evaluated by the runner against the ledger:
//! early success wrap-up (skip remaining entrypoints) and fail-on-unmet.

use jesterky_actor::{FakeActor, MemArtifactStore, MemEventSink, ReplayClock};
use jesterky_contract::{
    Bindings, CallKind, EventKind, GoalKind, GoalPlan, GoalSpec, GoalState, Node, NodeKind,
    NodePath, Ref, RunPlan, RunStatus, WorkflowSpec,
};
use jesterky_core::{Actor, ProgramRegistry, Runner};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn actor_node(actor: &str) -> Node {
    Node {
        kind: NodeKind::Actor {
            actor: actor.to_string(),
        },
        inputs: Bindings::new(),
        outputs: Bindings::new(),
    }
}

fn spec_with_goals(entrypoints: &[&str], goals: GoalPlan) -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    for id in entrypoints {
        nodes.insert((*id).to_string(), actor_node(id));
    }
    WorkflowSpec {
        name: "goals_spec".to_string(),
        entrypoint: entrypoints.iter().map(|s| s.to_string()).collect(),
        nodes,
        runplan: RunPlan {
            goals,
            ..RunPlan::default()
        },
        host: None,
    }
}

fn runner() -> Runner {
    Runner {
        programs: ProgramRegistry::new(),
        actor: Arc::new(FakeActor) as Arc<dyn Actor>,
        resource: None,
        sink: Arc::new(MemEventSink::new()),
        clock: Arc::new(ReplayClock::default()),
        store: Arc::new(MemArtifactStore::new()),
        checkpoints: None,
    }
}

fn threshold_goal(id: &str, path: &str, min: f64) -> GoalSpec {
    GoalSpec {
        id: id.to_string(),
        kind: GoalKind::MetricThreshold {
            path: path.to_string(),
            min,
        },
        required: true,
        label: None,
        show_progress: true,
    }
}

#[tokio::test]
async fn required_goal_met_completes_and_attaches_snapshot() {
    let plan = GoalPlan {
        goals: vec![threshold_goal("quality", "score", 0.8)],
        ..GoalPlan::default()
    };
    let spec = spec_with_goals(&["a"], plan);
    // Args seed the ledger: slot `score` = 0.9 satisfies the threshold.
    let manifest = runner()
        .run(&spec, "goal-met".to_string(), json!({ "score": 0.9 }))
        .await
        .expect("run completes");

    assert_eq!(manifest.status, RunStatus::Completed);
    let goals = manifest.goals.expect("goal snapshot attached");
    assert_eq!(goals.state, GoalState::Met);
    assert_eq!(goals.required_met, 1);
}

#[tokio::test]
async fn required_goal_unmet_fails_the_run() {
    let plan = GoalPlan {
        goals: vec![threshold_goal("quality", "score", 0.8)],
        ..GoalPlan::default()
    };
    let spec = spec_with_goals(&["a"], plan);
    // No `score` seeded → path missing → Unknown → unmet → fail_on_unmet.
    let manifest = runner()
        .run(&spec, "goal-unmet".to_string(), json!({}))
        .await
        .expect("run completes (finalizes Failed, not Err)");

    assert_eq!(manifest.status, RunStatus::Failed);
    let goals = manifest.goals.expect("goal snapshot attached");
    assert_eq!(goals.state, GoalState::Unmet);
    // A WorkflowFailed event carries the goal_unmet reason for the host.
    let failed = manifest
        .events
        .iter()
        .find(|e| e.kind == EventKind::WorkflowFailed)
        .expect("WorkflowFailed emitted");
    assert!(failed.payload["error"]
        .as_str()
        .unwrap()
        .contains("goal_unmet"));
}

#[tokio::test]
async fn met_goal_terminates_early_and_skips_remaining_entrypoints() {
    let plan = GoalPlan {
        goals: vec![threshold_goal("quality", "score", 0.8)],
        terminate_on_met: true,
        ..GoalPlan::default()
    };
    let spec = spec_with_goals(&["a", "b"], plan);
    // Goal is satisfiable from seeded args, so after entrypoint `a` completes the
    // runner skips `b` (early success wrap-up; no in-flight cancel in v1).
    let manifest = runner()
        .run(&spec, "goal-early".to_string(), json!({ "score": 0.9 }))
        .await
        .expect("run completes");

    assert_eq!(manifest.status, RunStatus::Completed);
    let goals = manifest.goals.expect("goal snapshot attached");
    assert!(goals.terminated_early, "should mark early termination");

    let ran_a = manifest.events.iter().any(|e| {
        e.kind == EventKind::NodeStarted && e.addr.node_path == NodePath::root().child("a")
    });
    let ran_b = manifest.events.iter().any(|e| {
        e.kind == EventKind::NodeStarted && e.addr.node_path == NodePath::root().child("b")
    });
    assert!(ran_a, "first entrypoint runs");
    assert!(!ran_b, "second entrypoint is skipped after goal met");
}

/// Spec: one warmup entrypoint plus a non-entrypoint `wrapup` actor named as the
/// goal plan's `finalize` node.
fn spec_with_finalize(goals: GoalPlan) -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert("warmup".to_string(), actor_node("noop"));
    nodes.insert("wrapup".to_string(), actor_node("finalizer"));
    WorkflowSpec {
        name: "finalize_spec".to_string(),
        entrypoint: vec!["warmup".to_string()],
        nodes,
        runplan: RunPlan {
            goals,
            ..RunPlan::default()
        },
        host: None,
    }
}

fn ran_actor(manifest: &jesterky_contract::RunManifest, actor_name: &str) -> bool {
    manifest
        .recorded
        .iter()
        .any(|r| matches!(&r.call, CallKind::Actor { actor } if actor == actor_name))
}

#[tokio::test]
async fn finalize_node_runs_on_early_success() {
    let plan = GoalPlan {
        goals: vec![GoalSpec {
            id: "flag".into(),
            kind: GoalKind::LedgerPred {
                path: "flag".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }],
        terminate_on_met: true,
        finalize: Some("wrapup".to_string()),
        ..GoalPlan::default()
    };
    let manifest = runner()
        .run(
            &spec_with_finalize(plan),
            "finalize-met".to_string(),
            json!({ "flag": true }),
        )
        .await
        .expect("run completes");
    assert_eq!(manifest.status, RunStatus::Completed);
    assert!(manifest.goals.as_ref().unwrap().terminated_early);
    assert!(
        ran_actor(&manifest, "finalizer"),
        "finalize node executed on success"
    );
}

#[tokio::test]
async fn finalize_node_is_skipped_when_goal_unmet() {
    let plan = GoalPlan {
        goals: vec![GoalSpec {
            id: "flag".into(),
            kind: GoalKind::LedgerPred {
                path: "flag".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }],
        terminate_on_met: true,
        finalize: Some("wrapup".to_string()),
        fail_on_unmet: false, // so the run completes; we only assert finalize skipped
        ..GoalPlan::default()
    };
    let manifest = runner()
        .run(
            &spec_with_finalize(plan),
            "finalize-unmet".to_string(),
            json!({ "flag": false }),
        )
        .await
        .expect("run completes");
    assert!(
        !ran_actor(&manifest, "finalizer"),
        "finalize skipped when goal unmet"
    );
}

/// Spec: a warmup actor entrypoint, then a `map` entrypoint fanning `worker`
/// over the seeded `items`. Goals are supplied by the caller.
fn warmup_then_map_spec(goals: GoalPlan) -> WorkflowSpec {
    let mut nodes = BTreeMap::new();
    nodes.insert("warmup".to_string(), actor_node("noop"));
    nodes.insert(
        "fanout".to_string(),
        Node {
            kind: NodeKind::Map {
                over: Ref("ledger.items".to_string()),
                item_as: "item".to_string(),
                concurrency: Some(1),
                min_success: 1.0,
                body: Box::new(Node {
                    kind: NodeKind::Actor {
                        actor: "worker".to_string(),
                    },
                    inputs: [("item".to_string(), Ref("item".to_string()))]
                        .into_iter()
                        .collect::<Bindings>(),
                    outputs: Bindings::new(),
                }),
            },
            inputs: Bindings::new(),
            outputs: Bindings::new(),
        },
    );
    WorkflowSpec {
        name: "cancel_spec".to_string(),
        entrypoint: vec!["warmup".to_string(), "fanout".to_string()],
        nodes,
        runplan: RunPlan {
            goals,
            ..RunPlan::default()
        },
        host: None,
    }
}

fn worker_calls(manifest: &jesterky_contract::RunManifest) -> usize {
    manifest
        .recorded
        .iter()
        .filter(|r| matches!(&r.call, CallKind::Actor { actor } if actor == "worker"))
        .count()
}

#[tokio::test]
async fn cancel_in_flight_skips_pending_map_items_once_goal_met() {
    // flag is seeded, so after `warmup` completes the required goal is met; with
    // terminate_on_met=false + cancel_in_flight=true the runner keeps going but
    // trips cancel, so the `fanout` map starts none of its 5 items.
    let plan = GoalPlan {
        goals: vec![GoalSpec {
            id: "flag".into(),
            kind: GoalKind::LedgerPred {
                path: "flag".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }],
        terminate_on_met: false,
        cancel_in_flight: true,
        ..GoalPlan::default()
    };
    let spec = warmup_then_map_spec(plan);
    let manifest = runner()
        .run(
            &spec,
            "cancel-on".to_string(),
            json!({ "flag": true, "items": [1, 2, 3, 4, 5] }),
        )
        .await
        .expect("run completes");

    assert_eq!(manifest.status, RunStatus::Completed);
    assert_eq!(worker_calls(&manifest), 0, "cancelled map runs no items");
    // A cancelled, deliberately-partial map must NOT trip the invariant self-check.
    assert!(manifest.invariants.as_ref().unwrap().all_ok);
}

#[tokio::test]
async fn without_cancel_flag_the_map_runs_all_items() {
    // Same met goal, but cancel_in_flight defaults false → the map runs fully.
    let plan = GoalPlan {
        goals: vec![GoalSpec {
            id: "flag".into(),
            kind: GoalKind::LedgerPred {
                path: "flag".into(),
                equals: json!(true),
            },
            required: true,
            label: None,
            show_progress: true,
        }],
        terminate_on_met: false,
        cancel_in_flight: false,
        ..GoalPlan::default()
    };
    let spec = warmup_then_map_spec(plan);
    let manifest = runner()
        .run(
            &spec,
            "cancel-off".to_string(),
            json!({ "flag": true, "items": [1, 2, 3, 4, 5] }),
        )
        .await
        .expect("run completes");
    assert_eq!(worker_calls(&manifest), 5, "no cancel → all items run");
}

#[tokio::test]
async fn terminate_disabled_runs_all_entrypoints_even_when_met() {
    let plan = GoalPlan {
        goals: vec![threshold_goal("quality", "score", 0.8)],
        terminate_on_met: false,
        ..GoalPlan::default()
    };
    let spec = spec_with_goals(&["a", "b"], plan);
    let manifest = runner()
        .run(&spec, "goal-noterm".to_string(), json!({ "score": 0.9 }))
        .await
        .expect("run completes");

    let goals = manifest.goals.expect("goal snapshot attached");
    assert_eq!(goals.state, GoalState::Met);
    assert!(!goals.terminated_early);
    let ran_b = manifest.events.iter().any(|e| {
        e.kind == EventKind::NodeStarted && e.addr.node_path == NodePath::root().child("b")
    });
    assert!(ran_b, "both entrypoints run when terminate_on_met is false");
}
