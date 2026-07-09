# Handoff — Limit engine (landed) → Goals / work engine (next)

**Date:** 2026-07-08  
**Branch context:** `feature/jesterky-terminal-viz` (+ OBLIQ + budgets work)  
**Audience:** next agent / self — continue the “resource & outcome control planes” line without re-deriving intent from chat.

**Related:**

| Doc | Role |
|---|---|
| `docs/BUDGETS.md` | Resource budgets JSON + typed API (shipped) |
| `docs/OBLIQ.md` | OBLIQ-Bench workload + difficulty ladder (shipped) |
| `HANDOFF_jesterky_terminal_viz.md` | Live panel / follow |
| `HANDOFF_jesterky_release_and_blog_plan.md` | Release + optimizer surface |
| optimizers `LimitEngine` / SMR progress-toward-limits | Pattern ancestors for budgets |

---

## 1. Intentions (owner voice, preserved)

### 1.1 Product shape

jesterky should feel like a **workflow runtime with real control planes**, not just a DAG executor:

1. **Topology** — what runs (nodes, maps, while, sessions).  
2. **Limits** — how hard we push (concurrency + resource spend + ETA).  
3. **Goals / work** — what “done well” means (scoped work products, success signals, early wrap-up).  

(1) is largely locked.  
(2) landed this cycle as formal **resource budgets** + live progress/ETA.  
(3) is the **next engine** — dual of (2): *progress toward goals*, not only *progress toward exhaustion*.

### 1.2 Why this matters

- Long agent runs are not “run the graph until the entrypoint ends.” Users care about:
  - **Will I hit the token/wall cap?** (limit / budget engine — *shipped*)
  - **Have I already produced the thing I wanted?** (goals / work engine — *not shipped*)
- OBLIQ-style evals, artifact-generation workflows, and “search until found” all need **semantic termination**, not only structural completion.
- Optimizers (GEPA/GELO/MAPO) need clean **outcome surfaces**: score/signal today; goal snapshots tomorrow.

### 1.3 Non-goals (keep Chinese walls)

- Goals grade **outcomes / work products**, not “was the model smart.”  
- No JS/eval DSL in workflow JSON — conditions stay **ledger refs + pure programs**, or typed goal predicates later.  
- Do not conflate **concurrency limits**, **resource budgets**, and **goal success** into one blob.

---

## 2. Three control planes (vocabulary — keep distinct)

| Plane | Field / surface | Question | Status |
|---|---|---|---|
| **Concurrency limits** | `runplan.limits` → semaphores | How many things run at once? | ✅ core (`Limit` / `limits.rs`) |
| **Resource budgets** | `runplan.budgets` → `BudgetEngine` | How much may we *spend*? When empty? | ✅ contract + CLI + viz |
| **Goals / work** | proposed `runplan.goals` / work engine | Have we *achieved* the scoped product? Can we stop? | ❌ **next** |

Naming note: “limit engine” in conversation has meant two things:

1. **Optimizers / SMR `LimitEngine`** — forecast progress-toward-resource-limits (pattern we mirrored).  
2. **jesterky resource budgets** — the OSS formalization (`BudgetPlan` / `BudgetEngine`).  

Concurrency semaphores are a third, older, smaller thing. Prefer saying **budget engine** for (2) and **goals/work engine** for the new plane.

---

## 3. Budget / limit engine — what landed

### 3.1 Typed surface (`jesterky-contract`)

Source of truth: `crates/jesterky-contract/src/budget.rs`  
Docs: `docs/BUDGETS.md`  
Schema: `jesterky.schema.json` definitions `BudgetPlan`, `BudgetCap`, `BudgetEtaConfig`, `BudgetVizConfig`, …  
Python: regenerated via `python/gen.sh`.

| Type | Role |
|---|---|
| `BudgetPlan` | Full JSON config on `runplan.budgets` |
| `BudgetCap` | One dim: `kind`, `max`, `hard`, `label`, `warning_percent`, `show_*` |
| `BudgetKind` | `actor_calls` \| `tokens` \| `wall_seconds` |
| `BudgetEtaConfig` / `BudgetEtaMode` | Forecast knobs (`off` / `nearest_only` / `all`) |
| `BudgetVizConfig` | Panel lines (progress / ETA / nearest / short labels) |
| `BudgetEngine::snapshot` | Pure: plan + observations → `BudgetSnapshot` |
| `BudgetPlan::overlay_json` | Deep-merge for `--args.budgets` partial overrides |
| `RunManifest.budgets` | Final + live snapshot (`budget_engine.v1`) |

