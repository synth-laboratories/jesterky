# HANDOFF — Process-tree I/O (the optimizer data plane)

**Next core implementation.** Fills `ProcessNode` inputs / outputs / scores so the
trace is actually consumable by optimizers. This is Tier-1 #1 in `jesterky_notes.md`
§10 ("process tree is optimizer-starved") and the true prerequisite for critical-path
step 1 (GEPA/GELO consuming `state/jesterky_*`). Core-only; no host/model work.

Prereq context: goals engine v1 just shipped (`docs/GOALS.md`,
`crates/jesterky-contract/src/goal.rs`, runner eval). This handoff is independent of it.

---

## Why this, why now

The whole point of jesterky's `RunManifest.trace` (ProcessNode tree, ADR #2) is to
be the optimizer-facing artifact — the thing GEPA/GELO grade and mutate. Right now
it is **hollow**: an optimizer reading the trace sees mostly `null`. Until it carries
real I/O + scores, the optimizer hook (critical path step 1) has nothing to consume,
so this blocks the integration that changes jesterky's grade from "good substrate"
to "used substrate."

---

## Current state (measured, not assumed)

`crates/jesterky-core/src/runner.rs`, `build_trace` + `insert_recorded` + `leaf_from`
(~lines 893–984):

| Node | `inputs` | `outputs` | `score` / `signal` |
|---|---|---|---|
| root `workflow:<name>` | `ctx.args` ✅ | **`Null`** ❌ | None |
| interior (`map:…`, `[i]`, `reduce:…`) | **`Null`** ❌ | **`Null`** ❌ | None |
| leaf (`actor:…`, `observe:…`, `step:…`) | **`Null`** ❌ | `rec.outputs` ✅ | `rec.score` / `rec.signal` (usually None) |

**Landmine — the `leaf_from` comment is wrong.** It says "inputs live in the event
stream … the label + addr join it back to the `ActorInvoked` event that carries them."
They do **not**. The emit is:

```rust
// runner.rs ~383
self.emit(ctx, &path, iteration, EventKind::ActorInvoked,
          serde_json::json!({ "actor": actor }));   // <-- inputs NOT included
```

The resolved `inputs` are moved into `ActorRequest` and dropped. `RecordedOutput`
has no `inputs` field either. **So leaf inputs are recoverable from nowhere today** —
filling them requires recording them, not just joining. Fix the comment as part of
Phase 1.

---

## Plan (3 phases, land independently)

### Phase 1 — Leaf inputs (highest value; unblocks optimizer)

Optimizers need the (input → output [→ score]) triple per call. Inputs are the
missing leg.

**Approach A (recommended, non-breaking): put inputs on the event payload.**
`ActorInvoked`/`ResourceInvoked` payloads are free-form `serde_json::Value` — adding
a field is **not** a closed-enum/contract change (ADR #3 is about node kinds + event
*kinds*, not payload shape).

