//! Sessions — the abstraction behind `session_group` and `resume_session`, and
//! the long-horizon / turn-taking workloads (DungeonGrid). A session is a
//! persistent, checkpointable line of execution bound to one actor and one
//! element of the session collection.
//!
//! The turn loop is expressed in the topology (`session_group` → per-session
//! `while` body: acquire(env-limit) → observe → actor.drive(obs+inbox) → step →
//! checkpoint → release → drain). This module holds the session's runtime
//! identity and checkpoint plumbing; the loop itself is driven by the runner.

use jesterky_contract::NodePath;

/// A running session's identity within a run.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    /// The actor this session drives (from the `session_group` node).
    pub actor: String,
    /// The session's element of the session collection (bound like a map `item`).
    pub item: serde_json::Value,
    /// Root path for this session's events (`session_group[i]`), so its turns
    /// get a private logical-clock lane (ADR #5).
    pub path: NodePath,
    /// Current `while` iteration (turn number). Stamps `Addr::iteration`.
    pub turn: u32,
}

impl Session {
    pub fn new(
        id: impl Into<String>,
        actor: impl Into<String>,
        item: serde_json::Value,
        path: NodePath,
    ) -> Self {
        Self {
            id: id.into(),
            actor: actor.into(),
            item,
            path,
            turn: 0,
        }
    }

    /// Advance to the next turn (increments the iteration used for event Addrs).
    pub fn advance(&mut self) {
        self.turn += 1;
    }
}

// NOTE(impl): checkpoint save/load goes through the `CheckpointStore` host trait
// (traits.rs). `resume_session` loads the latest checkpoint into a `Session`;
// each turn's post-step snapshot is saved and emitted as `CheckpointCreated`.
// Recorded env `observe`/`step` results (CallKind::Resource*) make the whole
// session replayable without a live env (ADR #7).