**Host meters; engine only projects.** CLI samples actor calls / tokens / wall and calls `BudgetEngine::snapshot`.

### 3.2 Semantics (easy to get wrong)

| Surface | Meaning |
|---|---|
| **Progress** | `spent/max`, `%`, state (`ok` / `warning` / `exhausted`) |
| **ETA** | Estimated **time until that cap is exhausted** at current burn rate — **not** time until the workflow finishes |

Episode-scale ETA ⇒ set caps near episode size (e.g. `actor_calls.max ≈ max_turns`).  
Campaign-scale caps (e.g. 64 calls for an 8-turn run) correctly show long, slow ETAs.

### 3.3 Highly configurable in workflow JSON

Example knobs (all optional with defaults):

```json
"runplan": {
  "budgets": {
    "warning_percent": 80,
    "fail_on_hard_exhaust": true,
    "eta": {
      "enabled": true,
      "mode": "all",
      "min_wall_secs": 1.0,
      "sample_interval_secs": 0.45
    },
    "viz": {
      "enabled": true,
      "show_progress": true,
      "show_eta": true,
      "show_nearest_tag": true,
      "short_labels": true
    },
    "caps": [
      { "kind": "actor_calls", "max": 20, "hard": false, "label": "calls" },
      { "kind": "tokens", "max": 400000, "hard": false },
      { "kind": "wall_seconds", "max": 900, "hard": false }
    ]
  }
}
```

Per-run partial override (deep merge, caps array replaces if present):

```bash
--args '{"budgets":{"eta":{"mode":"nearest_only"},"warning_percent":70}}'
```

### 3.4 Viz wiring

- Live panel: two lines when enabled — progress + ETA (`crates/jesterky-actor/src/viz.rs` `budget_lines`).  
- Config read from `snap.plan.viz` / `snap.plan.eta` (echoed plan on snapshot).  
- Dual interval: `eta.sample_interval_secs` = budget history sample gap; `--viz-interval` = redraw cadence.

### 3.5 CLI / host enforcement

- `resolve_budget_plan` → `BudgetPlan::overlay_json`  
- `project_budgets` mid-follow + post-run  
- Optional `fail_on_hard_exhaust` → `RunStatus::Failed` when a **hard** cap is exhausted  
- Soft caps only warn / show state

### 3.6 Alignment with optimizers / SMR

Intentionally shaped like:

- optimizers **`LimitEngine`** — burn-rate / seconds-to-limit forecasts  
- SMR **progress-toward-resource-limits** — utilization + nearest limit  

jesterky’s OSS version is **declarative in workflow JSON**, pure engine, host-metered, panel + manifest.

### 3.7 Intention notes for budgets (carry forward)

- Keep budgets **highly configurable** and **well typed** (schemars + docs + Python).  
- Do not invent “time-to-workflow-end” without a separate goal engine — ETA stays **time-to-cap**.  
- Future budget kinds (cost USD, tool calls, env steps) should extend `BudgetKind` carefully (contract bump).  
- Episode-scale template: `examples/budgets_episode_scale.json`.

---

## 4. Workloads & measured evidence (context for goals)

### 4.1 OBLIQ-Bench (new workload)

| Piece | Path |
|---|---|
| Programs / roles | `crates/jesterky-quality/src/obliq.rs` |
| Spec | `examples/obliq_math_verify.json` |
| Schemas | `examples/obliq_rank.schema.json`, `obliq_metrics.schema.json` |
| Docs | `docs/OBLIQ.md` |
| Data | `data/obliq-bench/math/` (gitignored; HF `dianetc/OBLIQ-Bench`) |

**Difficulty ladder (`mode`):**

| Mode | Meaning | Difficulty |
|---|---|---|
| `verify` | Gold-infused pool + random distractors | Easy (ceiling) |
| `hard_verify` | **1 gold** + lexical near-miss distractors | Harder needle |
| `retrieve` | Pure lexical top-k, **no gold infuse** | First-stage bottleneck |
| `retrieve_hard` | Only queries with 0 lexical@pool golds | Hardest |

### 4.2 Flash vs pro (same seed=7, n=12, pool=16, k=10)

