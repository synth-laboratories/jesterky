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
use crate::traits::{
    Actor, ActorRequest, ArtifactStore, CheckpointStore, Clock, EventSink, Resource,
};
use async_recursion::async_recursion;
use jesterky_contract::{
    Addr, Bindings, CallKind, Checkpoint, Event, EventKind, Node, NodeKind, NodePath,
    RecordedOutput, RunManifest, RunStatus, WorkflowSpec,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;

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
    map_concurrency: Option<u32>,
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
            map_concurrency: spec.runplan.map_concurrency,
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
    ) -> Addr {
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
            addr: addr.clone(),
            kind,
            payload,
            wall_ms: self.clock.now_ms(),
        };
        self.sink.emit(event.clone());
        ctx.events.lock().unwrap().push(event);
        addr
    }

    fn peek_next_addr(&self, ctx: &RunCtx, path: &NodePath, iteration: u32) -> Addr {
        let local_seq = ctx
            .addr_seqs
            .lock()
            .unwrap()
            .get(&(path.clone(), iteration))
            .copied()
            .unwrap_or(0);
        Addr {
            run_id: ctx.run_id.clone(),
            node_path: path.clone(),
            iteration,
            local_seq,
        }
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
        self.execute_node_with_ledger(ctx, path, node, &ctx.ledger, None)
            .await
            .map(|_| ())
    }

    #[async_recursion]
    async fn execute_node_with_ledger(
        &self,
        ctx: &RunCtx,
        path: NodePath,
        node: &Node,
        ledger: &Mutex<Ledger>,
        input_override: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, CoreError> {
        self.emit(
            ctx,
            &path,
            0,
            EventKind::NodeStarted,
            serde_json::Value::Null,
        );
        let result = match &node.kind {
            NodeKind::Program { op } | NodeKind::Reduce { op } => {
                let inputs = match input_override {
                    Some(inputs) => inputs,
                    None => ledger.lock().unwrap().resolve_bindings(&node.inputs)?,
                };
                let program = self
                    .programs
                    .get(op)
                    .cloned()
                    .ok_or_else(|| CoreError::UnknownProgram(op.clone()))?;
                let result = {
                    let ledger = ledger.lock().unwrap();
                    program(&ledger, &inputs)?
                };
                ledger
                    .lock()
                    .unwrap()
                    .store_outputs(&node.outputs, &result)?;
                result
            }
            NodeKind::Actor { actor } => {
                let inputs = match input_override {
                    Some(inputs) => inputs,
                    None => ledger.lock().unwrap().resolve_bindings(&node.inputs)?,
                };
                let addr = self.peek_next_addr(ctx, &path, 0);
                let actor_result = self
                    .actor
                    .drive(ActorRequest {
                        addr: addr.clone(),
                        actor: actor.clone(),
                        inputs,
                    })
                    .await?;
                ctx.recorded.lock().unwrap().push(RecordedOutput {
                    addr: addr.clone(),
                    call: CallKind::Actor {
                        actor: actor.clone(),
                    },
                    outputs: actor_result.outputs.clone(),
                    score: actor_result.score,
                    signal: actor_result.signal.clone(),
                    artifacts: actor_result.artifacts.clone(),
                });
                ledger
                    .lock()
                    .unwrap()
                    .store_outputs(&node.outputs, &actor_result.outputs)?;
                let emitted_addr = self.emit(
                    ctx,
                    &path,
                    0,
                    EventKind::ActorInvoked,
                    serde_json::json!({ "actor": actor }),
                );
                debug_assert_eq!(emitted_addr, addr);
                actor_result.outputs
            }
            NodeKind::Map { .. } => self.execute_map(ctx, path.clone(), node, ledger).await?,
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
        };
        self.emit(
            ctx,
            &path,
            0,
            EventKind::NodeCompleted,
            serde_json::Value::Null,
        );
        Ok(result)
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
        ctx: &RunCtx,
        path: NodePath,
        node: &Node,
        ledger: &Mutex<Ledger>,
    ) -> Result<serde_json::Value, CoreError> {
        let NodeKind::Map {
            over,
            concurrency,
            min_success,
            body,
            ..
        } = &node.kind
        else {
            unreachable!("execute_map is only called for map nodes");
        };

        let items = {
            let ledger = ledger.lock().unwrap();
            match ledger.resolve(over)? {
                serde_json::Value::Array(items) => items,
                _ => return Err(crate::ledger::LedgerError::TypeMismatch(over.0.clone()).into()),
            }
        };
        let total = items.len();
        let width = (*concurrency).or(ctx.map_concurrency).unwrap_or(1).max(1) as usize;
        let results = if width <= 1 {
            self.execute_map_serial(ctx, &path, body, ledger, &items)
                .await
        } else {
            self.execute_map_parallel(ctx, &path, body, ledger, &items, width)
                .await
        }?;

        let successes = results.iter().filter(|r| r.is_ok()).count();
        let success_ratio = if total == 0 {
            1.0
        } else {
            successes as f64 / total as f64
        };
        if success_ratio + f64::EPSILON < *min_success {
            return Err(CoreError::MapMinSuccess {
                required: *min_success,
                successes,
                total,
            });
        }

        let map_result = collect_map_outputs(&node.outputs, results)?;
        ledger
            .lock()
            .unwrap()
            .store_outputs(&node.outputs, &map_result)?;
        self.emit(
            ctx,
            &path,
            0,
            EventKind::MapCompleted,
            serde_json::json!({
                "successes": successes,
                "total": total,
            }),
        );
        Ok(map_result)
    }

    async fn execute_map_serial(
        &self,
        ctx: &RunCtx,
        path: &NodePath,
        body: &Node,
        ledger: &Mutex<Ledger>,
        items: &[serde_json::Value],
    ) -> Result<Vec<Result<serde_json::Value, String>>, CoreError> {
        let base_ledger = ledger.lock().unwrap().clone();
        let mut results = Vec::with_capacity(items.len());

        for (i, item) in items.iter().cloned().enumerate() {
            let item_path = path.index(i as u32);
            self.emit(
                ctx,
                &item_path,
                0,
                EventKind::MapItemStarted,
                serde_json::json!({ "index": i }),
            );
            let item_ledger = Mutex::new(base_ledger.with_item(item));
            match self
                .execute_node_with_ledger(ctx, item_path.clone(), body, &item_ledger, None)
                .await
            {
                Ok(result) => {
                    self.emit(
                        ctx,
                        &item_path,
                        0,
                        EventKind::MapItemCompleted,
                        serde_json::json!({ "index": i }),
                    );
                    results.push(Ok(result));
                }
                Err(err) => {
                    let message = err.to_string();
                    self.emit(
                        ctx,
                        &item_path,
                        0,
                        EventKind::MapItemFailed,
                        serde_json::json!({ "index": i, "error": message }),
                    );
                    results.push(Err(message));
                }
            }
        }

        Ok(results)
    }

    async fn execute_map_parallel(
        &self,
        ctx: &RunCtx,
        path: &NodePath,
        body: &Node,
        ledger: &Mutex<Ledger>,
        items: &[serde_json::Value],
        width: usize,
    ) -> Result<Vec<Result<serde_json::Value, String>>, CoreError> {
        let base_ledger = ledger.lock().unwrap().clone();
        let mut prepared = Vec::with_capacity(items.len());
        for (i, item) in items.iter().cloned().enumerate() {
            let item_ledger = base_ledger.with_item(item);
            let inputs = item_ledger.resolve_bindings(&body.inputs)?;
            prepared.push((i, item_ledger, inputs));
        }

        let mut results = (0..items.len()).map(|_| None).collect::<Vec<_>>();
        for chunk in prepared.chunks(width) {
            let futures = chunk
                .iter()
                .map(|(i, item_ledger, inputs)| {
                    let index = *i;
                    let item_path = path.index(index as u32);
                    let item_ledger = Arc::new(Mutex::new(item_ledger.clone()));
                    let inputs = inputs.clone();
                    Box::pin(async move {
                        self.emit(
                            ctx,
                            &item_path,
                            0,
                            EventKind::MapItemStarted,
                            serde_json::json!({ "index": index }),
                        );
                        let result = self
                            .execute_node_with_ledger(
                                ctx,
                                item_path.clone(),
                                body,
                                item_ledger.as_ref(),
                                Some(inputs),
                            )
                            .await;
                        match result {
                            Ok(result) => {
                                self.emit(
                                    ctx,
                                    &item_path,
                                    0,
                                    EventKind::MapItemCompleted,
                                    serde_json::json!({ "index": index }),
                                );
                                (index, Ok(result))
                            }
                            Err(err) => {
                                let message = err.to_string();
                                self.emit(
                                    ctx,
                                    &item_path,
                                    0,
                                    EventKind::MapItemFailed,
                                    serde_json::json!({ "index": index, "error": message }),
                                );
                                (index, Err(message))
                            }
                        }
                    })
                        as Pin<
                            Box<
                                dyn Future<Output = (usize, Result<serde_json::Value, String>)>
                                    + Send
                                    + '_,
                            >,
                        >
                })
                .collect::<Vec<_>>();
            for (i, result) in join_all_ordered(futures).await {
                results[i] = Some(result);
            }
        }

        Ok(results
            .into_iter()
            .map(|result| result.expect("map result slot filled"))
            .collect())
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

fn collect_map_outputs(
    outputs: &Bindings,
    results: Vec<Result<serde_json::Value, String>>,
) -> Result<serde_json::Value, CoreError> {
    let mut collected = serde_json::Map::new();
    let single_output = outputs.len() == 1;

    for field in outputs.keys() {
        let mut values = Vec::with_capacity(results.len());
        for result in &results {
            let value = match result {
                Ok(value) if single_output => value
                    .as_object()
                    .and_then(|object| object.get(field))
                    .cloned()
                    .unwrap_or_else(|| value.clone()),
                Ok(value) => value
                    .as_object()
                    .and_then(|object| object.get(field))
                    .cloned()
                    .ok_or_else(|| {
                        crate::ledger::LedgerError::Unresolved(format!("map result.{field}"))
                    })?,
                Err(_) => serde_json::Value::Null,
            };
            values.push(value);
        }
        collected.insert(field.clone(), serde_json::Value::Array(values));
    }

    Ok(serde_json::Value::Object(collected))
}

async fn join_all_ordered<T>(
    mut futures: Vec<Pin<Box<dyn Future<Output = T> + Send + '_>>>,
) -> Vec<T> {
    let mut results = (0..futures.len()).map(|_| None).collect::<Vec<_>>();
    std::future::poll_fn(move |cx| {
        let mut all_done = true;
        for (idx, future) in futures.iter_mut().enumerate() {
            if results[idx].is_some() {
                continue;
            }
            match future.as_mut().poll(cx) {
                Poll::Ready(value) => results[idx] = Some(value),
                Poll::Pending => all_done = false,
            }
        }

        if all_done {
            Poll::Ready(
                results
                    .iter_mut()
                    .map(|result| result.take().expect("completed future has output"))
                    .collect(),
            )
        } else {
            Poll::Pending
        }
    })
    .await
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
    #[error("map min_success gate failed: required {required}, got {successes}/{total}")]
    MapMinSuccess {
        required: f64,
        successes: usize,
        total: usize,
    },
}
