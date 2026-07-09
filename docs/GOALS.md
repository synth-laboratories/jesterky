# Goals, work products, and semantic termination

Goals are the **semantic dual** of resource [budgets](./BUDGETS.md). Budgets cap
*spend* and answer "will I hit the token/wall cap?". Goals set *targets on
achievement* and answer "have I already produced the thing I wanted?".

A run that structurally finished its graph may still not have produced its
deliverable; a run that produced it early can stop. Goals make that termination
**semantic** (a predicate over the ledger), not just structural (graph-finished).

| Concept | Field | Meaning |
|---|---|---|
| **Resource budgets** | `runplan.budgets` | Caps on calls / tokens / wall — how much you may *spend* |
| **Goals / work products** | `runplan.goals` | Targets on *achievement* — what must be true to be done |

Machine-readable schema: root `jesterky.schema.json` → definitions `GoalPlan`,
`GoalSpec`, `GoalKind`, `GoalVizConfig`. Snapshot: `jesterky.manifest.schema.json`
→ `GoalSnapshot`, `GoalStatus`, `GoalState`.

---

## Budgets vs goals

| Budget engine | Goal engine |
|---|---|
| Caps on *spend* | Targets on *achievement* |
| Progress: `spent / max` | Progress: hit (0/1) or `value / min` |
| Exhausted → optional **fail** (`fail_on_hard_exhaust`) | Unmet required → optional **fail** (`fail_on_unmet`) |
| Host **meters** tokens/wall/calls | Runner **evaluates** predicates over the ledger |
| `BudgetSnapshot` on `RunManifest.budgets` | `GoalSnapshot` on `RunManifest.goals` |

