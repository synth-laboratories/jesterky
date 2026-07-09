//! The runner — walks a [`WorkflowSpec`], drives nodes, emits the event stream,
//! and produces a [`RunManifest`]. This is the heart of the core.
//!
//! The joints (LOCKED): the seam wiring, the per-node logical-clock allocation in
//! [`Runner::emit`] (ADR #5), the pure-vs-recorded split (programs re-run,
//! actors/resources recorded — ADR #7), and the manifest shape. Every node kind
//! is implemented — program/reduce/actor, parallel map, for_each, while, branch,
//! session_group (limit-serialized "poll under a center logic"), and
//! resume_session — plus the ProcessNode trace tree in [`Runner::build_trace`].

use crate::ledger::Ledger;
use crate::limits::LimitSet;
use crate::traits::{
    Actor, ActorRequest, ArtifactStore, CheckpointStore, Clock, EventSink, Resource,
};
use async_recursion::async_recursion;
use jesterky_contract::{
    Addr, Bindings, CallKind, Checkpoint, Event, EventKind, GoalEngine, GoalPlan, GoalSnapshot,
    GoalState, Node, NodeKind, NodePath, PathSeg, ProcessNode, RecordedOutput, RunManifest,
    RunStatus, RunStopReason, WorkflowSpec,
};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

const EVENT_INPUT_INLINE_LIMIT_BYTES: usize = 64 * 1024;

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
    nodes: BTreeMap<String, Node>,
    ledger: Mutex<Ledger>,
    /// The logical clock: next `local_seq` per `(node_path, iteration)`.
    addr_seqs: Mutex<HashMap<(NodePath, u32), u32>>,
    recorded: Mutex<Vec<RecordedOutput>>,
    node_io: Mutex<HashMap<(NodePath, u32), NodeIo>>,
    checkpoints: Mutex<Vec<Checkpoint>>,
    /// Named concurrency budgets / the central-serialization semaphores. `Arc`
    /// so a `LimitGuard` can outlive the acquiring call and release on drop.
    limits: Arc<LimitSet>,
    map_concurrency: Option<u32>,
    /// Set when a met goal with `cancel_in_flight` asks the runner to stop fanning
    /// out pending `map`/`for_each` items. Interior mutability so the between-node
    /// goal check (holding `&RunCtx`) can trip it; the map dispatch loops read it.
    cancel: AtomicBool,
}

#[derive(Clone, Debug)]
struct NodeIo {
    inputs: serde_json::Value,
    outputs: serde_json::Value,
    score: Option<f64>,
    signal: Option<serde_json::Value>,
}

#[derive(Clone, Copy)]
struct MapDispatch {
    width: usize,
    iteration: u32,
}

impl RunCtx {
    fn new(run_id: String, args: serde_json::Value, spec: &WorkflowSpec) -> Self {
        // Seed the ledger with the run args' top-level fields so a spec can
        // parameterize itself via `ledger.<argkey>` bindings (e.g. a scan's
        // `target`). Args are also carried on `WorkflowStarted`; this makes them
        // addressable state, not just an event payload. Author opt-in: nothing
        // reads these keys unless a binding names them.
        let mut ledger = Ledger::new();
        if let Some(fields) = args.as_object() {
            for (key, value) in fields {
                ledger.set(key, value.clone());
            }
        }
        Self {
            run_id,
            args,
            nodes: spec.nodes.clone(),
            ledger: Mutex::new(ledger),
            addr_seqs: Mutex::new(HashMap::new()),
            recorded: Mutex::new(Vec::new()),
            node_io: Mutex::new(HashMap::new()),
            checkpoints: Mutex::new(Vec::new()),
            limits: Arc::new(LimitSet::from_permits(&spec.runplan.limits)),
            map_concurrency: spec.runplan.map_concurrency,
            cancel: AtomicBool::new(false),
        }
    }

    fn record_node_io(
        &self,
        path: NodePath,
        iteration: u32,
        inputs: serde_json::Value,
        outputs: serde_json::Value,
        score: Option<f64>,
        signal: Option<serde_json::Value>,
    ) {
        self.node_io.lock().unwrap().insert(
            (path, iteration),
            NodeIo {
                inputs,
                outputs,
                score,
                signal,
            },
        );
    }
}