| Mode | flash nDCG@10 | pro nDCG@10 | Separation |
|---|---|---|---|
| `verify` | ~0.99 | ~0.99 | none (ceiling) |
| **`hard_verify`** | **0.75** | **0.95** | **clear pro win (+0.20)** |
| **`retrieve`** | **0.09** | **0.13** | almost none (first-stage) |

Proof artifacts under `proof/obliq_math_*.manifest.json`.

**Reading (matches paper):** verification is easy once golds are in the pool; pure retrieve fails because lexical first-stage rarely surfaces golds. Pro separates on hard needle ranking, not on empty pools.

### 4.2 Soft “verdict” is not run termination

OBLIQ aggregate / recorder emit `verdict: pass|fail` and metrics in JSON.  
That does **not** set `RunStatus` or stop the entrypoint early.  
Semantic success today is **payload-only**.

---

## 5. How “done” works today (structural only)

```text
done (engine)  ⇔  entrypoint nodes finished without error
                  + map min_success gates
success (engine) ⇔  RunStatus::Completed
success (semantic) ⇔  whatever you put in ledger / summary JSON
                      (core does not interpret)
```

### 5.1 Existing knobs (not a goals engine)

| Mechanism | What it does | Limitation |
|---|---|---|
| `entrypoint` order | Sequential graph finish | No “goal achieved → stop” |
| `map.min_success` | Fraction of shards that must succeed | Failure gate, not success goal |
| `while` + `cond` + `max_iters` | Loop until ledger ref falsy | DIY only; no parallel cancel |
| `branch` | Conditional path | Not global wrap-up |
| Budgets hard exhaust | Host may mark `Failed` | Stop for *spend*, not *achievement* |
| Actor `score` / `signal` | Optimizer slots on recorded outputs | Not termination signals |
| Output schemas | JSON shape | Not goals |

### 5.2 DIY early-stop pattern (document, don’t pretend it’s first-class)

```text
init → while(not ledger.done, max_iters) { step; maybe set done } → finalize
```

- Works for serial “search until found” / “generate until good enough.”  
- Does **not** cancel in-flight map siblings.  
- Condition is a **ledger truthy value**, not a typed goal predicate.  
- No first-class **scoped work product** as completion unit.

### 5.3 Owner pain points that motivate the goals/work engine

1. **OBLIQ / search:** “If we find the answer early, wrap up — don’t burn the rest of the map.”  
2. **Artifact gen:** “Once the scoped work product is deemed achieved, finalize and stop.”  
3. **Operator UX:** progress should show **toward the goal**, not only **toward the token wall**.  
4. **Optimizers:** need a stable “goal snapshot” analogous to `BudgetSnapshot`.

---

## 6. Goals / work engine — proposed next product

Treat this as the **dual of the budget engine**:

| Budget engine | Goals / work engine |
|---|---|
| Caps on *spend* | Targets on *achievement* |
| Progress: spent / max | Progress: work done / goal criteria |
| ETA: time-to-exhaust | ETA: time-to-goal (optional, burn-rate toward criteria) |
| Exhausted → optional fail | Met → optional **successful early terminate** + finalize |
| `BudgetSnapshot` on manifest | `GoalSnapshot` / `WorkSnapshot` on manifest |
| Host meters resources | Host / programs **evaluate** goal predicates or attach work products |

### 6.1 Concepts (draft vocabulary)

| Term | Meaning |
|---|---|
| **Scoped work product** | Named deliverable for the run (artifact ref, ledger key, schema-validated JSON, file path, metric threshold, …) |
| **Goal** | Predicate or checklist over work products + optional scores |
| **Terminate signal** | Engine-honored “goal met (or failed irrevocably) → stop orchestration, run finalize path” |
| **Goal progress** | Partial credit: checklist %, best score so far, shards that found hits |
| **Finalize path** | Optional node id / subgraph to run on success wrap-up |

### 6.2 Desired behaviors (acceptance stories)

1. **Early success wrap-up**  
   OBLIQ search loop / artifact gen: when goal is met, remaining map work is cancelled or skipped; finalize runs; status = `Completed` with `goal.state = met`.

2. **Hard fail goal**  
   “Must produce X with schema Y”; if max budget or max iters hit without X → `Failed` with reason `goal_unmet` (distinct from topology error).

3. **Soft goal**  
   Record progress and verdict; do **not** alter control flow (today’s OBLIQ `verdict` behavior — explicit mode).

4. **Panel**  
   ```text
   goal    answer found 1/1 · work product artifact/obliq-hit.json ✓
   budget  calls 4/20 (20%) · tok 40k/400k
   ETA     calls 2m · tok 8m   nearest calls
   ```

