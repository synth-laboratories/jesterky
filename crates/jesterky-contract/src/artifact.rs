//! Artifacts, the process-tree trace, and the run manifest (ADR #2 + #7).
//!
//! Two design commitments live here:
//!  * **Artifacts are referenced, not inlined.** A large output goes to the
//!    host `ArtifactStore` and travels as an [`ArtifactRef`] (offload pattern
//!    from `blob.rs`/`trace_upload.rs`, §11).
//!  * **The trace is a typed PROCESS TREE, not flat logs** — per node: typed
//!    inputs/outputs, a `score` slot and a `signal` slot, and outcome-artifact
//!    refs. This is what lets GEPA/GELO walk a run as a process object. The
//!    Chinese wall: these are *outcomes*, never a code judgment of actor
//!    quality (house rule).

use crate::event::{Addr, Event};
use serde::{Deserialize, Serialize};

/// A stable reference to an artifact held in the host `ArtifactStore`.
#[derive(Clone, Debug, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// e.g. `blob/ab12cd`. Opaque to the core; meaningful to the store.
    pub key: String,
    pub size_bytes: u64,
    pub content_type: String,
}

/// Either an inline small value or a reference to an offloaded one. The runner
/// applies the inline/offload split at a size cap.
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum Artifact {
    Inline(serde_json::Value),
    Ref(ArtifactRef),
}

/// One node's contribution to the process tree.
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct ProcessNode {
    pub addr: Addr,
    /// e.g. "map:audit_jobs", "actor:quality_auditor".
    pub label: String,
    /// Typed inputs the node received (post-binding-resolution).
    pub inputs: serde_json::Value,
    /// Typed outputs the node produced.
    pub outputs: serde_json::Value,
    /// Optimizer slot: an outcome score, if the actor supplied one. Grades
    /// OUTCOMES only.
    pub score: Option<f64>,
    /// Optimizer slot: a wall-safe verifier signal, if any.
    pub signal: Option<serde_json::Value>,
    /// Large outputs offloaded to the store.
    pub artifacts: Vec<ArtifactRef>,
    pub children: Vec<ProcessNode>,
}

/// Which impure call produced a [`RecordedOutput`]. Lets a single recorded
/// stream serve both [`crate`]-level actor replay AND resource (env) replay:
/// the `ReplayActor` matches `Actor` entries, the `ReplayResource` matches
/// `ResourceObserve`/`ResourceStep` entries, both keyed by [`Addr`].
#[derive(Clone, Debug, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "call")]
pub enum CallKind {
    Actor { actor: String },
    ResourceObserve { session: String },
    ResourceStep { session: String },
}

/// What an actor/resource returned, recorded so replay can re-drive without the
/// live model/env (ADR #7). Keyed by [`Addr`] via the `addr` field (kept in a
/// `Vec` rather than a map so the manifest serializes to plain JSON).
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct RecordedOutput {
    pub addr: Addr,
    pub call: CallKind,
    pub outputs: serde_json::Value,
    pub score: Option<f64>,
    pub signal: Option<serde_json::Value>,
    pub artifacts: Vec<ArtifactRef>,
    /// Evidence-quality verdict for this output, SEPARATE from run lifecycle
    /// (see [`crate::grounding`]). Missing on legacy rows deserializes to
    /// [`crate::grounding::Grounding::Ungraded`] — never to grounded.
    #[serde(default)]
    pub grounding: crate::grounding::Grounding,
}

/// A per-session state snapshot (DungeonGrid takes one per turn). `state` may be
/// offloaded to the checkpoint store when large. A `resume_session` node
/// rehydrates from the latest checkpoint for its session.
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct Checkpoint {
    pub session: String,
    pub addr: Addr,
    pub state: Artifact,
}

/// A mailbox message (multi-agent coordination). Reserved contract vocabulary;
/// no core node kind routes these messages yet.
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct Message {
    pub topic: String,
    pub sender: String,
    pub body: serde_json::Value,
    pub recipients: Vec<String>,
}

/// The full record of a run: enough to replay it and to hand an optimizer a
/// structured process object. Produced by the runner; consumed by the CLI/Stack
/// visualizer, the replay engine, and the optimizers.
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub workflow_name: String,
    /// The canonical graph hash this run was produced from (ADR #5). A replay
    /// asserts it re-drives the SAME topology.
    pub spec_hash: String,
    pub args: serde_json::Value,
    /// The event stream, in emission order. Sort by `event.addr` for the
    /// canonical order (replay compares against that).
    pub events: Vec<Event>,
    /// Recorded impure outputs, one per actor/resource invocation.
    pub recorded: Vec<RecordedOutput>,
    /// Session state snapshots taken during the run.
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    /// The process tree for optimizer consumption.
    pub trace: Option<ProcessNode>,
    pub status: RunStatus,
    /// Machine-readable terminal reason; consumers should not parse
    /// `WorkflowFailed.payload.error`.
    #[serde(default)]
    pub stop_reason: RunStopReason,
    /// Formal resource-budget projection (progress + ETA) when the run declared
    /// [`crate::topology::RunPlan::budgets`]. Host-metered; optional for back-compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budgets: Option<crate::budget::BudgetSnapshot>,
    /// Formal goal / work-product projection when the run declared
    /// [`crate::topology::RunPlan::goals`]. Evaluated from the final ledger;
    /// optional for back-compat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goals: Option<crate::goal::GoalSnapshot>,
    /// Structural self-checks over this manifest (event-addr identity, no orphaned
    /// records, per-map fan/gate consistency). Attached when a trace exists.
    /// A failing check indicates a runner/manifest bug, not a workload failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariants: Option<InvariantReport>,
    /// Annotation-grounding projection when the run declared
    /// [`crate::topology::RunPlan::grounding`]. SEPARATE from [`Self::status`]:
    /// a `completed` run may still report `required_trace_unread` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grounding: Option<crate::grounding::GroundingReport>,
}

