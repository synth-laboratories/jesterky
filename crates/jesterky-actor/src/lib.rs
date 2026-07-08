//! `jesterky-actor` — the host-side SDK: reference implementations of the seam
//! traits plus test doubles. Real hosts (Stack, Cloud) implement [`Actor`] /
//! [`Resource`] with actual model/env backends; this crate gives you in-memory
//! and replay implementations for tests, the CLI, and M1's fake-actor demo.
//!
//! The centerpiece is [`ReplayActor`]: it implements the SAME [`Actor`] trait as
//! a live model host, but answers from a recorded [`RunManifest`]. That one type
//! is the proof that the seam (ADR #6) supports both live execution and replay
//! (ADR #7) with zero change to the core.

use async_trait::async_trait;
use jesterky_contract::{Addr, ArtifactRef, CallKind, RunManifest};
use jesterky_core::{
    Actor, ActorRequest, ActorResult, ArtifactStore, CheckpointStore, Clock, EventSink, HostError,
    Resource,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub mod viz;

/// Replays recorded actor outputs by [`Addr`] (ADR #7). Live orchestration +
/// this actor ⇒ a byte-identical event stream, which is the core acceptance
/// invariant ("replay fidelity").
pub struct ReplayActor {
    by_addr: HashMap<Addr, ActorResult>,
}

impl ReplayActor {
    pub fn from_manifest(m: &RunManifest) -> Self {
        let by_addr = m
            .recorded
            .iter()
            .filter(|r| matches!(r.call, CallKind::Actor { .. }))
            .map(|r| {
                (
                    r.addr.clone(),
                    ActorResult {
                        outputs: r.outputs.clone(),
                        score: r.score,
                        signal: r.signal.clone(),
                        artifacts: r.artifacts.clone(),
                    },
                )
            })
            .collect();
        Self { by_addr }
    }
}

#[async_trait]
impl Actor for ReplayActor {
    async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError> {
        self.by_addr
            .get(&req.addr)
            .cloned()
            .ok_or(HostError::ReplayMiss(req.addr))
    }
}

/// Replays recorded env `observe`/`step` results by [`Addr`] — the resource
/// counterpart to [`ReplayActor`], so a whole stateful workload (DungeonGrid)
/// replays without a live env. `observe` and `step` share the recorded stream;
/// the [`Addr`] disambiguates which call this is.
pub struct ReplayResource {
    by_addr: HashMap<Addr, serde_json::Value>,
}

impl ReplayResource {
    pub fn from_manifest(m: &RunManifest) -> Self {
        let by_addr = m
            .recorded
            .iter()
            .filter(|r| {
                matches!(
                    r.call,
                    CallKind::ResourceObserve { .. } | CallKind::ResourceStep { .. }
                )
            })
            .map(|r| (r.addr.clone(), r.outputs.clone()))
            .collect();
        Self { by_addr }
    }

    fn lookup(&self, addr: &Addr) -> Result<serde_json::Value, HostError> {
        self.by_addr
            .get(addr)
            .cloned()
            .ok_or_else(|| HostError::ReplayMiss(addr.clone()))
    }
}

#[async_trait]
impl Resource for ReplayResource {
    async fn observe(&self, addr: &Addr, _session: &str) -> Result<serde_json::Value, HostError> {
        self.lookup(addr)
    }
    async fn step(&self, addr: &Addr, _session: &str, _action: serde_json::Value) -> Result<serde_json::Value, HostError> {
        self.lookup(addr)
    }
}

/// Deterministic replay clock: a monotonic counter (timestamps are metadata
/// only — ADR #5 — so replay need not reproduce wall times).
#[derive(Default)]
pub struct ReplayClock {
    tick: AtomicU64,
}

impl Clock for ReplayClock {
    fn now_ms(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::Relaxed)
    }
}

/// In-memory checkpoint store for tests.
#[derive(Default)]
pub struct MemCheckpointStore {
    latest: Mutex<HashMap<String, serde_json::Value>>,
}

impl MemCheckpointStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CheckpointStore for MemCheckpointStore {
    async fn save(&self, session: &str, state: serde_json::Value) -> Result<ArtifactRef, HostError> {
        let mut latest = self.latest.lock().unwrap();
        let size_bytes = state.to_string().len() as u64;
        latest.insert(session.to_string(), state);
        Ok(ArtifactRef {
            key: format!("ckpt/{session}"),
            size_bytes,
            content_type: "application/json".to_string(),
        })
    }
    async fn load(&self, session: &str) -> Result<Option<serde_json::Value>, HostError> {
        Ok(self.latest.lock().unwrap().get(session).cloned())
    }
}

/// A trivial actor for tests / the M1 fake-actor demo: echoes inputs as outputs.
/// Swap this for a real model host to go from "runs the topology" to "runs the
/// real workload" (M1 → M2).
pub struct FakeActor;

#[async_trait]
impl Actor for FakeActor {
    async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError> {
        Ok(ActorResult {
            outputs: req.inputs,
            score: None,
            signal: None,
            artifacts: Vec::new(),
        })
    }
}

/// In-memory event sink for tests and the CLI. A real host forwards to
/// file/redis and buffers.
#[derive(Default)]
pub struct MemEventSink {
    events: Mutex<Vec<jesterky_contract::Event>>,
}

impl MemEventSink {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn drain(&self) -> Vec<jesterky_contract::Event> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

impl EventSink for MemEventSink {
    fn emit(&self, event: jesterky_contract::Event) {
        self.events.lock().unwrap().push(event);
    }
}

/// System wall-clock (host-side — the core never calls this directly). A
/// `ReplayClock` returning recorded timestamps is the replay counterpart.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// In-memory artifact store for tests.
#[derive(Default)]
pub struct MemArtifactStore {
    blobs: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ArtifactStore for MemArtifactStore {
    async fn put(&self, bytes: Vec<u8>, content_type: &str) -> Result<ArtifactRef, HostError> {
        // Deterministic key from length + a running index keeps tests stable
        // without a clock/RNG. A real store content-hashes.
        let mut blobs = self.blobs.lock().unwrap();
        let key = format!("blob/{}", blobs.len());
        let size_bytes = bytes.len() as u64;
        blobs.insert(key.clone(), bytes);
        Ok(ArtifactRef {
            key,
            size_bytes,
            content_type: content_type.to_string(),
        })
    }

    async fn get(&self, r: &ArtifactRef) -> Result<Vec<u8>, HostError> {
        self.blobs
            .lock()
            .unwrap()
            .get(&r.key)
            .cloned()
            .ok_or_else(|| HostError::Store(format!("missing {}", r.key)))
    }
}
