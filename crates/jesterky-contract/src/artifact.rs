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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// e.g. `blob/ab12cd`. Opaque to the core; meaningful to the store.
    pub key: String,
    pub size_bytes: u64,
    pub content_type: String,
}

/// Either an inline small value or a reference to an offloaded one. The runner
/// applies the inline/offload split at a size cap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum Artifact {
    Inline(serde_json::Value),
    Ref(ArtifactRef),
}

/// One node's contribution to the process tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "call")]
pub enum CallKind {
    Actor { actor: String },
    ResourceObserve { session: String },
    ResourceStep { session: String },
}

/// What an actor/resource returned, recorded so replay can re-drive without the
/// live model/env (ADR #7). Keyed by [`Addr`] via the `addr` field (kept in a
/// `Vec` rather than a map so the manifest serializes to plain JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordedOutput {
    pub addr: Addr,
    pub call: CallKind,
    pub outputs: serde_json::Value,
    pub score: Option<f64>,
    pub signal: Option<serde_json::Value>,
    pub artifacts: Vec<ArtifactRef>,
}

/// A per-session state snapshot (DungeonGrid takes one per turn). `state` may be
/// offloaded to the checkpoint store when large. A `resume_session` node
/// rehydrates from the latest checkpoint for its session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub session: String,
    pub addr: Addr,
    pub state: Artifact,
}

/// A mailbox message (multi-agent coordination). Orchestration-level, not IO —
/// the core routes it between sessions (see the mailbox module).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub topic: String,
    pub sender: String,
    pub body: serde_json::Value,
    pub recipients: Vec<String>,
}

/// The full record of a run: enough to replay it and to hand an optimizer a
/// structured process object. Produced by the runner; consumed by the CLI/Stack
/// visualizer, the replay engine, and the optimizers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Completed,
    Failed,
}