/// One structural invariant checked over a finished [`RunManifest`].
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct InvariantCheck {
    /// Stable check id (e.g. `unique_event_addrs`, `map:scan_jobs.over_matches_total`).
    pub name: String,
    pub ok: bool,
    /// Human-readable evidence (counts, the offending addr, …).
    pub detail: String,
}

/// The manifest self-check report. `all_ok` is the AND of every check.
#[derive(Clone, Debug, PartialEq, schemars::JsonSchema, Serialize, Deserialize)]
pub struct InvariantReport {
    pub all_ok: bool,
    pub checks: Vec<InvariantCheck>,
}

impl InvariantReport {
    /// Compute the structural invariants from a finished manifest — a PURE
    /// function of `events` + `recorded` + `trace`. Every `map`/`for_each` fan
    /// node (identified by a recorded `{successes,total}` signal) is checked for
    /// fan/gate consistency; global checks cover event-addr identity (ADR #5) and
    /// record⊆trace coverage.
    pub fn compute(manifest: &RunManifest) -> InvariantReport {
        let mut checks = Vec::new();

        // ADR #5: every event Addr is distinct (guards the logical-clock joint).
        {
            let mut seen = std::collections::HashSet::new();
            let dup = manifest.events.iter().find(|e| !seen.insert(&e.addr));
            checks.push(InvariantCheck {
                name: "unique_event_addrs".into(),
                ok: dup.is_none(),
                detail: match dup {
                    Some(e) => format!("duplicate event addr {:?}", e.addr),
                    None => format!("{} events, all addrs distinct", manifest.events.len()),
                },
            });
        }

        if let Some(trace) = &manifest.trace {
            // Every recorded impure output is represented as a trace leaf.
            let mut leaf_addrs = std::collections::HashSet::new();
            collect_leaf_addrs(trace, &mut leaf_addrs);
            let orphans = manifest
                .recorded
                .iter()
                .filter(|r| !leaf_addrs.contains(&r.addr))
                .count();
            checks.push(InvariantCheck {
                name: "no_orphaned_records".into(),
                ok: orphans == 0,
                detail: if orphans == 0 {
                    format!(
                        "{} recorded outputs all present in trace",
                        manifest.recorded.len()
                    )
                } else {
                    format!("{orphans} recorded outputs missing from the trace")
                },
            });

            check_fan_nodes(trace, &mut checks);
        }

        let all_ok = checks.iter().all(|c| c.ok);
        InvariantReport { all_ok, checks }
    }
}

fn collect_leaf_addrs<'a>(node: &'a ProcessNode, out: &mut std::collections::HashSet<&'a Addr>) {
    if node.children.is_empty() {
        out.insert(&node.addr);
    }
    for child in &node.children {
        collect_leaf_addrs(child, out);
    }
}

/// Check every fan node (a `map`/`for_each` that recorded a `{successes,total}`
/// signal): the fanned-over collection length matches `total`, the success gate
/// held, and the item children don't exceed the attempted total.
fn check_fan_nodes(node: &ProcessNode, checks: &mut Vec<InvariantCheck>) {
    let signal = node.signal.as_ref().and_then(|s| s.as_object());
    let total = signal.and_then(|s| s.get("total")).and_then(|v| v.as_u64());
    let successes = signal
        .and_then(|s| s.get("successes"))
        .and_then(|v| v.as_u64());
    if let (Some(total), Some(successes)) = (total, successes) {
        let inputs = node.inputs.as_object();
        let over_len = inputs
            .and_then(|i| i.get("over"))
            .and_then(|v| v.as_array())
            .map(|a| a.len() as u64);
        let min_success = inputs
            .and_then(|i| i.get("min_success"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // A cancelled map (met goal + cancel_in_flight) is deliberately partial:
        // it fans over fewer items and is exempt from the success gate.
        let cancelled = signal
            .and_then(|s| s.get("cancelled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let label = &node.label;

        if let Some(over_len) = over_len {
            checks.push(InvariantCheck {
                name: format!("map:{label}.over_matches_total"),
                ok: over_len == total,
                detail: format!("fanned over {over_len} items, recorded total {total}"),
            });
        }
        let ratio = if total == 0 {
            1.0
        } else {
            successes as f64 / total as f64
        };
        checks.push(InvariantCheck {
            name: format!("map:{label}.min_success_honored"),
            ok: successes <= total && (cancelled || ratio + f64::EPSILON >= min_success),
            detail: format!(
                "{successes}/{total} succeeded (>= min_success {min_success}){}",
                if cancelled { ", cancelled" } else { "" }
            ),
        });
        let child_items = node
            .children
            .iter()
            .filter(|c| c.label.starts_with('['))
            .count() as u64;
        checks.push(InvariantCheck {
            name: format!("map:{label}.children_within_total"),
            ok: child_items <= total,
            detail: format!("{child_items} item children <= {total} attempted"),
        });
    }
    for child in &node.children {
        check_fan_nodes(child, checks);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Failed,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, schemars::JsonSchema, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RunStopReason {
    #[default]
    Completed,
    NodeFailed,
    GoalUnmet,
    BudgetExhausted,
}
