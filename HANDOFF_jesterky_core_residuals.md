# HANDOFF — Core logic residuals

The remaining **core-implementation** work after process-tree I/O
(`HANDOFF_jesterky_process_tree_io.md`). Everything here is inside the Rust crates —
no ship hygiene (CI, publish, GitHub-public), no Stack/Hosted/content. Grounded in
the actual code as of the goals-v1 ship.

Tiers are by leverage, not strict order. Tier A items are small and can land in one
PR; Tier B is the data/typing plane; Tier C completes the control planes; the
Capstone (optimizer hook) is a separate large handoff, blocked on process-tree I/O.

Master list this expands: `jesterky_notes.md` §21. Items keyed to the "what remains"
numbering (#2–#10, #16).

---

## Tier A — correctness / hygiene (small, land together)

### #6 — Dual-write emit cleanup
**State.** `runner.rs::emit` (~line 244) writes every event **twice**:
```rust
self.sink.emit(event.clone());
ctx.events.lock().unwrap().push(event);
```
`build_manifest` reads `ctx.events`; the `EventSink` is a parallel copy (used by
`SharedEventSink` for `--follow`). Two sources of the same truth can diverge.
**Plan.** Make the sink the single owner. `RunCtx` holds an `Arc<dyn EventSink>`;
`build_manifest` drains events from a `MemEventSink`-backed collector instead of
`ctx.events`. `--follow` already has its own `SharedEventSink`; a `TeeSink` (mem +
shared) keeps both consumers without a second `Vec` in `RunCtx`.
**Acceptance.** Existing replay/ordering tests stay green; assert `manifest.events`
equals the sink's drained events. No `ctx.events` field remains.
**Touch.** `runner.rs` (`RunCtx`, `emit`, `build_manifest`), `traits.rs` (EventSink),
`jesterky-actor` sinks.

### #7 — Mailbox: wire or hide
**State.** `Mailbox` is constructed (`runner.rs:120`, field at `:87`) and **never
read** — a dangling capability. §5 of the notes calls the shared event stream "one
stream per run" but the mailbox is not hooked to any publish node.
**Plan.** Decide: (a) wire it to a `publish`/`session_group` fan-in node with a real
consumer, or (b) delete the field + module until there's a consumer. Default **(b)**
— don't ship a dead seam that implies a feature. If (a), it's a node-kind change
(ADR #3) and needs its own design.
**Acceptance.** Either a test exercising mailbox publish→consume, or the field/module
gone and the build clean.
**Touch.** `runner.rs`, `mailbox.rs`.

### #10 — Kill absolute default paths
**State.** `DEFAULT_BLOG_DIR` / `DEFAULT_DOCS_DIR` hardcode `/Users/joshpurtell/…`
(quality-scan hosts). Breaks for every other user; an OSS-ship blocker.
**Plan.** Require the path via spec/args/flag; no machine-absolute default. Error
clearly when unset. `grep -rn '/Users/joshpurtell' crates/` to find them all.
**Acceptance.** `grep -rn '/Users/' crates/` returns nothing; scan specs pass the
dir explicitly.
**Touch.** `jesterky-quality`, any example specs referencing the constants.

---

## Tier B — data plane & typing honesty

### #3 — `--events-out` NDJSON (streamed event API)
**State.** Two live streams, neither a real API: the durable `EventSink`/`ctx.events`
(post-hoc, in the manifest) and the ephemeral `LiveBus`/`LiveEvent` (`--follow` only,
not serialized). No way to tail a run's canonical event stream as it happens.
**Plan.** Add `--events-out <path|->`: a host `EventSink` that appends each `Event`
as one NDJSON line (canonical `Addr`-ordered on flush). This is the honest precursor
to SSE (`jesterky_notes.md` §5 "add `--events-out` / multi-subscriber host sink
before SSE"). Keep core IO-free — the NDJSON sink is a host (CLI/actor) sink.
**Acceptance.** `jesterky run … --events-out run.ndjson`; each line parses to `Event`;
line set equals `manifest.events`; sorting by `addr` matches the manifest's canonical
order.
**Touch.** `jesterky-cli` (flag + sink wiring), `jesterky-actor` (NDJSON sink), a CLI
test.

### #4 — Typing honesty (Ref AST + binding validation)
**State.** `ledger.rs::resolve_bindings` + `item_source_and_path` parse a **subset**
(`ledger.<k>`, `item.<path>`, literals) with a `TODO(M0)` to fully parse a `Ref` into
`{source, path}`. Unresolvable/mistyped bindings are discovered at run time, not at
`validate_and_hash`.
**Plan.** Either (a) parse `Ref` into a typed `{source, path}` AST and **validate all
bindings at spec-validation time** (unknown source / dangling ledger key → a
`Diagnostic`, not a runtime error), or (b) if full typing is deferred, document
"JSON-only, resolved by declared bindings" and enforce host output schema when
declared (see #5). Pick (a) — it's the ADR #3 "declared-edge I/O" promise made real.
**Acceptance.** A spec with a dangling `Ref` fails `validate()` with a pointed
`Diagnostic`; valid specs unaffected; add parse unit tests for each source form.
**Touch.** `jesterky-contract` (`Ref` parse + `topology::validate`), `ledger.rs`
(use the AST), tests.

### #5 — Schema-validate actor outputs when declared
**State.** `HostConfig` can declare per-actor output schemas (used for docs/prompts),
but the runner does **not** validate an actor's returned JSON against them — a
malformed model output flows into the ledger unchecked.
**Plan.** When a node's actor has a declared output schema, validate the actor result
after `drive()`; on mismatch, fail the node (retryable, like other actor failures)
with a schema-violation reason. Reuse the `schemars`/`jsonschema` path already in the
crate.
**Acceptance.** An actor returning off-schema JSON fails the node with a clear reason;
on-schema passes; a fake-actor test covers both.
**Touch.** `runner.rs` (actor arm, post-`drive`), `jesterky-contract` (schema lookup
on `HostConfig`), test.

---

## Tier C — control-plane completion

### #2 — Goals v2 (finalize + in-flight cancel)
**State.** Goals v1 shipped (`docs/GOALS.md`): runner evaluates goals, early-terminate
**skips remaining entrypoints**, `fail_on_unmet`. Two gaps: `GoalPlan.finalize` is
recorded but **never executed**, and early-terminate does **not** cancel a parallel
`map` already in flight (entrypoint-skip only).
**Plan.**
- *Finalize:* on success wrap-up (all required met, `terminate_on_met`), if
  `finalize` names a node, execute that node before finalizing `Completed`. It runs
  once, after the break, against the settled ledger.
- *In-flight cancel:* thread a cancellation token into `execute_map` so a met-goal
  can stop pending map items. This touches the concurrency core — gate behind a plan
  flag (`cancel_in_flight`, default false) so v1 semantics are preserved by default.
**Acceptance.** A spec with `finalize` runs the finalize node exactly once on early
success (assert its event/recorded output present, and absent when goals unmet). A
cancel test: a slow map with a goal met mid-flight leaves pending items unstarted.
**Touch.** `runner.rs` (run loop finalize call; `execute_map` cancel token),
`goal.rs` (already has `finalize` field; add `cancel_in_flight`), `docs/GOALS.md`,
tests.

### #8 — Budget residuals (reserve + stopper vocabulary)
**State.** `BudgetEngine` projects spent/ETA and the CLI fails on hard-exhaust, but:
(a) no **reserved** budget accounting under wide parallel maps (N in-flight calls can
blow a token cap before the meter catches up), and (b) the manifest has no explicit
**stop reason** — a budget-failed run and a node-failed run both read `status:
failed` with no `budget_exhausted` marker.
**Plan.**
- *Reserve:* host meters a reservation when a map dispatches K concurrent actor calls,
  released on completion; `BudgetStatus` gains `reserved` (dual of optimizers'
  `spent + reserved`). Pure-projection stays pure; the host supplies reservations as
  observations.
- *Stopper vocabulary:* add a `stop_reason` to the manifest (`completed` |
  `budget_exhausted` | `goal_unmet` | `node_failed`) so consumers don't string-match
  the `WorkflowFailed` payload.
**Acceptance.** A wide map with a low token cap trips exhaust from reservations before
overshooting; `stop_reason` is set on each terminal path; tests per reason.
**Touch.** `budget.rs` (`reserved` field), `jesterky-cli` (reservation observations,
`stop_reason`), `artifact.rs` (`RunManifest.stop_reason`), schema regen, tests.

### #9 — Invariant report + stronger parity fixture
**State.** mloky parity is outcome-layer only (conservation + termination). No
per-run invariant report; the parity fixture is minimal.
**Plan.** Emit an invariant report on every map/reduce run (min_success honored,
child count == input count, no orphaned records) as a structured manifest addendum.
Strengthen the mloky fixture beyond the 8-job reference. Coordinate an mloky freeze
so the oracle can't drift (that freeze itself is ship-hygiene, out of scope here).
**Acceptance.** Invariant report present on map/reduce manifests; a violation is a
test failure; expanded fixture passes `conformance`.
**Touch.** `runner.rs` / `jesterky-quality`, `jesterky-contract` (report type),
fixtures.

---

## Capstone — #16 Optimizer hook (DEFERRED, separate large handoff)

**Deferred until the foundational core work (process-tree I/O + Tier A–C) lands.**
Not started; do not begin until the trace is real and typed.

**Dependency direction is fixed and one-way: `optimizers` depends on `jesterky`,
never the reverse.** jesterky is the substrate; the optimizer repo *consumes* it
(reads its `RunManifest`/ProcessNode trace, links `jesterky-contract` types).
jesterky must **never** take a dependency on `optimizers` — no optimizer types,
crates, or imports in any jesterky crate. The integration is authored on the
optimizers side (`optimizers/` materializing `state/jesterky_*` from the trace);
jesterky's only obligation is to emit a good trace, which is what process-tree I/O
delivers.

When it resumes (its own handoff): once the trace carries (inputs → outputs → score),
materialize `state/jesterky_*` into the GEPA/GELO proposer workspace and run a
Craftax A/B — cross-repo work in `optimizers/`, spanning the Chinese-wall +
workspace-file contracts (`optimizers/skills/gepa/SKILL.md`, `jesterky_notes.md` §14).
Residuals Tier A–C are independent of it; #1 process-tree I/O is its gate.

---

## Suggested order

1. Tier A in one PR (dual-write, mailbox, paths) — small, unblocks a clean tree.
2. #1 process-tree I/O (other handoff) — the strategic gate.
3. Tier B (#3 events-out, #4 typing, #5 output schema) — the consumer data/typing plane.
4. Tier C (#2 goals v2, #8 budgets, #9 invariants) — finish the control planes.
5. Capstone #16 — **DEFERRED** until 1–4 land; then its own handoff, authored on the
   optimizers side (jesterky never depends on optimizers).

## ADR guardrails (all items)
- #5 event `Addr` identity — never a global emit counter.
- #6 core/host seam — new sinks/validation live host-side or in pure contract code; core stays IO/model/clock-free.
- #7 replay — recorded impure outputs re-drive orchestration; any new recording must be replay-deterministic.
- #3 closed node kinds — mailbox-wire / new I/O must not smuggle an eval DSL into JSON.