/// Resolve the effective goal plan: the spec's `runplan.goals` deep-merged with
/// a run-time `args.goals` overlay (partial overrides allowed — see
/// [`GoalPlan::overlay_json`]). Absent/`null` overlay leaves the plan unchanged.
fn resolve_goal_plan(base: &GoalPlan, args: &serde_json::Value) -> GoalPlan {
    match args.get("goals") {
        Some(overlay) if !overlay.is_null() => base.overlay_json(overlay),
        _ => base.clone(),
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

        // A node failure (a min_success gate, an unresolved binding, an actor that
        // ran out of retries) STOPS the pipeline — but the run still produced
        // events + recorded outputs, so we finalize a `Failed` manifest instead of
        // discarding the work. The failure reason rides a `WorkflowFailed` event so
        // the host can surface it; `run()` returns `Err` only for a malformed spec
        // (before anything ran). Replay/callers key resilience off `manifest.status`.
        // Goals (semantic termination targets) are the dual of budgets, but —
        // unlike budgets, which need host metering of tokens/wall — they are pure
        // predicates over the ledger the core already owns, so the runner
        // evaluates them. `--args.goals` deep-merges onto the declared plan.
        let goal_plan = resolve_goal_plan(&spec.runplan.goals, &ctx.args);

        let mut failure: Option<String> = None;
        let mut goal_terminated = false;
        for id in &spec.entrypoint {
            let node = match spec.nodes.get(id) {
                Some(node) => node,
                None => {
                    failure = Some(format!("unknown node: {id}"));
                    break;
                }
            };
            if let Err(err) = self
                .execute_node(&ctx, NodePath::root().child(id.clone()), node)
                .await
            {
                failure = Some(err.to_string());
                break;
            }
            // Success wrap-up, evaluated once every required goal is met:
            //  * `terminate_on_met` skips the remaining entrypoints entirely;
            //  * otherwise `cancel_in_flight` keeps running the graph but trips the
            //    cancel flag so later `map`/`for_each` nodes stop fanning out
            //    pending items (finish the run cheaply). See `docs/GOALS.md`.
            if !goal_plan.is_empty() {
                let ledger = ctx.ledger.lock().unwrap().snapshot_json();
                let snapshot = GoalEngine::snapshot(&ctx.run_id, &goal_plan, &ledger);
                if snapshot.all_required_met() {
                    if goal_plan.terminate_on_met {
                        goal_terminated = true;
                        break;
                    } else if goal_plan.cancel_in_flight {
                        ctx.cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        }

        if failure.is_none() && goal_terminated {
            if let Some(finalize) = &goal_plan.finalize {
                match spec.nodes.get(finalize) {
                    Some(node) => {
                        if let Err(err) = self
                            .execute_node(&ctx, NodePath::root().child(finalize.clone()), node)
                            .await
                        {
                            failure = Some(err.to_string());
                        }
                    }
                    None => {
                        failure = Some(format!("unknown finalize node: {finalize}"));
                    }
                }
            }
        }

        // Final goal projection from the settled ledger (attached to the manifest).
        let goal_snapshot: Option<GoalSnapshot> = if goal_plan.is_empty() {
            None
        } else {
            let ledger = ctx.ledger.lock().unwrap().snapshot_json();
            let mut snap = GoalEngine::snapshot(&ctx.run_id, &goal_plan, &ledger);
            snap.terminated_early = goal_terminated;
            Some(snap)
        };
        // A required goal still unmet at run end fails the run (dual of budget
        // hard-exhaust). Node failures take precedence over goal failures.
        let goal_failure = if failure.is_none() {
            goal_snapshot.as_ref().filter(|s| s.should_fail()).map(|s| {
                let unmet: Vec<&str> = s
                    .items
                    .iter()
                    .filter(|i| i.required && i.state != GoalState::Met)
                    .map(|i| i.id.as_str())
                    .collect();
                format!("goal_unmet: {}", unmet.join(","))
            })
        } else {
            None
        };

        let (status, stop_reason) = if let Some(reason) = failure.as_ref().or(goal_failure.as_ref())
        {
            self.emit(
                &ctx,
                &NodePath::root(),
                0,
                EventKind::WorkflowFailed,
                serde_json::json!({ "error": reason }),
            );
            let stop_reason = if goal_failure.is_some() && failure.is_none() {
                RunStopReason::GoalUnmet
            } else {
                RunStopReason::NodeFailed
            };
            (RunStatus::Failed, stop_reason)
        } else {
            self.emit(
                &ctx,
                &NodePath::root(),
                0,
                EventKind::WorkflowCompleted,
                serde_json::Value::Null,
            );
            (RunStatus::Completed, RunStopReason::Completed)
        };
        let mut manifest = self.build_manifest(&ctx, spec, spec_hash, status, stop_reason);
        manifest.goals = goal_snapshot;
        // Structural self-check of the manifest we just produced (event-addr
        // identity, no orphaned records, per-map fan/gate consistency). Pure
        // function of the manifest; a failing check is a runner bug, not a
        // workload failure — so it does not change `status`.
        manifest.invariants = Some(jesterky_contract::InvariantReport::compute(&manifest));
        Ok(manifest)
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
        self.execute_node_with_ledger(ctx, path, node, &ctx.ledger, None, 0)
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
        iteration: u32,
    ) -> Result<serde_json::Value, CoreError> {
        self.emit(
            ctx,
            &path,
            iteration,
            EventKind::NodeStarted,
            serde_json::Value::Null,
        );
        let result = match &node.kind {
            NodeKind::Program { op } | NodeKind::Reduce { op } => {
                let inputs = match input_override {
                    Some(inputs) => inputs,
                    None => ledger
                        .lock()
                        .unwrap()
                        .resolve_bindings(&node.inputs)
                        .map_err(|e| at_node(e, &path))?,
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
                let (score, signal) = match &node.kind {
                    NodeKind::Reduce { .. } => reduce_outcome(&result),
                    _ => (None, None),
                };
                ctx.record_node_io(
                    path.clone(),
                    iteration,
                    inputs,
                    result.clone(),
                    score,
                    signal,
                );
                result
            }
            NodeKind::Actor { actor } => {
                let inputs = match input_override {
                    Some(inputs) => inputs,
                    None => ledger
                        .lock()
                        .unwrap()
                        .resolve_bindings(&node.inputs)
                        .map_err(|e| at_node(e, &path))?,
                };
                let addr = self.peek_next_addr(ctx, &path, iteration);
                let event_inputs = self.event_inputs_payload(inputs.clone()).await?;
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
                ctx.record_node_io(
                    path.clone(),
                    iteration,
                    event_inputs.clone(),
                    actor_result.outputs.clone(),
                    actor_result.score,
                    actor_result.signal.clone(),
                );
                let emitted_addr = self.emit(
                    ctx,
                    &path,
                    iteration,
                    EventKind::ActorInvoked,
                    serde_json::json!({ "actor": actor, "inputs": event_inputs }),
                );
                debug_assert_eq!(emitted_addr, addr);
                actor_result.outputs
            }
            NodeKind::Map { .. } => {
                self.execute_map(ctx, path.clone(), node, ledger, iteration)
                    .await?
            }
            NodeKind::ForEach { .. } => {
                self.execute_for_each(ctx, path.clone(), node, ledger, iteration)
                    .await?
            }
            NodeKind::While {
                cond,
                body,
                max_iters,
            } => {
                let mut last = serde_json::Value::Null;
                for pass in 0..*max_iters {
                    let cond_value = ledger.lock().unwrap().resolve(cond)?;
                    if !is_truthy(&cond_value) {
                        break;
                    }
                    last = self
                        .execute_node_with_ledger(ctx, path.child("body"), body, ledger, None, pass)
                        .await?;
                }
                last
            }
            NodeKind::Branch {
                cond,
                then,
                otherwise,
            } => {
                let cond_value = ledger.lock().unwrap().resolve(cond)?;
                let target = if is_truthy(&cond_value) {
                    Some(then)
                } else {
                    otherwise.as_ref()
                };
                match target {
                    Some(target_id) => {
                        let target_node = ctx
                            .nodes
                            .get(target_id)
                            .ok_or_else(|| CoreError::UnknownNode(target_id.clone()))?;
                        self.execute_node_with_ledger(
                            ctx,
                            path.child(target_id.clone()),
                            target_node,
                            ledger,
                            None,
                            iteration,
                        )
                        .await?
                    }
                    None => serde_json::Value::Null,
                }
            }
            NodeKind::SessionGroup {
                sessions,
                actor: _actor,
                body,
                limit,
            } => {
                // One session per element, each on its OWN ledger clone (like map
                // items — no shared-state race) under `path.child(session_id)`.
                // Sessions run concurrently; a `limit` (permits=1) serializes them
                // — the "poll under a center logic" joint. Session state lives in
                // checkpoints/env, not the shared ledger, so results are collected.
                let items = ledger.lock().unwrap().resolve(sessions)?;
                let items = items.as_array().cloned().ok_or_else(|| {
                    CoreError::from(crate::ledger::LedgerError::TypeMismatch(format!(
                        "session_group `{path:?}` sessions is not an array"
                    )))
                })?;
                let base = ledger.lock().unwrap().clone();

                let futures = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let session_id = item
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| i.to_string());
                        let session_path = path.child(session_id.clone());
                        let session_ledger = Arc::new(Mutex::new(base.with_item(item.clone())));
                        let limit = limit.clone();
                        Box::pin(async move {
                            self.emit(
                                ctx,
                                &session_path,
                                0,
                                EventKind::SessionStarted,
                                serde_json::json!({ "session": session_id }),
                            );
                            // Acquire the shared limit around the body; permits=1
                            // makes sessions take strict turns (nested acquire/
                            // release per session, never interleaved).
                            let guard = match &limit {
                                Some(l) => {
                                    let g = ctx.limits.acquire(&l.name).await?;
                                    self.emit(
                                        ctx,
                                        &session_path,
                                        0,
                                        EventKind::SemaphoreAcquired,
                                        serde_json::json!({ "limit": l.name }),
                                    );
                                    Some((l.name.clone(), g))
                                }
                                None => None,
                            };
                            let result = self
                                .execute_node_with_ledger(
                                    ctx,
                                    session_path.child("body"),
                                    body.as_ref(),
                                    session_ledger.as_ref(),
                                    None,
                                    0,
                                )
                                .await;
                            if let Some((name, g)) = guard {
                                drop(g); // releases the permit + wakes a waiter
                                self.emit(
                                    ctx,
                                    &session_path,
                                    0,
                                    EventKind::SemaphoreReleased,
                                    serde_json::json!({ "limit": name }),
                                );
                            }
                            result
                        })
                            as Pin<
                                Box<
                                    dyn Future<Output = Result<serde_json::Value, CoreError>>
                                        + Send
                                        + '_,
                                >,
                            >
                    })
                    .collect::<Vec<_>>();

                let mut collected = Vec::with_capacity(items.len());
                for result in join_all_ordered(futures).await {
                    collected.push(result?);
                }
                serde_json::Value::Array(collected)
            }
            NodeKind::ResumeSession { session, body } => {
                let session_value = ledger.lock().unwrap().resolve(session)?;
                let session_id = session_value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| session_value.to_string());
                // Rehydrate the latest checkpoint (if a store + a saved state
                // exist). Recorded actor/env outputs make the resumed run
                // replayable without re-executing prior turns.
                let state = match &self.checkpoints {
                    Some(store) => store
                        .load(&session_id)
                        .await?
                        .unwrap_or(serde_json::Value::Null),
                    None => serde_json::Value::Null,
                };
                self.emit(
                    ctx,
                    &path,
                    iteration,
                    EventKind::SessionResumed,
                    serde_json::json!({ "session": session_id }),
                );
                // Bind the rehydrated state as the `session` item source so the
                // body reads it as `session.<field>` — same binding shape as
                // `session_group`'s `item`.
                let body_ledger = Arc::new(Mutex::new(
                    ledger.lock().unwrap().with_item_as("session", state),
                ));
                self.execute_node_with_ledger(
                    ctx,
                    path.child("body"),
                    body,
                    body_ledger.as_ref(),
                    None,
                    iteration,
                )
                .await?
            }
        };
        self.emit(
            ctx,
            &path,
            iteration,
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
        iteration: u32,
    ) -> Result<serde_json::Value, CoreError> {
        let NodeKind::Map {
            over,
            item_as,
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
            self.execute_map_serial(ctx, &path, item_as, body, ledger, &items, iteration)
                .await
        } else {
            self.execute_map_parallel(
                ctx,
                &path,
                item_as,
                body,
                ledger,
                &items,
                MapDispatch { width, iteration },
            )
            .await
        }?;

        let cancelled = ctx.cancel.load(Ordering::Relaxed);
        let successes = results.iter().filter(|r| r.is_ok()).count();
        let attempted = results.len();
        let success_ratio = if total == 0 {
            1.0
        } else {
            successes as f64 / total as f64
        };
        // A cancelled map is intentionally partial — do NOT hold it to min_success
        // (that gate guards against flaky failures, not deliberate early stop).
        if !cancelled && success_ratio + f64::EPSILON < *min_success {
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
        ctx.record_node_io(
            path.clone(),
            iteration,
            serde_json::json!({
                "over": items,
                "item_as": item_as,
                "concurrency": width,
                "min_success": min_success,
            }),
            map_result.clone(),
            None,
            Some(serde_json::json!({
                "successes": successes,
                "total": total,
                "attempted": attempted,
                "cancelled": cancelled,
            })),
        );
        self.emit(
            ctx,
            &path,
            iteration,
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
        item_as: &str,
        body: &Node,
        ledger: &Mutex<Ledger>,
        items: &[serde_json::Value],
        iteration: u32,
    ) -> Result<Vec<Result<serde_json::Value, String>>, CoreError> {
        let base_ledger = ledger.lock().unwrap().clone();
        let mut results = Vec::with_capacity(items.len());

        for (i, item) in items.iter().cloned().enumerate() {
            // Cancelled mid-map (a met goal with `cancel_in_flight`): stop starting
            // new items. Those already done stay in `results`; the gate is skipped.
            if ctx.cancel.load(Ordering::Relaxed) {
                break;
            }
            let item_path = path.index(i as u32);
            self.emit(
                ctx,
                &item_path,
                iteration,
                EventKind::MapItemStarted,
                serde_json::json!({ "index": i }),
            );
            let item_ledger = Mutex::new(base_ledger.with_item_as(item_as, item));
            match self
                .execute_node_with_ledger(
                    ctx,
                    item_path.clone(),
                    body,
                    &item_ledger,
                    None,
                    iteration,
                )
                .await
            {
                Ok(result) => {
                    self.emit(
                        ctx,
                        &item_path,
                        iteration,
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
                        iteration,
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
        item_as: &str,
        body: &Node,
        ledger: &Mutex<Ledger>,
        items: &[serde_json::Value],
        dispatch: MapDispatch,
    ) -> Result<Vec<Result<serde_json::Value, String>>, CoreError> {
        let base_ledger = ledger.lock().unwrap().clone();
        let mut prepared = Vec::with_capacity(items.len());
        for (i, item) in items.iter().cloned().enumerate() {
            let item_ledger = base_ledger.with_item_as(item_as, item);
            let inputs = item_ledger.resolve_bindings(&body.inputs)?;
            prepared.push((i, item_ledger, inputs));
        }

        let mut results = (0..items.len()).map(|_| None).collect::<Vec<_>>();
        for chunk in prepared.chunks(dispatch.width) {
            // Cancelled (a met goal with `cancel_in_flight`): stop dispatching new
            // chunks. Undispatched slots stay `None` and are dropped below.
            if ctx.cancel.load(Ordering::Relaxed) {
                break;
            }
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
                            dispatch.iteration,
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
                                dispatch.iteration,
                            )
                            .await;
                        match result {
                            Ok(result) => {
                                self.emit(
                                    ctx,
                                    &item_path,
                                    dispatch.iteration,
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
                                    dispatch.iteration,
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

        // Drop undispatched slots (only present when cancelled mid-map). When not
        // cancelled every slot is filled, so this is an order-preserving identity.
        Ok(results.into_iter().flatten().collect())
    }

    async fn execute_for_each(
        &self,
        ctx: &RunCtx,
        path: NodePath,
        node: &Node,
        ledger: &Mutex<Ledger>,
        iteration: u32,
    ) -> Result<serde_json::Value, CoreError> {
        let NodeKind::ForEach {
            over,
            item_as,
            body,
        } = &node.kind
        else {
            unreachable!("execute_for_each is only called for for_each nodes");
        };

        let items = {
            let ledger = ledger.lock().unwrap();
            match ledger.resolve(over)? {
                serde_json::Value::Array(items) => items,
                _ => return Err(crate::ledger::LedgerError::TypeMismatch(over.0.clone()).into()),
            }
        };

        let mut last = serde_json::Value::Null;
        let mut input_items = Vec::with_capacity(items.len());
        for (i, item) in items.into_iter().enumerate() {
            input_items.push(item.clone());
            let restore = ledger.lock().unwrap().bind_item(item_as, item);
            let result = self
                .execute_node_with_ledger(ctx, path.index(i as u32), body, ledger, None, iteration)
                .await;
            ledger.lock().unwrap().restore_item(restore);
            last = result?;
        }
        ctx.record_node_io(
            path,
            iteration,
            serde_json::json!({
                "over": input_items,
                "item_as": item_as,
            }),
            last.clone(),
            None,
            None,
        );

        Ok(last)
    }

    // (goal plan resolution is a free function below `Runner`)

    /// Assemble the run record. `trace` (the ProcessNode tree) is built from the
    /// recorded outputs + node structure.
    fn build_manifest(
        &self,
        ctx: &RunCtx,
        spec: &WorkflowSpec,
        spec_hash: String,
        status: RunStatus,
        stop_reason: RunStopReason,
    ) -> RunManifest {
        // Build the trace BEFORE the struct literal: temporaries in a struct
        // initializer live until the whole literal completes, so a
        // `ctx.recorded.lock()` in a field would still be held when `build_trace`
        // re-locks `recorded` — a self-deadlock on the non-reentrant Mutex.
        let events = self.sink.snapshot();
        let trace = self.build_trace(ctx, spec, &events);
        RunManifest {
            run_id: ctx.run_id.clone(),
            workflow_name: spec.name.clone(),
            spec_hash,
            args: ctx.args.clone(),
            events,
            recorded: ctx.recorded.lock().unwrap().clone(),
            checkpoints: ctx.checkpoints.lock().unwrap().clone(),
            trace,
            status,
            stop_reason,
            // Host meters resource budgets (tokens/wall/actor calls) and may
            // attach a [`BudgetSnapshot`] after the run; core leaves it empty.
            budgets: None,
            goals: None,
            invariants: None,
        }
    }

    /// Fold `recorded` + node structure into the ProcessNode tree (ADR #2, the
    /// optimizer-facing artifact). Interior nodes mirror the topology path
    /// (`map:audit_jobs` → `[0]` → …); leaves are the actual actor/env calls,
    /// carrying the outputs/score/signal/artifacts the optimizer grades. Built
    /// from `recorded` sorted by [`Addr`] so the tree is deterministic and
    /// replay-stable — same run, same tree.
    fn build_trace(
        &self,
        ctx: &RunCtx,
        spec: &WorkflowSpec,
        events: &[Event],
    ) -> Option<ProcessNode> {
        let recorded = ctx.recorded.lock().unwrap();
        let node_io = ctx.node_io.lock().unwrap().clone();
        if recorded.is_empty() && node_io.is_empty() {
            return None;
        }
        let event_inputs = event_inputs_by_addr(events);
        let mut root = ProcessNode {
            addr: Addr {
                run_id: ctx.run_id.clone(),
                node_path: NodePath::root(),
                iteration: 0,
                local_seq: 0,
            },
            label: format!("workflow:{}", spec.name),
            inputs: ctx.args.clone(),
            outputs: ctx.ledger.lock().unwrap().snapshot_json(),
            score: None,
            signal: None,
            artifacts: Vec::new(),
            children: Vec::new(),
        };
        let mut items: Vec<&RecordedOutput> = recorded.iter().collect();
        items.sort_by(|a, b| a.addr.cmp(&b.addr));
        for rec in items {
            insert_recorded(
                &mut root,
                &ctx.run_id,
                NodePath::root(),
                &rec.addr.node_path.0,
                rec,
                &node_io,
                &event_inputs,
            );
        }
        let mut node_items: Vec<(&(NodePath, u32), &NodeIo)> = node_io.iter().collect();
        node_items.sort_by(|a, b| a.0.cmp(b.0));
        for ((path, iteration), io) in node_items {
            insert_node_io(&mut root, &ctx.run_id, path, *iteration, io);
        }
        sort_trace(&mut root);
        Some(root)
    }

    async fn event_inputs_payload(
        &self,
        inputs: serde_json::Value,
    ) -> Result<serde_json::Value, CoreError> {
        let bytes = serde_json::to_vec(&inputs).map_err(|err| CoreError::Json(err.to_string()))?;
        if bytes.len() <= EVENT_INPUT_INLINE_LIMIT_BYTES {
            return Ok(inputs);
        }

        let reference = self.store.put(bytes, "application/json").await?;
        Ok(serde_json::json!({ "artifact_ref": reference }))
    }
}

/// Insert one recorded call into the trace tree, descending `segs` and creating
/// interior nodes on the way. `prefix` accumulates the path so interior nodes
/// carry a real [`Addr`]. When `segs` is empty we've reached the call's node —
/// the call is attached as a leaf so sibling calls under one node (a while
/// loop's turns, an env's observe+step) stay distinct.
fn insert_recorded(
    node: &mut ProcessNode,
    run_id: &str,
    prefix: NodePath,
    segs: &[PathSeg],
    rec: &RecordedOutput,
    node_io: &HashMap<(NodePath, u32), NodeIo>,
    event_inputs: &HashMap<Addr, serde_json::Value>,
) {
    let Some((head, tail)) = segs.split_first() else {
        node.children.push(leaf_from(rec, event_inputs));
        return;
    };
    let mut child_prefix = prefix;
    child_prefix.0.push(head.clone());
    let label = seg_label(head);
    let pos = node
        .children
        .iter()
        .position(|c| c.addr.node_path == child_prefix && c.label == label);
    let idx = match pos {
        Some(idx) => idx,
        None => {
            let io = node_io.get(&(child_prefix.clone(), rec.addr.iteration));
            node.children.push(ProcessNode {
                addr: Addr {
                    run_id: run_id.to_string(),
                    node_path: child_prefix.clone(),
                    iteration: rec.addr.iteration,
                    local_seq: 0,
                },
                label,
                inputs: io
                    .map(|io| io.inputs.clone())
                    .unwrap_or(serde_json::Value::Null),
                outputs: io
                    .map(|io| io.outputs.clone())
                    .unwrap_or(serde_json::Value::Null),
                score: io.and_then(|io| io.score),
                signal: io.and_then(|io| io.signal.clone()),
                artifacts: Vec::new(),
                children: Vec::new(),
            });
            node.children.len() - 1
        }
    };
    insert_recorded(
        &mut node.children[idx],
        run_id,
        child_prefix,
        tail,
        rec,
        node_io,
        event_inputs,
    );
}

fn insert_node_io(
    node: &mut ProcessNode,
    run_id: &str,
    path: &NodePath,
    iteration: u32,
    io: &NodeIo,
) {
    let mut prefix = NodePath::root();
    let mut current = node;
    for seg in &path.0 {
        prefix.0.push(seg.clone());
        let label = seg_label(seg);
        let pos = current
            .children
            .iter()
            .position(|c| c.addr.node_path == prefix && c.label == label);
        let idx = match pos {
            Some(idx) => idx,
            None => {
                current.children.push(ProcessNode {
                    addr: Addr {
                        run_id: run_id.to_string(),
                        node_path: prefix.clone(),
                        iteration,
                        local_seq: 0,
                    },
                    label,
                    inputs: serde_json::Value::Null,
                    outputs: serde_json::Value::Null,
                    score: None,
                    signal: None,
                    artifacts: Vec::new(),
                    children: Vec::new(),
                });
                current.children.len() - 1
            }
        };
        current = &mut current.children[idx];
    }
    current.inputs = io.inputs.clone();
    current.outputs = io.outputs.clone();
    current.score = io.score;
    current.signal = io.signal.clone();
}

/// A leaf ProcessNode for a recorded call. Inputs are emitted with the impure
/// invocation event and joined back by structural `Addr`; `RecordedOutput` stays
/// focused on replay outputs.
fn leaf_from(rec: &RecordedOutput, event_inputs: &HashMap<Addr, serde_json::Value>) -> ProcessNode {
    ProcessNode {
        addr: rec.addr.clone(),
        label: call_label(&rec.call),
        inputs: event_inputs
            .get(&rec.addr)
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        outputs: rec.outputs.clone(),
        score: rec.score,
        signal: rec.signal.clone(),
        artifacts: rec.artifacts.clone(),
        children: Vec::new(),
    }
}

fn event_inputs_by_addr(events: &[Event]) -> HashMap<Addr, serde_json::Value> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::ActorInvoked | EventKind::ResourceInvoked
            )
        })
        .filter_map(|event| {
            event
                .payload
                .get("inputs")
                .cloned()
                .map(|inputs| (event.addr.clone(), inputs))
        })
        .collect()
}

fn reduce_outcome(outputs: &serde_json::Value) -> (Option<f64>, Option<serde_json::Value>) {
    let summary = outputs.get("summary").unwrap_or(outputs);
    let score = explicit_score(summary)
        .or_else(|| ratio_score(summary, "passed", "total"))
        .or_else(|| inverse_count_score(summary, "blockers", "total"))
        .or_else(|| verdict_score(summary));
    let signal = reduce_signal(summary);
    (score, signal)
}

fn explicit_score(value: &serde_json::Value) -> Option<f64> {
    value.get("score").and_then(serde_json::Value::as_f64)
}

fn ratio_score(value: &serde_json::Value, numerator: &str, denominator: &str) -> Option<f64> {
    let numerator = value.get(numerator)?.as_f64()?;
    let denominator = value.get(denominator)?.as_f64()?;
    if denominator > 0.0 {
        Some((numerator / denominator).clamp(0.0, 1.0))
    } else {
        None
    }
}

fn inverse_count_score(value: &serde_json::Value, count: &str, total: &str) -> Option<f64> {
    let count = value.get(count)?.as_f64()?;
    let total = value.get(total)?.as_f64()?;
    if total > 0.0 {
        Some(((total - count) / total).clamp(0.0, 1.0))
    } else {
        None
    }
}

fn verdict_score(value: &serde_json::Value) -> Option<f64> {
    match value.get("verdict").and_then(serde_json::Value::as_str) {
        Some(verdict) if verdict.eq_ignore_ascii_case("pass") => Some(1.0),
        Some(verdict) if verdict.eq_ignore_ascii_case("fail") => Some(0.0),
        _ => None,
    }
}

fn reduce_signal(summary: &serde_json::Value) -> Option<serde_json::Value> {
    let mut signal = serde_json::Map::new();
    for key in [
        "verdict",
        "passed",
        "failed",
        "total",
        "blockers",
        "annotated",
    ] {
        if let Some(value) = summary.get(key) {
            signal.insert(key.to_string(), value.clone());
        }
    }
    if signal.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(signal))
    }
}

fn sort_trace(node: &mut ProcessNode) {
    node.children.sort_by(|a, b| a.addr.cmp(&b.addr));
    for child in &mut node.children {
        sort_trace(child);
    }
}

fn seg_label(seg: &PathSeg) -> String {
    match seg {
        PathSeg::Node(name) => name.clone(),
        PathSeg::Index(i) => format!("[{i}]"),
    }
}

fn call_label(call: &CallKind) -> String {
    match call {
        CallKind::Actor { actor } => format!("actor:{actor}"),
        CallKind::ResourceObserve { session } => format!("observe:{session}"),
        CallKind::ResourceStep { session } => format!("step:{session}"),
    }
}

fn is_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => {
            value.as_i64().is_some_and(|value| value != 0)
                || value.as_u64().is_some_and(|value| value != 0)
                || value.as_f64().is_some_and(|value| value != 0.0)
        }
        serde_json::Value::String(value) => !value.is_empty(),
        serde_json::Value::Array(value) => !value.is_empty(),
        serde_json::Value::Object(value) => !value.is_empty(),
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

/// Attach the failing node's name to a binding-resolution error, so
/// `unresolved reference: ledger.docs_json` becomes
/// `... (resolving inputs of node `expand_pages`)` — the caller learns *which*
/// node's inputs couldn't resolve, not just which reference. Same error variant
/// (louder string only), so exact-variant matchers keep working.
fn at_node(err: crate::ledger::LedgerError, path: &NodePath) -> crate::ledger::LedgerError {
    use crate::ledger::LedgerError;
    let node = path
        .0
        .iter()
        .rev()
        .find_map(|seg| match seg {
            PathSeg::Node(name) => Some(name.clone()),
            PathSeg::Index(_) => None,
        })
        .unwrap_or_else(|| "root".to_string());
    let ctx = format!(" (resolving inputs of node `{node}`)");
    match err {
        LedgerError::Unresolved(r) => LedgerError::Unresolved(format!("{r}{ctx}")),
        LedgerError::TypeMismatch(r) => LedgerError::TypeMismatch(format!("{r}{ctx}")),
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
    #[error(transparent)]
    Limit(#[from] crate::limits::LimitError),
    #[error("json serialization failed: {0}")]
    Json(String),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("program not registered: {0}")]
    UnknownProgram(String),
    #[error("map min_success gate failed: required {required}, got {successes}/{total}")]
    MapMinSuccess {
        required: f64,
        successes: usize,
        total: usize,
    },
}
