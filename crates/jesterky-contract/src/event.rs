//! Event stream — the load-bearing contract (ADR #5). Every event carries an
//! [`Addr`]: its identity AND its position in a deterministic total order.
//!
//! The whole replay guarantee rests on one rule: **event order is a function of
//! graph position, never of emission time.** Parallel map items may reach the
//! sink in any interleaving; sorting by [`Addr`] always yields the same stream.
//! (The deprecated `rust_backend/graph` used an emit-time `INCR` counter and
//! therefore could not replay — see the rebuild handoff §11.)

use serde::{Deserialize, Serialize};

/// One segment of a node path. A path alternates named nodes and numeric indices,
/// e.g. `audit_jobs` → `[3]` for the 4th map item. Kept typed (not a string) so
/// that `Index(2) < Index(10)` orders numerically, not lexically.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathSeg {
    Node(String),
    Index(u32),
}

/// Fully-qualified position of a node in a run's dynamic execution tree.
/// Ordering is lexicographic over segments (derived), which is exactly the
/// structural order we want.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodePath(pub Vec<PathSeg>);

impl NodePath {
    pub fn root() -> Self {
        NodePath(Vec::new())
    }
    /// Descend into a named child (e.g. entering node `audit_jobs`).
    pub fn child(&self, name: impl Into<String>) -> Self {
        let mut segs = self.0.clone();
        segs.push(PathSeg::Node(name.into()));
        NodePath(segs)
    }
    /// Descend into a map/for_each item (e.g. `audit_jobs[3]`).
    pub fn index(&self, i: u32) -> Self {
        let mut segs = self.0.clone();
        segs.push(PathSeg::Index(i));
        NodePath(segs)
    }
}

/// The structural address of an event — its identity and its sort key.
///
/// Field order matters: `derive(Ord)` compares in declaration order, so the
/// effective ordering is `(node_path, iteration, local_seq)` within a run
/// (`run_id` is constant across a run's events). NEVER add an emission counter.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Addr {
    pub run_id: String,
    /// Where in the execution tree this event was produced.
    pub node_path: NodePath,
    /// Loop/`while` iteration at this node (0 for non-looping nodes).
    pub iteration: u32,
    /// Monotonic sequence WITHIN a single `(node_path, iteration)`, allocated
    /// locally by the runner. Two parallel siblings never share a `local_seq`
    /// because they have distinct `node_path`s.
    pub local_seq: u32,
}

/// The closed set of event families. Payload detail rides in [`Event::payload`]
/// as a typed value; this enum is the stable vocabulary consumers switch on.
/// (Taxonomy adapted from `rlm/rlm_event_streaming_plan.md`, retyped — §11.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum EventKind {
    WorkflowStarted,
    NodeStarted,
    NodeCompleted,
    MapItemStarted,
    MapItemCompleted,
    MapItemFailed,
    MapCompleted,
    /// An actor/resource call was recorded (see replay manifest).
    ActorInvoked,
    ResourceInvoked,
    // Sessions (session_group / resume_session).
    SessionStarted,
    SessionResumed,
    CheckpointCreated,
    // Limits / semaphores (the central-serialization joint).
    SemaphoreAcquired,
    SemaphoreReleased,
    // Mailbox (multi-agent coordination).
    MessagePublished,
    MessageAvailable,
    ArtifactEmitted,
    WorkflowCompleted,
    WorkflowFailed,
}

/// One event in the stream. `wall_ms` is metadata ONLY — never part of ordering
/// or identity (that is [`Addr`]'s job).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub addr: Addr,
    pub kind: EventKind,
    pub payload: serde_json::Value,
    pub wall_ms: u64,
}