5. **JSON-configurable, typed, documented** — same bar as budgets (`docs/GOALS.md`, schemars, overlay, examples).

### 6.3 Sketch — typed interface (not implemented)

Illustrative only; final shapes need an ADR + contract bump:

```rust
// crates/jesterky-contract/src/goal.rs  (proposed)

pub struct GoalPlan {
    pub goals: Vec<GoalSpec>,
    /// On first met goal (or all required): stop orchestration after finalize.
    pub terminate_on_met: bool,
    pub finalize: Option<String>, // node id
    pub viz: GoalVizConfig,
}

pub struct GoalSpec {
    pub id: String,
    pub required: bool,
    /// How to evaluate — start narrow, expand carefully.
    pub kind: GoalKind,
    pub work_product: Option<WorkProductSpec>,
}

pub enum GoalKind {
    /// Ledger path truthy / equals value
    LedgerPred { path: String, equals: Option<serde_json::Value> },
    /// Named work product present + optional schema
    WorkProductReady { work_product_id: String },
    /// Metric on ledger or last reduce: path >= threshold
    MetricThreshold { path: String, min: f64 },
    /// Actor/process signal: signal.goal == "met"
    SignalFlag { key: String },
}

pub struct WorkProductSpec {
    pub id: String,
    pub source: WorkProductSource, // ledger path | artifact key | file path
    pub schema: Option<String>,    // JSON Schema path
    pub description: Option<String>,
}

pub struct GoalSnapshot {
    pub schema_version: String, // "goal_engine.v1"
    pub plan: GoalPlan,
    pub items: Vec<GoalStatus>,
    pub state: GoalState, // unmet | partial | met | failed
    pub terminated_early: bool,
}
```

Workflow JSON sketch:

```json
"runplan": {
  "budgets": { "...": "..." },
  "goals": {
    "terminate_on_met": true,
    "finalize": "finalize",
    "goals": [
      {
        "id": "hit",
        "required": true,
        "kind": "metric_threshold",
        "path": "ledger.summary.mean_ndcg_at_k",
        "min": 0.9
      },
      {
        "id": "answer_blob",
        "required": false,
        "kind": "work_product_ready",
        "work_product": {
          "id": "answer",
          "source": { "ledger": "ledger.answer" },
          "schema": "answer.schema.json"
        }
      }
    ]
  }
}
```

### 6.4 Engine vs runner responsibilities

| Layer | Responsibility |
|---|---|
| **GoalEngine** (pure, contract) | plan + observations/evaluations → `GoalSnapshot` |
| **Host / programs** | Produce work products; write ledger; optional evaluators |
| **Runner** | Honor terminate signal: cancel/skip remaining entrypoint work; run finalize; set status; emit events (`GoalMet`, `GoalFailed`, `WorkflowCompleted`) |
| **CLI / viz** | Meter + display goal lines next to budget lines |

**Hard design choices (decide in ADR, not casually):**

1. **Cancel in-flight map items?** (required for true early stop; needs cooperative cancel on actor/codex).  
2. **When to evaluate goals?** (after each node / each map item / only after reduces).  
3. **Required vs optional goals** for overall `met`.  
4. **Interaction with budgets:** goal met + budget ok → success; budget hard exhaust + goal unmet → fail; both race.  
5. **Replay:** goal evaluations must be deterministic from recorded outputs + pure predicates.

### 6.5 Minimal vertical slice (recommended first PR)

1. Contract: `GoalPlan` / `GoalSnapshot` / `GoalEngine` with **ledger predicate + metric threshold only**.  
2. Runner: after each top-level entrypoint node (or after map complete), evaluate; if `terminate_on_met` and required goals met → skip remaining entrypoint nodes, optionally run `finalize` node id.  
3. **No** in-flight cancel yet — only skip *not-yet-started* entrypoint siblings. Document that parallel map still runs to completion within the current node.  
4. CLI: attach `manifest.goals`; viz one goal line.  
5. Example: toy “search until found” while **or** OBLIQ serial step with early stop.  
6. Docs: `docs/GOALS.md` mirror of `docs/BUDGETS.md`.

Stretch (second PR): work product specs + schema validation; map-item-level evaluation; cooperative cancel.

### 6.6 Explicit non-goals for v1

