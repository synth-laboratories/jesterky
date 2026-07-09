# Resource budgets, progress, and ETA

jesterky has **two different “limit” concepts**. This doc is only about
**resource budgets** (caps + progress + burn-rate ETA).

| Concept | Field | Meaning |
|---|---|---|
| **Concurrency limits** | `runplan.limits` | Semaphores (`permits`) — how many things run at once |
| **Resource budgets** | `runplan.budgets` | Caps on calls / tokens / wall time — how much you may *spend* |

Machine-readable schema: root `jesterky.schema.json` → definitions `BudgetPlan`,
`BudgetCap`, `BudgetEtaConfig`, `BudgetVizConfig`, `BudgetEtaMode`, `BudgetKind`.

---

## Typed interface (`jesterky-contract`)

All of these are `Serialize` / `Deserialize` / `JsonSchema` and re-exported from
the crate root.

| Type | Role |
|---|---|
| [`BudgetPlan`](../crates/jesterky-contract/src/budget.rs) | Full JSON config (`runplan.budgets`) |
| [`BudgetCap`](../crates/jesterky-contract/src/budget.rs) | One dimension (`kind`, `max`, `hard`, display flags) |
| [`BudgetKind`](../crates/jesterky-contract/src/budget.rs) | `actor_calls` \| `tokens` \| `wall_seconds` |
| [`BudgetEtaConfig`](../crates/jesterky-contract/src/budget.rs) | ETA engine knobs (`enabled`, `mode`, …) |
| [`BudgetEtaMode`](../crates/jesterky-contract/src/budget.rs) | `off` \| `nearest_only` \| `all` |
| [`BudgetVizConfig`](../crates/jesterky-contract/src/budget.rs) | Terminal panel knobs |
| [`BudgetEngine`](../crates/jesterky-contract/src/budget.rs) | Pure projection: plan + observations → snapshot |
| [`BudgetObservation`](../crates/jesterky-contract/src/budget.rs) | Host meter sample `(kind, t_secs, spent)` |
| [`BudgetSnapshot`](../crates/jesterky-contract/src/budget.rs) | Live + final result; stored on `RunManifest.budgets` |
| [`BudgetStatus`](../crates/jesterky-contract/src/budget.rs) | Per-cap progress + forecast |
| [`BudgetForecast`](../crates/jesterky-contract/src/budget.rs) | `seconds_to_limit`, rate, confidence |

Host **meters** (CLI samples actor calls, tokens, wall). Engine only **projects**.

```rust
use jesterky_contract::{BudgetEngine, BudgetPlan, BudgetObservation};

// Load / construct plan (from workflow JSON or code).
let plan: BudgetPlan = serde_json::from_value(spec.runplan.budgets_json)?;

// Partial override without losing caps (CLI `--args.budgets` uses this).
let plan = plan.overlay_json(&serde_json::json!({
    "eta": { "mode": "nearest_only" },
    "warning_percent": 70
}));

let snap = BudgetEngine::snapshot(run_id, &plan, &observations, wall_secs);
// snap.items[*].spent / .remaining / .used_percent / .state
// snap.items[*].forecast.seconds_to_limit  // ETA to exhaust that cap
// snap.nearest                             // soonest ETA
// snap.plan                                // echoed config for viz/consumers
```

Python (schema-generated pydantic): `from jesterky.spec import BudgetPlan, BudgetCap, …`.

---

## JSON shape (`runplan.budgets`)

Every knob below is optional except each cap’s `kind` + `max`. Defaults match
the Rust `Default` impls.

```json
{
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
        {
          "kind": "actor_calls",
          "max": 8,
          "hard": true,
          "label": "turns",
          "warning_percent": 90,
          "show_progress": true,
          "show_eta": true
        },
        { "kind": "tokens", "max": 100000, "hard": false },
        { "kind": "wall_seconds", "max": 120, "hard": false }
      ]
    }
  }
}
```

### Fields

#### Plan-level (`BudgetPlan`)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `caps` | `BudgetCap[]` | `[]` | Declared caps (empty ⇒ budgets off) |
| `warning_percent` | number | `80` | Used % that flips state → `warning` |
| `fail_on_hard_exhaust` | bool | `true` | If any `hard` cap is exhausted, mark run `failed` |
| `eta` | `BudgetEtaConfig` | see below | Forecast / sampling knobs |
| `viz` | `BudgetVizConfig` | see below | Panel knobs |

#### Cap (`caps[]` → `BudgetCap`)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `kind` | enum | **required** | `actor_calls` \| `tokens` \| `wall_seconds` |
| `max` | number | **required** | Ceiling (`> 0`) |
| `hard` | bool | `false` | Soft vs hard (with `fail_on_hard_exhaust`) |
| `label` | string? | kind name | Display name (e.g. `"turns"`) |
| `warning_percent` | number? | plan default | Per-cap warn threshold |
| `show_progress` | bool | `true` | Include on progress line |
| `show_eta` | bool | `true` | Include on ETA line |

#### `eta` (`BudgetEtaConfig`)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Master switch for forecasts |
| `mode` | enum | `all` | `off` \| `nearest_only` \| `all` |
| `min_wall_secs` | number | `1.0` | Min wall before burn-rate ETA is trusted |
| `sample_interval_secs` | number | `0.45` | Min gap between history samples used for burn-rate (not the panel redraw rate) |