Because goals are pure predicates over ledger state the core already owns, the
**runner** evaluates them (no host metering needed). This is also what lets the
runner terminate early — see [Early success wrap-up](#early-success-wrap-up).

---

## Typed interface (`jesterky-contract`)

All `Serialize` / `Deserialize` / `JsonSchema`, re-exported from the crate root.

| Type | Role |
|---|---|
| [`GoalPlan`](../crates/jesterky-contract/src/goal.rs) | Full JSON config (`runplan.goals`) + `overlay_json` |
| [`GoalSpec`](../crates/jesterky-contract/src/goal.rs) | One goal (`id`, `required`, predicate) |
| [`GoalKind`](../crates/jesterky-contract/src/goal.rs) | `ledger_pred` \| `metric_threshold` |
| [`GoalVizConfig`](../crates/jesterky-contract/src/goal.rs) | Terminal panel knobs |
| [`GoalEngine`](../crates/jesterky-contract/src/goal.rs) | Pure projection: plan + ledger → snapshot |
| [`GoalSnapshot`](../crates/jesterky-contract/src/goal.rs) | Live + final result; stored on `RunManifest.goals` |
| [`GoalStatus`](../crates/jesterky-contract/src/goal.rs) | Per-goal state + progress + detail |
| [`GoalState`](../crates/jesterky-contract/src/goal.rs) | `met` \| `unmet` \| `unknown` |

```rust
use jesterky_contract::{GoalEngine, GoalPlan};

// plan from workflow JSON; ledger is a JSON object of slot keys → values
let snap = GoalEngine::snapshot("run-1", &plan, &ledger_json);
assert!(snap.all_required_met());
```

---

## Declaring goals in workflow JSON

Goals live on `runplan.goals` and may be overridden per run with
`--args '{"goals":{...}}'` (deep-merged via `GoalPlan::overlay_json`; the `goals`
array **replaces** when present).

```json
"runplan": {
  "goals": {
    "terminate_on_met": true,
    "fail_on_unmet": true,
    "goals": [
      { "id": "all_pass", "kind": "ledger_pred", "path": "summary.verdict", "equals": "pass" },
      { "id": "coverage", "kind": "metric_threshold", "path": "summary.passed", "min": 6, "required": false }
    ]
  }
}
```

### Goal kinds (v1)

| `kind` | Fields | Met when |
|---|---|---|
| `ledger_pred` | `path`, `equals` | ledger value at `path` deep-equals `equals` |
| `metric_threshold` | `path`, `min` | numeric ledger value at `path` is `>= min` |

`path` is a dotted path into the ledger snapshot: segments split on `.`, numeric
segments index into arrays (`results.0.reward`). The ledger is seeded with the
run args' top-level keys and each node's declared `outputs` bindings — so a
reduce writing `outputs: { "summary": "ledger.summary" }` makes `summary.verdict`
addressable.

### Plan knobs

| Field | Default | Meaning |
|---|---|---|
| `goals` | `[]` | The declared goals. Empty ⇒ no evaluation, no panel line. |
| `terminate_on_met` | `true` | When every **required** goal is met, skip remaining entrypoints. |
| `fail_on_unmet` | `true` | A required goal still unmet at run end → `RunStatus::Failed`. |
| `finalize` | `null` | Optional node id to execute once after early success wrap-up. |
| `cancel_in_flight` | `false` | With `terminate_on_met: false`, keep running the graph but stop fanning out pending `map`/`for_each` items once the goal is met (finish cheaply). A cancelled map is exempt from its `min_success` gate. |
| `viz` | on | Panel goal line knobs. |

### Per-goal fields

| Field | Default | Meaning |
|---|---|---|
| `id` | — | Stable identifier (unique in the plan). |
| `required` | `true` | Required goals gate the run state and `fail_on_unmet`. Non-required record progress only. |
| `label` | `id` | Display name. |
| `show_progress` | `true` | Include on the panel goal line. |

---

## Semantics

| Surface | Meaning |
|---|---|
| **Per-goal progress** | `[0,1]`: `1.0` met; `value/min` for a threshold; `0.0` unmet/unknown. |
| **Per-goal state** | `met` / `unmet` / `unknown` (path missing or wrong type). |
| **Run state** | `met` iff every `required` goal is met. Non-required goals never block. |
| **Unknown** | An unresolved path is `unknown`, which is *not* met — a required unknown blocks the run. |

### Early success wrap-up

When `terminate_on_met` is true, the runner re-evaluates goals after each
entrypoint node. Once every required goal is met it **skips the remaining
entrypoints**, runs the optional `finalize` node once, and finalizes `Completed`.
Set `terminate_on_met: false` to always run the full graph (a pure quality gate).

With `terminate_on_met: false` **and** `cancel_in_flight: true`, the graph keeps
running after the goal is met, but any subsequent `map`/`for_each` node stops
starting new items — the run finishes the remaining nodes cheaply instead of
fanning out work whose result is no longer needed. Cancellation is checked
between items (serial) and between dispatch chunks (parallel); items already
running complete, and a cancelled map is exempt from its `min_success` gate.
Note: goal state only advances when a node writes the ledger, so a goal cannot
flip *within* a single map — cancellation takes effect at the next node.

### Fail on unmet

At run end, a required goal still unmet fails the run when `fail_on_unmet` is
true — the dual of budget hard-exhaust. A `WorkflowFailed` event carries a
`goal_unmet: <ids>` reason. Node failures take precedence over goal failures.

---

## Snapshot on the manifest

`RunManifest.goals: Option<GoalSnapshot>` (absent when no goals were declared):

```json
"goals": {
  "schema_version": "goal_engine.v1",
  "run_id": "goal-demo-001",
  "state": "met",
  "required_total": 1,
  "required_met": 1,
  "terminated_early": false,
  "items": [
    { "id": "all_pass", "kind": "ledger_pred", "required": true, "state": "met",
      "progress": 1.0, "detail": "summary.verdict == \"pass\"", "observed": "pass" }
  ]
}
```

The settled CLI panel prints a one-line summary:

```
goals 1/1 required met · no failing dimension ✓ · dimensions passed ✓ (opt)
```

---

## Try it

```bash
# happy path — all fake dimensions pass, both goals met
jesterky run examples/goal_quality_gate.json --actor fake --run-id goal-demo-001

# gate fails when a required goal can't be met (args overlay)
jesterky run examples/goal_quality_gate.json --actor fake \
  --args '{"goals":{"goals":[{"id":"impossible","kind":"metric_threshold","path":"summary.passed","min":999}]}}'
# -> status=failed · goals 0/1 required met
```

---

## Follow-ups

- `work_product_ready` / `signal_flag` goal kinds (see the `GoalKind` sketch in
  `jesterky_notes.md` §9).
- A dedicated goal line inside the live `--follow` panel (currently prints on the
  settled frame).
- Cooperatively cancelling items *already running* (today's cancel stops starting
  new items; in-flight items complete).