- Free-form expression language in JSON  
- LLM-as-judge as the only goal evaluator without a pure fallback  
- Replacing `min_success` / `while` (compose with them)  
- Claiming “time-to-goal ETA” before goal progress is stable

---

## 7. Architecture diagram (target)

```text
                    ┌─────────────────────────────┐
                    │        WorkflowSpec         │
                    │  topology + runplan         │
                    │   limits | budgets | goals  │
                    └─────────────┬───────────────┘
                                  │
              ┌───────────────────┼───────────────────┐
              ▼                   ▼                   ▼
     Concurrency semaphores  BudgetEngine       GoalEngine
     (core limits.rs)        (contract pure)    (contract pure)
              │                   │                   │
              │            host meters           host/programs
              │            observations          evaluate / attach
              │                   │                   │
              └───────────────────┼───────────────────┘
                                  ▼
                            Runner control
                     start → step → (budget fail?)
                                   (goal met? → finalize + stop)
                                  ▼
                         RunManifest
                    events + recorded + budgets + goals
                                  ▼
                         Viz / optimizers / CLI
```

---

## 8. Files map (current)

### Budgets / limits

| Path | Notes |
|---|---|
| `crates/jesterky-contract/src/budget.rs` | Budget plan + engine |
| `crates/jesterky-contract/src/topology.rs` | `RunPlan.budgets`, `RunPlan.limits` |
| `crates/jesterky-contract/src/artifact.rs` | `RunManifest.budgets` |
| `crates/jesterky-cli/src/main.rs` | resolve/project budgets, hard exhaust |
| `crates/jesterky-actor/src/viz.rs` | budget panel lines |
| `crates/jesterky-core/src/limits.rs` | concurrency only |
| `docs/BUDGETS.md` | user-facing budget docs |
| `examples/budgets_episode_scale.json` | episode ETA template |

### OBLIQ

| Path | Notes |
|---|---|
| `crates/jesterky-quality/src/obliq.rs` | expand / aggregate / roles |
| `examples/obliq_math_verify.json` | workflow |
| `docs/OBLIQ.md` | modes + measured results |
| `proof/obliq_math_*.manifest.json` | flash/pro × verify/hard/retrieve |

### Goals / work

| Path | Notes |
|---|---|
| *(none yet)* | This handoff is the charter |

---

## 9. Suggested next steps (ordered)

1. **ADR:** Goals/work engine vs budgets vs concurrency (three planes).  
2. **Contract draft** in `goal.rs` + schema emit + empty default on `RunPlan`.  
3. **Minimal GoalEngine** + unit tests (ledger pred + metric threshold).  
4. **Runner hook** post-node evaluation + skip rest of entrypoint + optional finalize.  
5. **CLI attach** `manifest.goals`; **viz** goal line.  
6. **Example** serial early-stop search (OBLIQ or toy).  
7. **Docs** `docs/GOALS.md` + README pointer.  
8. Later: work products, map-item evaluation, cancel in-flight actors, time-to-goal ETA.

---

## 10. Open questions for the owner

1. Is **successful early terminate** more important than **rich work product schemas** for v1? (Recommend: early terminate first.)  
2. Should unmet required goals after full entrypoint force `Failed`, or only `Completed` + `goal.state=unmet`?  
3. Do we want optimizers to **target** `GoalSnapshot` the same way they might target budget efficiency?  
4. Scope of cancel: entrypoint-skip only (v1) vs full cooperative cancel (v2)?  
5. Keep OBLIQ soft `verdict` as `terminate_on_met: false` default for eval workflows?

---

## 11. One-paragraph summary

We formalized a **resource budget / limit engine** (typed JSON, pure `BudgetEngine`, live progress + time-to-cap ETA, host metering) as the OSS cousin of optimizers’ LimitEngine / SMR progress-toward-limits. We proved it on real runs (DungeonGrid, OBLIQ). We also learned that **completion is still structural**: graphs finish when entrypoints finish; soft metrics do not wrap up work early. The intentional next product is the **goals / work engine** — dual of budgets — for scoped work products, goal progress, and a **terminate signal** that successfully finalizes a run when the goal is met (e.g. OBLIQ finds the answer early; artifact gen deems the product done). Build it with the same bar as budgets: highly configurable JSON, clear typed interface, pure engine, host evaluation, docs + schema, panel + manifest snapshot.

---

*End of handoff. Prefer this file + `docs/BUDGETS.md` + `docs/OBLIQ.md` over chat scrollback.*
