//! The core/host seam (ADR #6). These five traits are the ENTIRE boundary
//! between the OSS orchestration core and a host (Stack local, Synth Cloud).
//! The core depends only on these; it contains no model, HTTP, subprocess, or
//! clock code. The proof this line is right: the same seam serves both live
//! execution and replay — you swap the [`Actor`]/[`Resource`] impl and nothing
//! else changes (§11 shows the deprecated service already validated this shape).
//!
//! CRITICAL: none of these types carry model-shaped fields (no `temperature`,
//! `tools`, `thread_id`). An actor maps inputs → outputs; whether it is a model
//! call or a subprocess is the host's private concern, living in the host's own
//! request type, never in this contract.

use async_trait::async_trait;
use jesterky_contract::{Addr, ArtifactRef, Event};

/// What the core hands an actor. `addr` is included so a recording/replaying
/// host can key by execution position (ADR #7).
#[derive(Clone, Debug)]
pub struct ActorRequest {
    pub addr: Addr,
    /// The registered actor name from the topology (e.g. `quality_auditor`).
    pub actor: String,
    /// Resolved, typed inputs for this invocation.
    pub inputs: serde_json::Value,
}

/// What an actor returns. `score`/`signal` are the optimizer slots (ADR #2);
/// `artifacts` are already-offloaded large outputs.
#[derive(Clone, Debug)]
pub struct ActorResult {
    pub outputs: serde_json::Value,
    pub score: Option<f64>,
    pub signal: Option<serde_json::Value>,
    pub artifacts: Vec<ArtifactRef>,
}

/// A stateless-per-call unit of impure work: a model, a subprocess. One `Actor`
/// impl typically ROUTES by `req.actor` to the right backend — the core never
/// hard-codes a model. Recorded for replay.
#[async_trait]
pub trait Actor: Send + Sync {
    async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError>;
}

/// A stateful, long-lived EXTERNAL system that is polled — the second kind of
/// host dependency (the DungeonGrid env). Unlike an [`Actor`], the core holds a
/// handle across many turns and calls `observe`/`step` under a `Limit` that
/// serializes access ("poll under a center logic"). Also recorded for replay.
#[async_trait]
pub trait Resource: Send + Sync {
    /// Poll the current observation for a session (whose turn, done, state).
    async fn observe(&self, session: &str) -> Result<serde_json::Value, HostError>;
    /// Submit an action; get back reward/next-state/done.
    async fn step(&self, session: &str, action: serde_json::Value)
        -> Result<serde_json::Value, HostError>;
}

/// Where the event stream goes (in-memory, file, redis). Sync + infallible by
/// design: emission must never block or fail orchestration; a slow/lossy sink is
/// the host's problem to buffer.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

/// Wall-clock, injected so the core never calls the OS clock directly — a
/// prerequisite for deterministic replay (a `ReplayClock` returns recorded
/// timestamps). Timestamps are metadata only (ADR #5).
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Blob storage for offloaded artifacts (ADR #2, §11). Async because it is IO.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, bytes: Vec<u8>, content_type: &str) -> Result<ArtifactRef, HostError>;
    async fn get(&self, r: &ArtifactRef) -> Result<Vec<u8>, HostError>;
}

/// Persistence for session checkpoints (ADR #7). Checkpoints can be large, so
/// they live host-side like artifacts; the core writes the latest per session
/// and `resume_session` reads it back.
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn save(&self, session: &str, state: serde_json::Value) -> Result<ArtifactRef, HostError>;
    async fn load(&self, session: &str) -> Result<Option<serde_json::Value>, HostError>;
}

/// Errors from across the seam. The core classifies but does not interpret host
/// failures; it surfaces the class (mirrors the house rule on informative errors).
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("actor {actor} failed: {message}")]
    Actor { actor: String, message: String },
    #[error("resource error: {0}")]
    Resource(String),
    #[error("artifact store error: {0}")]
    Store(String),
    #[error("not found in replay manifest: {0:?}")]
    ReplayMiss(Addr),
}