#### `viz` (`BudgetVizConfig`)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Draw budget lines at all |
| `show_progress` | bool | `true` | Progress line |
| `show_eta` | bool | `true` | ETA line |
| `show_nearest_tag` | bool | `true` | Append `nearest <kind>` |
| `short_labels` | bool | `true` | `calls`/`tok`/`wall` vs full names |

---

## Common recipes

| Goal | Config |
|---|---|
| **Episode ETA** (ETA ≈ run end) | Cap `actor_calls.max` ≈ `max_turns`; see `examples/budgets_episode_scale.json` |
| **Campaign budget** (long ceiling) | High `max` (e.g. 64 calls / 500k tok); ETA is time-to-cap, not time-to-episode |
| **Only soonest ETA** | `"eta": { "mode": "nearest_only" }` |
| **Progress only, no ETA** | `"eta": { "enabled": false }` or `"viz": { "show_eta": false }` |
| **Hide one dim from ETA** | Cap-level `"show_eta": false` (still meters + progress) |
| **Custom labels** | Cap `"label": "turns"` |
| **Warn earlier on tokens** | Cap `"warning_percent": 50` or plan-level `warning_percent` |
| **Soft ceiling** | `"hard": false` (warn/exhaust state, run still completes) |
| **Hard stop** | `"hard": true` + plan `"fail_on_hard_exhaust": true` |

---

## Semantics: progress vs ETA

| | Progress | ETA |
|---|---|---|
| **Question** | How full is the tank? | When do we hit empty *at this burn rate*? |
| **Formula** | `spent/max`, `%`, state | `(max - spent) / rate` → `seconds_to_limit` |
| **Not** | — | Time until *this workflow* finishes |

### Making ETA ≈ “when does this run end?”

Set caps near the **episode size**:

```json
"caps": [
  { "kind": "actor_calls", "max": 8, "hard": true, "label": "turns" }
]
```

with `"max_turns": 8` on a DungeonGrid args payload.

If you set `"max": 64` and only run 8 turns, ETA stays ~minutes (time to burn the
*remaining* 56 calls) and moves slowly — that is correct for a **campaign
budget**, not an episode timer.

---

## Per-run override (`--args.budgets`)

CLI **deep-merges** `args.budgets` onto the workflow’s `runplan.budgets` via
[`BudgetPlan::overlay_json`](../crates/jesterky-contract/src/budget.rs):

- Nested objects (`eta`, `viz`) merge field-by-field.
- `caps` **replaces** when present (not element-wise).
- Omitted fields keep the workflow defaults.

**Partial override** (keep caps, change ETA mode):

```bash
cargo run -p jesterky-cli -- run examples/dungeongrid_4p.json --actor fake \
  --args '{"budgets":{"eta":{"mode":"nearest_only"},"warning_percent":70}}'
```

**Replace caps** (episode-scale ETA):

```bash
cargo run -p jesterky-cli -- run examples/dungeongrid_4p.json --actor fake \
  --args "$(jq -n --slurpfile b examples/budgets_episode_scale.json \
    '{budgets:$b[0], max_turns:8, party_size:4}')"
```

Or inline:

```bash
--args '{
  "budgets": {
    "warning_percent": 70,
    "eta": { "mode": "nearest_only" },
    "caps": [
      { "kind": "actor_calls", "max": 16, "hard": false, "label": "turns" }
    ]
  }
}'
```

---

## Panel

```text
budget  turns 4/8 (50%) · tok 20k/100k (20%) · wall 16s/2m00s (13%)
ETA     turns 18s · tok 1m12s · wall 1m44s  nearest turns
```

Instant fake runs (`wall < min_wall_secs`) show:

```text
ETA     — (need >1s wall for burn-rate)
```

### Two “interval” knobs

| Knob | Where | Controls |
|---|---|---|
| `eta.sample_interval_secs` | workflow JSON | Min wall gap between budget **history samples** used for burn-rate |
| `--viz-interval` | CLI flag (default `0.2`) | Live panel **redraw** cadence |

---

## Metering (host / CLI)

| Kind | Metered from |
|---|---|
| `actor_calls` | `ActorInvoked` events / recorded actor calls |
| `tokens` | Live shard progress (`--follow` + real model); 0 under `--actor fake` |
| `wall_seconds` | Host wall clock since run start |

---

## Snapshot on the manifest

Finished runs attach `RunManifest.budgets` (`BudgetSnapshot`, schema version
`budget_engine.v1`): echoed `plan`, per-item `BudgetStatus` (progress + forecast),
and `nearest`. Live follow uses the same projection each frame.

---

## Examples

- **DungeonGrid** (`examples/dungeongrid_4p.json`) — full plan with campaign-scale defaults.
- **Episode-scale template** (`examples/budgets_episode_scale.json`) — drop-in `budgets` block.
- Any workflow: add `runplan.budgets` or pass `--args.budgets`.

See also: optimizers `LimitEngine` (forecast pattern), SMR
`progress-toward-resource-limits` (progress pattern).