1. Clone `inputs` before `actor.drive(...)` (it's currently moved).
2. Emit `json!({ "actor": actor, "inputs": inputs_clone })` (and the resource path).
3. In `build_trace`, build an `addr → inputs` map from `ctx.events` filtered to
   `ActorInvoked`/`ResourceInvoked`, and have `leaf_from` fill `inputs` from it.
4. Fix the `leaf_from` doc comment.

*Alternative B (contract change):* add `inputs: serde_json::Value` to `RecordedOutput`.
Cleaner for `build_trace` (no event join) but bumps `jesterky.manifest.schema.json`
and every `RecordedOutput` literal. Prefer A unless the event-join map proves ugly.

**Guardrail:** inputs can be large (full essay bodies, batch JSON). Offload via the
existing `ArtifactStore` seam when over a threshold, mirroring how large outputs are
handled — do **not** inline megabytes into every event. Check `store_outputs` /
`ArtifactRef` for the existing offload pattern before inlining.

**Acceptance:** new `tests/process_tree_io.rs` — run `quality_scan` with `--actor fake`,
assert every `actor:quality_scanner` leaf has non-null `inputs` matching the bound
`item`. Assert replay still reproduces the trace (ADR #7 — the event carries the
input deterministically, so replay is unaffected; add a replay assertion).

### Phase 2 — Root + interior outputs

- **Root `outputs`:** attach the settled ledger. `Ledger::snapshot_json()` already
  exists (added for goals) — set `root.outputs = ctx.ledger.lock().snapshot_json()`.
  Free win; gives optimizers the final work product at the top of the tree.
- **Interior `outputs`:** a `map:` node's collected result and a `reduce:` node's
  output are computed in `execute_map` / the reduce path (`store_outputs(&node.outputs,
  &result)`). Record node-level output keyed by node path (new `ctx.node_io: Mutex<
  HashMap<NodePath, (inputs, outputs)>>` populated at execute time), then fill interior
  nodes in `insert_recorded`. Interior `inputs` = the resolved binding inputs, same map.

**Acceptance:** assert `reduce:aggregate` interior node has `outputs.summary.verdict`
populated and the root `outputs.summary` mirrors the ledger.

### Phase 3 — Scores / signals from reduces (optimizer grade surface)

Actor `score` is usually `None` (models grade outcomes, not every call). The optimizer
wants a per-subtree score. Let a `reduce` op return an optional `score`/`signal`
(the aggregate already computes `passed`/`verdict` — surface it as a score) and
propagate to the interior reduce node. Keep the **no-code-quality-verdict** rule:
code only surfaces scores the actor/verifier produced; it does not invent quality
judgments (see `feedback_no_code_actor_quality_verdicts`).

**Acceptance:** `reduce:aggregate` node carries a numeric `score` derived from the
aggregate's own output; assert it's present and matches.

---

## Touch points

| File | Change |
|---|---|
| `crates/jesterky-core/src/runner.rs` | `execute_node` actor/resource arms (emit inputs); `build_trace`/`insert_recorded`/`leaf_from` (fill I/O); optional `RunCtx.node_io` |
| `crates/jesterky-core/src/ledger.rs` | reuse `snapshot_json()` (already added) |
| `crates/jesterky-contract/src/event.rs` | none for Approach A (payload is free JSON) |
| `crates/jesterky-contract/src/artifact.rs` | only if Approach B (add `RecordedOutput.inputs`) |
| `crates/jesterky-core/tests/process_tree_io.rs` | new |
| `jesterky.manifest.schema.json` | regenerate only if Approach B |

---

## ADR guardrails (do not regress)

- **ADR #5** — event identity is the structural `Addr`; the addr→inputs join keys on
  `Addr`, never emission order. Do not add a global counter.
- **ADR #7** — replay re-drives orchestration with recorded impure outputs. Inputs on
  the event are deterministic replay metadata; assert replay fidelity holds.
- **§11 landmine** — inputs/outputs can be huge; offload to `ArtifactStore`, don't
  inline unbounded blobs into the event stream.
- Keep core free of model/HTTP/clock (ADR #6) — this is all pure data plumbing.

---

## After this: the optimizer hook is unblocked (but DEFERRED)

This makes the trace *consumable*, but the optimizer hook is **deferred** until the
foundational core work (this + `HANDOFF_jesterky_core_residuals.md` Tier A–C) lands.
Dependency direction is one-way: **`optimizers` consumes `jesterky`, never the
reverse** — the hook is authored on the optimizers side (materializing
`state/jesterky_*` from this trace); jesterky only owes a good trace. No optimizer
dependency ever enters a jesterky crate.

---

## Alternative if you'd rather finish goals first

Smaller, self-contained: **goals v2** — execute the `finalize` node on success
wrap-up, and in-flight cancel of running map siblings on early-terminate (v1 skips
remaining entrypoints only). See `docs/GOALS.md` "Not in v1". Lower strategic
leverage than process-tree I/O, but completes the control plane just shipped.
