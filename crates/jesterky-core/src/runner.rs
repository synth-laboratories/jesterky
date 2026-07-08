//! The runner — walks a [`WorkflowSpec`], drives nodes, emits the event stream,
//! and produces a [`RunManifest`]. This is the heart of the core.
//!
//! What is LOCKED here (the joints): the seam wiring, the per-node logical-clock
//! allocation in [`Runner::emit`] (ADR #5), the pure-vs-recorded split (programs
//! re-run, actors/resources recorded — ADR #7), and the manifest shape. What is
//! SKELETAL: the per-kind execution bodies and the parallel map dispatch, left as
//! documented `todo!()` for the implementing engineer.

use crate::ledger::Ledger;
use crate::limits::LimitSet;
use crate::mailbox::Mailbox;
use crate::traits::{Actor, ArtifactStore, CheckpointStore, Clock, EventSink, Resource};
use async_recursion::async_recursion;
use jesterky_contract::{
    Addr, Checkpoint, Event, EventKind, Node, NodeKind, NodePath, RecordedOutput, RunManifest,
    RunStatus, WorkflowSpec,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A pure, deterministic program op (ADR #7): `(ledger, inputs) -> outputs`, no
/// IO, re-run verbatim on replay. Registered by the author/host.
pub type ProgramFn =
    Arc<dyn Fn(&Ledger, &serde_json::Value) -> Result<serde_json::Value, CoreError> + Send + Sync>;

#[derive(Default, Clone)]
pub struct ProgramRegistry {
    programs: HashMap<String, ProgramFn>,
}

impl ProgramRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, op: impl Into<String>, f: ProgramFn) {
        self.programs.insert(op.into(), f);
    }
    pub fn get(&self, op: &str) -> Option<&ProgramFn> {
        self.programs.get(op)
    }
}

/// The runner, wired to the host across the seam (ADR #6). One `Actor` impl
/// routes by name; `resource` is present only for workflows with a `Resource`
/// (e.g. DungeonGrid's env).
pub struct Runner {
    pub programs: ProgramRegistry,
    pub actor: Arc<dyn Actor>,
    pub resource: Option<Arc<dyn Resource>>,
    pub sink: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    pub store: Arc<dyn ArtifactStore>,
    /// Present only for workflows with sessions/checkpoints.
    pub checkpoints: Option<Arc<dyn CheckpointStore>>,
}

/// Per-run mutable state, behind interior mutability so parallel map items can
/// emit/record concurrently (ADR #5). The top-level ledger is sequential; map
/// items operate on per-item CLONES (see [`Runner::execute_map`]).
struct RunCtx {
    run_id: String,
    args: serde_json::Value,
    ledger: Mutex<Ledger>,
    /// The logical clock: next `local_seq` per `(node_path, iteration)`.
    addr_seqs: Mutex<HashMap<(NodePath, u32), u32>>,
    events: Mutex<Vec<Event>>,
    recorded: Mutex<Vec<RecordedOutput>>,
    checkpoints: Mutex<Vec<Checkpoint>>,
    /// Named concurrency budgets / the central-serialization semaphores.
    limits: LimitSet,
    /// Inter-session message-passing.
    mailbox: Mailbox,
}

impl RunCtx {
    fn new(run_id: String, args: serde_json::Value, spec: &WorkflowSpec) -> Self {
        Self {
            run_id,
            args,
            ledger: Mutex::new(Ledger::new()),
            addr_seqs: Mutex::new(HashMap::new()),
            events: Mutex::new(Vec::new()),
            recorded: Mutex::new(Vec::new()),
            checkpoints: Mutex::new(Vec::new()),
            limits: LimitSet::from_permits(&spec.runplan.limits),
            mailbox: Mailbox::new(),
        }
    }
}

impl Runner {
    /// Run a workflow. `run_id` is caller-supplied — the core never invents
    /// identity (no clocks/RNG; hosted identity is the host's concern, and
    /// determinism requires it). Returns the full [`RunManifest`].
    pub async fn run(
        &self,
        spec: &WorkflowSpec,
        run_id: String,
        args: serde_json::Value,
    ) -> Result<RunManifest, CoreError> {
        let spec_hash = spec.validate_and_hash()?;
        let ctx = RunCtx::new(run_id, args.clone(), spec);
        self.emit(&ctx, &NodePath::root(), 0, EventKind::WorkflowStarted, args);

        for id in &spec.entrypoint {
            let node = spec
                .nodes
                .get(id)
                .ok_or_else(|| CoreError::UnknownNode(id.clone()))?;
            self.execute_node(&ctx, NodePath::root().child(id.clone()), node)
                .await?;
        }

        self.emit(
            &ctx,
            &NodePath::root(),
            0,
            EventKind::WorkflowCompleted,
            serde_json::Value::Null,
        );
        Ok(self.into_manifest(&ctx, spec, spec_hash))
    }

    /// Allocate an [`Addr`] and emit an event. THIS is the logical-clock joint
    /// (ADR #5): `local_seq` is allocated per `(node_path, iteration)`, so two
    /// parallel map siblings — which have distinct `node_path`s — never collide,
    /// and the stream is deterministically orderable by `Addr` regardless of the
    /// thread interleaving that produced it. NEVER replace this with a global
    /// emit-time counter (the §11 bug).
    fn emit(
        &self,
        ctx: &RunCtx,
        path: &NodePath,
        iteration: u32,
        kind: EventKind,
        payload: serde_json::Value,
    ) {
        let addr = {
            let mut seqs = ctx.addr_seqs.lock().unwrap();
            let next = seqs.entry((path.clone(), iteration)).or_insert(0);
            let local_seq = *next;
            *next += 1;
            Addr {
                run_id: ctx.run_id.clone(),
                node_path: path.clone(),
                iteration,
                local_seq,
            }
        };
        let event = Event {
            addr,
            kind,
            payload,
            wall_ms: self.clock.now_ms(),
        };
        self.sink.emit(event.clone());
        ctx.events.lock().unwrap().push(event);
    }

    /// Dispatch one node by kind. Async-recursive because map/while/session
    /// bodies are themselves nodes.
    #[async_recursion]
    async fn execute_node(
        &self,
        ctx: &RunCtx,
        path: NodePath,
        node: &Node,
    ) -> Result<(), CoreError> {
        self.emit(ctx, &path, 0, EventKind::NodeStarted, serde_json::Value::Null);
        match &node.kind {
            NodeKind::Program { op: _ } | NodeKind::Reduce { op: _ } => {
                // TODO: resolve inputs → look up program → run (pure) → store
                // outputs. NOT recorded (re-run on replay, ADR #7).
                todo!("resolve bindings, run ProgramFn, store outputs")
            }
            NodeKind::Actor { actor: _ } => {
                // TODO: resolve inputs → build ActorRequest{addr,...} → await
                // self.actor.drive → RECORD the ActorResult into ctx.recorded
                // keyed by addr (ADR #7) → store outputs → emit ActorInvoked.
                // Offload oversized outputs via self.store (ADR #2).
                todo!("drive actor, record output for replay, store outputs")
            }
            NodeKind::Map { .. } => self.execute_map(ctx, path.clone(), node).await?,
            NodeKind::ForEach { .. } => todo!("serial iteration with visible side effects"),
            NodeKind::While { .. } => {
                // TODO: loop body while `cond` truthy, bumping the Addr.iteration
                // each pass (that is what keeps loop events distinct/orderable).
                todo!("while-loop, iteration-stamped events")
            }
            NodeKind::Branch { .. } => todo!("resolve cond → run then/otherwise"),
            NodeKind::SessionGroup { .. } => {
                // TODO: spawn one session per element, each running `body` under
                // the `limit` (permits) that serializes shared-resource access —
                // the "poll under a center logic" pattern. Env access goes to
                // self.resource; observe/step results are RECORDED (ADR #7).
                todo!("session group; limit-gated resource turns; record env calls")
            }
            NodeKind::ResumeSession { .. } => todo!("rehydrate checkpoint, run body"),
        }
        self.emit(ctx, &path, 0, EventKind::NodeCompleted, serde_json::Value::Null);
        Ok(())
    }

    /// Fan a map body over its collection. Serial when `concurrency` is 1/None
    /// (byte-identical to a for-loop); parallel otherwise.
    ///
    /// THE JOINT (ADR #5): in the parallel path, resolve EACH item's inputs on
    /// the main thread first (binding `item` on a per-item ledger CLONE), then
    /// dispatch the body calls concurrently with those concrete inputs — so the
    /// threaded work never races on shared `current_item`. Each item runs under
    /// `path.index(i)`, giving it a distinct `node_path` and therefore a private
    /// logical-clock lane. Collect into slots by index → deterministic order.
    /// Apply the `min_success` reduce-gate at the end.
    async fn execute_map(
        &self,
        _ctx: &RunCtx,
        _path: NodePath,
        _node: &Node,
    ) -> Result<(), CoreError> {
        todo!(
            "serial vs parallel by concurrency; per-item path.index(i); \
             pre-resolve inputs on a per-item ledger clone; min_success gate"
        )
    }

    /// Assemble the run record. `trace` (the ProcessNode tree) is built from the
    /// recorded outputs + node structure.
    fn into_manifest(&self, ctx: &RunCtx, spec: &WorkflowSpec, spec_hash: String) -> RunManifest {
        RunManifest {
            run_id: ctx.run_id.clone(),
            workflow_name: spec.name.clone(),
            spec_hash,
            args: ctx.args.clone(),
            events: ctx.events.lock().unwrap().clone(),
            recorded: ctx.recorded.lock().unwrap().clone(),
            checkpoints: ctx.checkpoints.lock().unwrap().clone(),
            // TODO: fold `recorded` + node structure into the ProcessNode tree
            // (ADR #2, the optimizer-facing artifact).
            trace: None,
            status: RunStatus::Completed,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("unknown node: {0}")]
    UnknownNode(String),
    #[error(transparent)]
    Contract(#[from] jesterky_contract::ContractError),
    #[error(transparent)]
    Ledger(#[from] crate::ledger::LedgerError),
    #[error(transparent)]
    Host(#[from] crate::traits::HostError),
    #[error("program not registered: {0}")]
    UnknownProgram(String),
}
