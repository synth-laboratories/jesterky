# jesterky notes (full)

**Date:** 2026-07-08  
**Purpose:** single durable notes file covering product status, core architecture, limit/budget engine, goals/work product engine, code quality, data/LLM systems patterns, optimizers/Stack/SMR integration, OSS ship, evals, and what to do better.  
**Sources:** this repo (code + handoffs + proof), `mloky` roadmap/status, `optimizers` platform crates, `workflows/BUILD.md`, Mintlify drafts under sibling `docs`.

**Head (when reviewed):** `f39d8da` on `feature/jesterky-terminal-viz` plus a large dirty tree (budgets, follow, DungeonGrid, OBLIQ, GEPA/GELO, blog/docs scans uncommitted).

---

# Table of contents

1. [What jesterky is](#1-what-jesterky-is)  
2. [Status scorecard](#2-status-scorecard)  
3. [Operator critical path & OSS ship](#3-operator-critical-path--oss-ship)  
4. [Core architecture](#4-core-architecture)  
5. [Event stream (shared? SSE?)](#5-event-stream-shared-sse)  
6. [Typing of inputs/outputs](#6-typing-of-inputsoutputs)  
7. [Three control planes](#7-three-control-planes)  
8. [Limit / budget engine (detail)](#8-limit--budget-engine-detail)  
9. [Goals / work product engine (next)](#9-goals--work-product-engine-next)  
10. [What should be done better (code & systems)](#10-what-should-be-done-better-code--systems)  
11. [Data systems patterns](#11-data-systems-patterns)  
12. [LLM systems patterns](#12-llm-systems-patterns)  
13. [Strong patterns from optimizers code](#13-strong-patterns-from-optimizers-code)  
14. [Adding jesterky to optimizers (GEPA/GELO/MAPO)](#14-adding-jesterky-to-optimizers-gepagelomapo)  
15. [Stack integration (M3)](#15-stack-integration-m3)  
16. [Hosted Cloud + synth-ai (M4)](#16-hosted-cloud--synth-ai-m4)  
17. [SMR + ReportBench](#17-smr--reportbench)  
18. [Workloads, evals, proof packet](#18-workloads-evals-proof-packet)  
19. [Testing & E2E](#19-testing--e2e)  
20. [Content / launch L3](#20-content--launch-l3)  
21. [Master checklist](#21-master-checklist)  
22. [Doc index](#22-doc-index)  
23. [Bottom line](#23-bottom-line)

---

# 1. What jesterky is

Pinned Rust workflow substrate for the Synth stack (*ještěrky* = lizards: they regrow tails → replay/resume). Supersedes `workflow-rs` / `rust_backend/graph` (mines them for ideas; does not fork them).

**Product claim (from workflows BUILD):** define, run, visualize, evaluate, compare, and optimize agent workflows as first-class artifacts.

**Discipline:** one pinned contract across OSS, Stack, Cloud, and optimizers. The contract is the product.

### Product ladder (roadmap)

```text
M0 contract ──┬── M1 core ──┬── M2 direct workflows (CLI) ──┬── M5a optimizer traces ★ FIRST APP
              │             │                               │     (GEPA/GELO annotate)
              │             │                               ├── M3 Stack
              │             │                               ├── M4 Hosted prod ──┬── M5 product ── M6 loop
              │             └── ★ Content (blog + Mintlify + proof) at M2
```

Operator order (2026-07-08): **1 GEPA/GELO test → 2 DungeonGrid → 3 blog → 4 optimizers prod → 5 hosted Cloud → 6 synth-ai SDK → other (Stack cockpit, goals engine, viz polish).**

### Crates

| Crate | Role |
|---|---|
| `jesterky-contract` | Topology, events, artifacts/manifest, budgets; schemars → JSON Schema; Python types codegen |
| `jesterky-core` | Pure orchestration; zero IO |
| `jesterky-actor` | Host SDK: replay, fakes, mem sinks, viz |
| `jesterky-model` | `ModelActor` + `CodexModel` (codex exec, ChatGPT-bundle / proxy) |
| `jesterky-quality` | Workloads: quality scan, blog/docs, DungeonGrid, OBLIQ, GEPA/GELO trace |
| `jesterky-cli` | `run` / `replay` / `validate` / `schema` / `visualize` (+ `--follow` in WIP) |

### Three version trains

1. **contract** — `jesterky-contract` (Rust + PyPI types)  
2. **runtime** — core, CLI, actor, model, quality  
3. **client** — `synth-ai` HTTP (M4)

**Decided:** no pyo3. Python = contract types only. Local run = Rust CLI. Hosted = Rust service (M4). No arbitrary JS embedded in workflow JSON (topology combinators + registered Rust program ops).

### The four contracts inside `jesterky-contract`

| Contract | What it pins | Primary consumers |
|---|---|---|
| Topology | How a workflow is declared | Authors, core |
| Event stream | What a run emits | Viz, replay, Stack, optimizers |
| Artifact / manifest | Run record, process tree, recorded I/O | Replay, optimizers |
| Replay/exec semantics | Determinism + resume guarantees | Cloud, replay, optimizers |

Event + artifact schema must stay **optimizer-first** (typed process tree, score/signal slots) or M5/M6 re-cut the contract.

---

# 2. Status scorecard

| Area | Grade | Notes |
|---|---|---|
| Core / contract / replay | A− | Addr clock + pure/impure split solid |
| OSS packages | B+ | 6 crates + PyPI @0.1.0 live; GitHub held; no CI |
| Live E2E evidence packet | C | Proven once; live manifests not fully committed; hollow-scan risk |
| mloky parity | C+ | Outcome layer only (conservation + termination) |
| Limit / budget engine | B+ | Typed + proven on runs; WIP not published |
| Goals / work engine | A− | v1 + v2 shipped: `GoalEngine`, early-terminate, fail-on-unmet, `finalize` execution, in-flight map cancel. Foundational core "up to optimizer" COMPLETE (process-tree I/O, output-schema validation, invariant report, budget reserve, stop-reason); 96 tests. WIP uncommitted. |
| Workloads (WIP tree) | B | Breadth high; platform integration low |
| Evals (OBLIQ) | B | Real flash/pro separation; not productized/CI |
| Optimizer / GEPA-GELO ship | B− | GEPA Craftax prompt ablation now powered at paired n=64; GELO dropped until a real non-seed GELO prompt exists; optimizer hooks still need prod/OSS promotion |
| Stack / hosted / SDK | C− | 5 verbs + hosted runner + SDK wired, contract-correct on inspection; unrun, untested |
| SMR ReportBench example | C | Spec authored + validates + real map-reduce; never run; scorer unwired |
| Public story (blog live) | C+ | Draft updated with real GEPA evidence; private GitHub + L3 + live quality proof + SMR waiver/runner still owed |

**Honest posture:** *OSS substrate 0.1.0 published; interesting workloads proven locally in a dirty branch; not a public product launch; not optimizer-, Stack-, or SMR-integrated.*

### Done ✅

- M0 contract — schema, drift guard, crates.io + PyPI types @0.1.0  
- M1 runtime — fake quality scan, replay, CLI  
- M2 live scan — DeepSeek proxy, concurrency 4, replay ok (operator-proven)  
- mloky parity gate — outcome-layer `conformance.rs`  
- Fake E2E — `proof/quality_min.manifest.json`  
- Blog draft — `blog/jesterky-launch.md` (`status: draft`)  
- Changelog — `CHANGELOG.md` @0.1.0  
- `kill_on_drop` on codex subprocess (M2 orphan DoD partial)  

### WIP (uncommitted when reviewed)

- Resource budgets + ETA (`BudgetEngine`, `docs/BUDGETS.md`)  
- Live `--follow` + btop panel  
- DungeonGrid 4p LLM path + proof  
- OBLIQ-Bench math modes + flash/pro proofs  
- GEPA/GELO trace annotate specs + Craftax proof packet maintenance  
- Blog/docs quality scan hosts  

### Still open / gated

- Live blog quality proof (`proof/quality_scan_blogs.live.manifest.json`)  
- SMR ReportBench with/without ablation runner or explicit blog waiver  
- Real non-seed GELO prompt, if GELO is to make any uplift claim  
- Optimizer hooks promoted to prod/OSS defaults  
- Public `github.com/jesterky`  
- Trusted publishing / GH release assets  

---

# 3. Operator critical path & OSS ship

### Status (07-08): substrate DONE, integrations WIRED, evidence ledger active

Core is complete + tested (96 tests): contract, runtime, replay, budgets, goals
v1+v2, process-tree I/O, output-schema validation, invariant report, stop-reason,
events-out. Integrations authored in consumer repos and **contract-correct on
inspection** (one-way dep clean): optimizer read-model (`optimizers/.../jesterky.rs`),
Stack 5 verbs (`stack/src/jesterky.ts`), hosted runner + SDK (`backend/.../jesterky_runner.py`,
`synth-ai/.../workflows.py`), SMR example (`examples/smr_reportbench_trace_evaluate.json`).
GEPA-Craftax now has a real paired n=64 ablation result; SMR ReportBench and live
blog quality proof remain the main evidence gaps. Full end-to-end plan:
`HANDOFF_jesterky_integrations.md`.

### NORTH STAR (Josh, 07-08, DUE TONIGHT)

**Ship the release blogpost AND the initial public version of jesterky workflows
for GEPA, GELO, Stack, and SMR — exposed via MCP to workers.** Gated on the
ablation proofs below. Deadline: tonight, Jul 8.

**Tonight-scoped reality:** a defensible ablation is days of eval work; tonight =
**minimum-viable-real**. Get REAL end-to-end runs with REAL uplift numbers even at
small n (label n honestly — "n=8, directional" beats a fake number), wire the MCP
workflow exposure so workers can invoke each, and assemble the blog draft around
whatever real numbers land. Cuts allowed tonight: small n, fake-actor where the
target still measures, one ablation (GEPA-Craftax) as the headline + the others
as "wired + first run" rather than full ablations if time runs out. NEVER fake a
number; a smaller honest result ships, a fabricated one does not.

**Deliverable = blog draft + `workflows` MCP tools live for gepa/gelo/stack/smr +
≥1 committed real ablation table (GEPA-Craftax headline).**

### Release plan — FINAL (ablation bar)

**The bar:** a compelling use case = a two-arm experiment, not a single run.
Named **target quantity** · **Arm A (without / ablation)** · **Arm B (with)** ·
**uplift = B−A with n** (+ significance, LW-style Spearman/permutation) · fair
ablation (arms differ ONLY in the intervention: same seeds, same n, same target).
A run that produces a manifest is only the plumbing gate; the blog content is the
uplift table committed to `proof/`.

The compelling core = THREE uplift ablations. Example 1 (quality scan) is a
substrate demo (runs live), not an ablation — keep it, but it is not the story.

| # | Use case | Spec | Target quantity | Arm A (without) → Arm B (with) | State |
|---|---|---|---|---|---|
| 2 | GEPA-Craftax (HEADLINE) | `gepa_trace_annotate.json` | Craftax mean reward (match champion-cycle reward-delta; precedent Δ3.72 @ n=64) | base ReAct prompt → GEPA-proposed prompt | powered n=64 done: +0.678, 95% CI [0.205, 1.151], exact two-sided p=0.007010; `proof/gepa_craftax_ablation.md` |
| 3 | GELO-Craftax | `gelo_trace_annotate.json` | same target | base → GELO-optimized (real prompt from goex_run) | dropped from headline for now: searched handoff-named goex runs; only accepted prompt is `source=seed`; `proof/gelo_craftax_ablation.md` |
| 4 | SMR-ReportBench | `smr_reportbench_trace_evaluate.json` | ReportBench score | baseline report → trace-evaluate-guided | spec validates, never run; scorer unwired |
| 1 | Blog quality scan (demo) | `quality_scan_blogs.json` | — (substrate demo) | live `--actor codex` manifest | fake-only; live owed |

**Acceptance tests per ablation (in order):** AT-1 plumbing (both arms run e2e →
manifests) · AT-2 measurement (target-metric scorer reads both arms → a number
each) · AT-3 uplift (B−A>0, n reported, table committed) · AT-4 fairness (only the
intervention differs). Plus optimizer-adapter **golden test** (real manifest →
valid `state/jesterky_*`) which gates Arm B, and the proof packet citing each
table. **No CI** (Josh, 07-08): the gate is `cargo test --workspace` +
`cargo build --all-targets` run locally before each handoff lands — I run it, not a
PR bot (that is what caught every untested break).

**Critical path to a great blog:**
1. Optimizer-adapter golden test (gates Arm B).
2. **GEPA-Craftax base-vs-GEPA ablation** — done at paired n=64 with CI + exact permutation p.
3. GELO-Craftax — dropped until a real non-seed GELO prompt exists.
4. SMR-ReportBench: wire ReportBench scorer to a trace-evaluate manifest → ablation.
5. Live quality-scan manifest; proof packet.
6. Prose (Josh) · Mintlify L3 · GitHub-public (Josh) · flip `draft:true`→live.

Sizing: this is EVAL/experiment work (days per ablation for a defensible n), not
plumbing. Everything hinges on the first real ablation number (#2).

### Blogpost evidence ledger — keep current

Last synced: 2026-07-08 after the powered GEPA Craftax run.

Rule: every numerical or capability claim in `blog/jesterky-launch.md` must point
at a row below. When a proof changes, update this table, the blog sentence, and
`proof/RELEASE_ABLATION_STATUS.md` in the same pass. If a proof is fake-only,
smoke-only, or missing a real target metric, say that here instead of implying a
headline claim.

| Blog claim | Current evidence | Status | Blog-safe wording |
|---|---|---|---|
| GEPA prompt improves Craftax reward | `proof/gepa_craftax_ablation.md`; `proof/gepa_craftax_ablation/ablation_summary.json`; 128 raw rollout records under `proof/gepa_craftax_ablation/`; runner `scripts/gepa_gelo_ablation.py` | ✅ headline proof | Base ReAct mean 2.328 → GEPA prompt mean 3.006; uplift +0.678, 95% CI [0.205, 1.151], exact two-sided p=0.007010, paired n=64. |
| The n=8 GEPA run is not the claim | Old smoke records remain in the same raw directory for seeds 501–508; invalid result called out in `HANDOFF_jesterky_gepa_gelo_largen_ablation.md` | ✅ audit-only | Mention only as prior smoke/audit trail if needed; never use +0.85 as headline. |
| GELO ± jesterky workflows (Craftax) | `proof/gelo_jesterky_workflow_ablation.md`; Arm A/B configs `proof/gelo_jesterky_workflow_arm_{a,b}.json`; runner `scripts/gelo_jesterky_workflow_ablation.py`; goex hook `go_ex.jesterky_workflow` | ✅ PASS (uplift over baseline) | A +0.339 vs B +0.977; B run `…010853Z` receipts non-empty (themes 8/32/47/65). Hollow `…003418Z` INVALID audit-only. Old prompt A/B remains a documented drop. |
| GEPA ± jesterky workflows (Craftax) | `proof/gepa_jesterky_workflow_ablation.md`; Arm A/B configs `proof/gepa_jesterky_workflow_arm_{a,b}.toml`; runner `scripts/gepa_jesterky_workflow_ablation.py`; GEPA hook `jesterky_workflow` | ❌ FAIL (B behind) | Bigger budget: A `…033652Z` heldout 1.5 > B `…040537Z` 1.0; B themes 20/23/22 non-empty; A zero receipts. Prior small-budget tie audit-only. Not a PASS. Separate from n=64 prompt A/B. |
| SMR ReportBench trace-evaluate workflow | `examples/smr_reportbench_trace_evaluate.json`; `proof/README.md` §12; fake AT-1 line in `proof/RELEASE_ABLATION_STATUS.md` | 🟡 wired, not measured | Safe as “trace evaluator spec exists / next rung.” Not safe as “ReportBench uplift” until with/without scorer runner and real trace dir are committed. |
| Blog quality scan workflow | `examples/quality_scan_blogs.json`; proof command in `proof/README.md` | 🟡 live proof owed | Safe as workflow shape only unless `proof/quality_scan_blogs.live.manifest.json` exists. |
| Worker-invocable workflows through Stack MCP | `proof/RELEASE_ABLATION_STATUS.md` MCP section; Stack tools `stack_jesterky_{register,launch,inspect,replay,compare}` | ✅ verified running per release status | Safe to say workers can register, launch, inspect, replay, and compare jesterky workflows when the binary is on PATH or `STACK_JESTERKY_COMMAND` is set. |
| Public install / launch readiness | crates/PyPI already published; GitHub public, Mintlify L3, live flip still Josh-gated | 🟡 gated | Safe: “published substrate + draft launch proof.” Not safe: “public product launch complete.” |

Next evidence updates:

1. Add or waive `proof/quality_scan_blogs.live.manifest.json`.
2. Decide whether SMR ReportBench gets a real with/without runner or an explicit blog waiver.
3. GELO ± jesterky workflows Craftax A/B re-PASS after extract/fail-closed fix (A +0.339 → B +0.977; non-empty themes on B).
4. GEPA ± jesterky workflows Craftax A/B scored FAIL on M5a primary even at bigger budget (A 1.5 > B 1.0; B themes non-empty); do not claim PASS.

### OSS ship gaps

| Gap | Why it matters |
|---|---|
| `github.com/jesterky` held private | crates.io / Cargo.toml / Mintlify clone URLs broken for outsiders |
| No GH release assets / brew | crates install only |
| No trusted publishing attestations | Release standards open item |
| Published 0.1.0 ≠ WIP tree | Demos outrun installs |
| mloky not frozen | Oracle can drift |

**Josh-gated:** GitHub public · blog live · (packages already authorized and live).

---

# 4. Core architecture

### Runner shape

```text
WorkflowSpec
  → validate_and_hash
  → seed ledger from --args
  → for each entrypoint node:
       resolve inputs (ledger refs)
       program | actor | map | while | branch | session_group | resume_session | …
       emit EventKinds
       write outputs to ledger
       record impure actor/resource outputs (by Addr)
  → ProcessNode trace (partial today)
  → RunManifest { events, recorded, trace, status, budgets?, goals? }
```

- **Programs** pure → re-run on replay  
- **Actors/resources** impure → recorded for replay  
- **Parallel map** per-item ledger clones + per-node `local_seq`  
- **Mailbox** constructed; **not** hooked to a publish node yet  
- **Done today** = graph finished (+ map `min_success` + budget hard fail)  

### Core/host seam (ADR #6)

| Trait | Role |
|---|---|
| `Actor` | Impure call: inputs → outputs + optional score/signal + artifacts |
| `Resource` | observe/step for env (DungeonGrid-style) |
| `EventSink` | Where the event stream goes (mem, file, future SSE/Redis) |
| `Clock` | Wall time injected (replay uses recorded timestamps as metadata only) |
| `ArtifactStore` | Blob offload |
| `CheckpointStore` | Session resume |

Core has **no** model, HTTP, subprocess, or OS clock. Same seam for live + replay (`ReplayActor` / `ReplayResource`).

### Node taxonomy (closed)

`program`, `reduce`, `actor`, `map`, `for_each`, `while`, `branch`, `session_group`, `resume_session`.

Adding a kind is a contract change. No eval DSL; data flows via declared bindings only.

### ADRs that must not regress

| ADR | Rule |
|---|---|
| #5 | Event identity = structural `Addr`; never emit-time global counter |
| #6 | Core/host trait seam; no model-shaped fields in core |
| #7 | Replay = re-drive orchestration with recorded impure outputs |
| #3 | Closed node kinds + declared-edge I/O |

---

# 5. Event stream (shared? SSE?)

### Short answers

| Question | Answer |
|---|---|
| Shared event stream? | **Yes** — one stream per run all nodes write into |
| Available via SSE? | **No** — not implemented; design allows a host `EventSink` that *could* be SSE |
| Multi-tenant bus? | **No** |
| Optimizers live tail? | **No** — post-hoc manifest only |

### Durable contract stream

```text
Runner::emit → EventSink::emit(Event)
             → RunCtx.events (Vec)   // dual-write today
             → RunManifest.events
```

- `Event { addr, kind, payload, wall_ms }`  
- Order/identity = `Addr` `(run_id, node_path, iteration, local_seq)`  
- `EventKind` closed enum; `payload` free JSON  
- Hosts today: `MemEventSink`, `SharedEventSink` (for `--follow`)  
- Canonical compare: **sort by Addr**, not emission order  

### Ephemeral live progress (not contract)

`LiveBus` / `LiveEvent` (WIP): tokens/steps/last action via std mpsc.

- Not serialized, not replayed, not new `EventKind`  
- Best-effort; no-op without `--follow`  

### Architecture picture

```text
┌─────────────────────────────────────────┐
│  jesterky-core (no IO)                  │
│    ledger ← bindings → nodes            │
│    emit(Event) ──► EventSink            │
│    recorded[] ──► replay                │
│    manifest out                         │
└────────────┬───────────────┬────────────┘
             │               │
   Mem/Shared sink    design: future sinks
             │         (SSE, Redis, file, Cloud)
             ▼
        CLI --follow + LiveBus
             ▼
        proof/*.manifest.json  ← only durable “API” today
```

**M4 intent:** hosted runner = same Event/manifest shape; SSE/streaming = host-side sink, not core. Smallest pre-SSE win: `jesterky run --events-out` NDJSON.

---

# 6. Typing of inputs/outputs

### Strongly typed

- Topology / closed `NodeKind`  
- Events / `Addr` / `NodePath`  
- Manifest shells (`RunManifest`, `RecordedOutput`, `ProcessNode`, `CallKind`)  
- Bindings maps: local name → `Ref`  
- Budgets (`BudgetPlan`, `BudgetSnapshot`, …)  

### Soft / JSON

| Surface | Reality |
|---|---|
| Ledger | `HashMap<String, Value>` |
| `Ref` | Newtype over `String` (ad-hoc parse; TODO AST) |
| Actor I/O | `serde_json::Value` |
| Event payload | Free JSON per kind |
| Process tree leaf inputs | Often **Null** (join via `ActorInvoked` events) |
| Model output schemas | Host/codex `--output-schema` only; **core does not validate** |

**Summary:** graph structure typed; business data is JSON. Optimizer-first “typed per-node I/O” is shape-ready, not fully populated or schema-gated.

### Score / signal

Slots exist on `ActorResult` / `ProcessNode` for wall-safe outcomes. **`ModelActor` usually sets both `None`**. DungeonGrid fills score more seriously. Must not put heldout labels into proposer-facing theme registries.

---

# 7. Three control planes

**Keep these distinct. Do not merge into one blob.**

| Plane | Field | Question | Status |
|---|---|---|---|
| **Concurrency limits** | `runplan.limits` → semaphores | How many at once? | ✅ core (`limits.rs`) |
| **Resource budgets** | `runplan.budgets` → `BudgetEngine` | How much may we *spend*? | ✅ WIP (`budget.rs`) |
| **Goals / work** | proposed `runplan.goals` | Have we *achieved* the product? | ❌ charter only |

Naming: optimizers/SMR “LimitEngine” ≈ jesterky **budget engine**. Concurrency is separate and older.

---

# 8. Limit / budget engine (detail)

### Intent

OSS formalization of optimizers **`LimitEngine`** + SMR **progress-toward-resource-limits**:

- Declarative in workflow JSON  
- Pure engine projects observations → snapshot  
- Host meters (actor calls, tokens, wall)  
- Panel + manifest  

### Types (`jesterky-contract` / `budget.rs`)

| Type | Role |
|---|---|
| `BudgetPlan` | Full JSON config on `runplan.budgets` |
| `BudgetCap` | One dim: kind, max, hard, label, warning_percent, show_* |
| `BudgetKind` | `actor_calls` \| `tokens` \| `wall_seconds` (extend carefully — contract bump) |
| `BudgetEtaConfig` / `BudgetEtaMode` | `off` / `nearest_only` / `all` |
| `BudgetVizConfig` | Panel lines |
| `BudgetObservation` | Host-metered spent samples |
| `BudgetState` | ok / warning / exhausted |
| `BudgetForecast` | rate + seconds_to_limit + confidence |
| `BudgetStatus` | per-cap status |
| `BudgetSnapshot` | plan + items + nearest; `budget_engine.v1` |
| `BudgetEngine::snapshot` | Pure: plan + observations → snapshot |
| `BudgetPlan::overlay_json` | Deep-merge for `--args.budgets` |

### Semantics (easy to get wrong)

| Surface | Meaning |
|---|---|
| Progress | spent/max, %, state |
| **ETA** | Estimated **time until that cap is exhausted** at current burn rate — **not** time-to-finish |
| Hard exhaust | Host may set `RunStatus::Failed` if `fail_on_hard_exhaust` |
| Soft cap | Warn / show state only |
| Episode-scale ETA | Set caps near episode size (e.g. `actor_calls.max ≈ max_turns`) |

### JSON knobs (example)

```json
"runplan": {
  "budgets": {
    "warning_percent": 80,
    "fail_on_hard_exhaust": true,
    "eta": { "enabled": true, "mode": "all", "min_wall_secs": 1.0 },
    "viz": { "show_progress": true, "show_eta": true, "show_nearest_tag": true },
    "caps": [
      { "kind": "actor_calls", "max": 20, "hard": false, "label": "calls" },
      { "kind": "tokens", "max": 400000, "hard": false },
      { "kind": "wall_seconds", "max": 900, "hard": false }
    ]
  }
}
```

Partial override: `--args '{"budgets":{"eta":{"mode":"nearest_only"}}}'`.

### Files map

| Path | Notes |
|---|---|
| `crates/jesterky-contract/src/budget.rs` | Engine + types |
| `crates/jesterky-contract/src/topology.rs` | `RunPlan.budgets`, `RunPlan.limits` |
| `crates/jesterky-contract/src/artifact.rs` | `RunManifest.budgets` |
| `crates/jesterky-cli/src/main.rs` | resolve/project, hard exhaust |
| `crates/jesterky-actor/src/viz.rs` | budget panel lines |
| `crates/jesterky-core/src/limits.rs` | concurrency only |
| `docs/BUDGETS.md` | user docs |
| `examples/budgets_episode_scale.json` | episode ETA template |

### vs optimizers LimitEngine

| Optimizers | jesterky |
|---|---|
| Many kinds (cost, train/heldout rollouts, generations, custom) | Three kinds |
| spent **+ reserved** | spent only |
| Forecast confidence bands | ETA nearest/all |
| Separate stopper stages (`deferred_budget`) | hard fail or complete |
| SQLite + progress events | Manifest snapshot + panel |

### Gaps to close on budgets

- Reserved budget under wide parallel maps  
- Cost USD / tool calls / env steps as future kinds  
- Publish with runtime train (not only dirty tree)  
- Stopper vocabulary on manifest (`budget_exhausted` vs completed)  
- Align naming docs: “budget engine” not “limit engine” when meaning spend  

---

# 9. Goals / work product engine (next)

### Why

Long agent runs care about:

1. Will I hit the token/wall cap? → **budgets (shipped WIP)**  
2. Have I already produced the thing I wanted? → **goals (not shipped)**  

OBLIQ, artifact gen, “search until found” need **semantic termination**, not only structural completion. Optimizers need a stable goal surface dual to `BudgetSnapshot`.

### Dual of budgets

| Budget engine | Goals / work engine |
|---|---|
| Caps on *spend* | Targets on *achievement* |
| Progress: spent / max | Progress: work done / criteria |
| ETA: time-to-exhaust | Optional time-to-goal |
| Exhausted → optional fail | Met → optional **successful early terminate** + finalize |
| `BudgetSnapshot` | `GoalSnapshot` / `WorkSnapshot` |
| Host meters resources | Host/programs evaluate predicates / attach work products |

### Concepts

| Term | Meaning |
|---|---|
| Scoped work product | Named deliverable (artifact ref, ledger key, schema JSON, metric threshold, …) |
| Goal | Predicate or checklist over work products + optional scores |
| Terminate signal | Goal met → stop orchestration, run finalize, status Completed + goal.state=met |
| Goal progress | Partial credit: checklist %, best score, shards with hits |
| Finalize path | Optional node id / subgraph on success wrap-up |

### Desired behaviors

1. **Early success wrap-up** — remaining work skipped/cancelled; finalize; goal met  
2. **Hard fail goal** — budget/iters exhausted without product → `Failed` / `goal_unmet`  
3. **Soft goal** — record progress only (today’s OBLIQ `verdict` behavior)  
4. **Panel** — goal line next to budget lines  
5. **JSON-configurable, typed, documented** — same bar as budgets (`docs/GOALS.md`)  

### Sketch (not implemented)

```rust
// proposed: crates/jesterky-contract/src/goal.rs
pub struct GoalPlan { /* goals, terminate_on_met, finalize, viz */ }
pub struct GoalSpec { /* id, required, kind, work_product */ }
pub enum GoalKind {
    LedgerPred { path, equals },
    WorkProductReady { work_product_id },
    MetricThreshold { path, min },
    SignalFlag { key },
}
pub struct GoalSnapshot { /* plan, items, state, terminated_early */ }
```

### Today’s “done” (structural only)

```text
done (engine)     ⇔ entrypoint finished without error + map min_success
success (engine)  ⇔ RunStatus::Completed
success (semantic)⇔ whatever is in ledger/summary JSON (core ignores)
```

Existing DIY: `while` + ledger cond (no cancel of in-flight map siblings).

### Minimal vertical slice (recommended first PR)

1. Contract: `GoalPlan` / `GoalSnapshot` / engine with **ledger pred + metric threshold** only  
2. Runner: after entrypoint node, evaluate; if met + terminate → skip remaining entrypoints; optional finalize  
3. **No** in-flight cancel yet  
4. CLI: `manifest.goals`; viz one goal line  
5. Example: serial “search until found” or OBLIQ early stop  
6. Docs: `docs/GOALS.md`  

### Open questions (owner)

1. Early terminate first vs rich work-product schemas first? (Recommend early terminate.)  
2. Unmet required goals → `Failed` or `Completed` + unmet?  
3. Should optimizers target `GoalSnapshot` like budget efficiency?  
4. Entrypoint-skip only (v1) vs cooperative cancel (v2)?  
5. Keep OBLIQ soft verdict as `terminate_on_met: false` default?  

### Target architecture

```text
                    WorkflowSpec
              limits | budgets | goals
              ┌───────┼───────┐
              ▼       ▼       ▼
        concurrency  BudgetEngine  GoalEngine
              │       │             │
              │  host meters   host/programs evaluate
              └───────┼─────────────┘
                      ▼
                Runner control
         (budget fail? goal met? → finalize + stop)
                      ▼
              RunManifest (+ budgets + goals)
                      ▼
              Viz / optimizers / CLI / Stack
```

---

# 10. What should be done better (code & systems)

### Tier 1 — before consumers build on this

1. **Process tree is optimizer-starved** — leaf `inputs: Null`; ModelActor score/signal usually `None`. Record inputs; fill scores from reduces.  
2. **Two live streams, neither a real API** — contract sink + LiveBus; add `--events-out` / multi-subscriber host sink before SSE.  
3. **Typing honesty** — parse `Ref` AST + validate bindings, or document JSON-only and enforce host schema when declared.  
4. **Machine-absolute defaults** — `DEFAULT_BLOG_DIR` / `DEFAULT_DOCS_DIR` under `/Users/joshpurtell/...` must die.  
5. **WIP ≠ published 0.1.0** — commit and cut 0.1.1/0.2 or demos forever outrun installs.  

### Tier 2 — core quality

6. Parity gate too thin for “same runtime as mloky” marketing — fixture of recorded outputs + Addr-sorted event kinds.  
7. Live quality scans prove substrate, not audit quality — bounded-read prompts + committed live blogs manifest.  
8. Dual-write emit (sink + `ctx.events`) — pick source of truth.  
9. Mailbox kinds exist, nothing publishes — wire or strip from public surface until used.  
10. Goals vs budgets asymmetry — charter goals engine.  
11. No `producer` / contract version on events (rebuild handoff wanted this).  
12. HostConfig lives in contract but core never reads it — document or split.  

### Tier 3 — ops / ship

13. No CI  
14. Public identity half-broken (private GitHub, public crate metadata)  
15. Doc drift (blog still says follow not wired; handoffs lag CLI)  
16. GEPA “integration” stops at local JSON  
17. Hollow verdicts undermine quality-scan marketing  

### Practical priority order

| # | Change |
|---|---|
| 1 | Commit + publish WIP (or park) |
| 2 | Kill absolute paths |
| 3 | Fill process-tree inputs + score/signal |
| 4 | `--events-out` |
| 5 | CI + schema drift |
| 6 | Live blogs proof + audit prompts |
| 7 | Optimizer `state/jesterky_*` hook |
| 8 | Stronger parity fixture |
| 9 | Goals engine v1 |
| 10 | SMR ReportBench or blog waiver |
| 11 | Stack M3 staging proof |
| 12 | Hosted M4 smoke |

### Already good — don’t “fix” by overbuilding

- Addr clock + replay  
- Core/host seam  
- No pyo3  
- No SSE in core  
- Closed node taxonomy; no JS in JSON  
- Fake E2E + schema drift guard  
- Pure BudgetEngine projection  

---

# 11. Data systems patterns

From optimizers platform + what jesterky should absorb.

| # | Pattern | Optimizers | jesterky today | Next |
|---|---|---|---|---|
| 1 | Append-only log + projections | events.jsonl + SQLite + state slices + freshness | Event + manifest | `--events-out`; later compare/normalize |
| 2 | Schema-versioned records | Everywhere `*.v1` | Budgets yes | All durable payloads |
| 3 | Content-addressed / stable hash | normalize → hash; trace digests | `spec_hash` | Payload/trace digests |
| 4 | Ledgers | Usage + limits + stopper | BudgetSnapshot | Usage on recorded outputs; reserve; stop reasons |
| 5 | Checkpoint / resume | Generation/frontier snapshots | Session checkpoints | Multi-gen run checkpoints if needed |
| 6 | Invariants as data products | InvariantReport | One conformance test | Per-run conservation report |
| 7 | Leases / isolation | resource_lease_record | Semaphores | Cloud multi-tenant later |
| 8 | Cache at external boundary | Request/response cache | None | Eval loops later |
| 9 | Lineage graph | Candidate deltas / plan links | Process tree only | Optimizers-side; emit joinable addrs |
| 10 | Export vs operational store | JSONL + SQLite | Manifest only | Hosted workspace later |

**Rule:** never treat live UI as source of truth; recompute from log + durable tables.

### Layers diagram

```text
 LLM calls / env steps
        │
        ▼
  SensorFrame + UsageLedger + TraceDigest     ← measurement plane
        │
        ▼
  append Event log (+ future workspace)       ← data plane
        │
        ├─► BudgetEngine / Stopper / Goals    ← control plane
        ├─► normalized events / cache         ← reproducibility plane
        ├─► EvidenceFrame / TraceAnnotation   ← retrieval plane
        └─► Candidate graph / levers          ← search plane (optimizers)
                │
                ▼
        Proposer (schema-gated, wall-safe)
                │
                ▼
        Container / verifier / env scores     ← oracle (Chinese wall)
```

---

# 12. LLM systems patterns

| # | Pattern | Meaning | jesterky implication |
|---|---|---|---|
| 1 | Chinese wall | Wall-safe proposer features vs grader/heldout | Theme registries must not leak heldout |
| 2 | Split roles | Policy vs proposer vs verifier; separate auth | Name actors clearly; separate codex homes |
| 3 | Prompt program as data | Modules, mutability, levers | Roles today are free strings; M5 needs lever-shaped prompts |
| 4 | Sensor frames | Episode rows: stage, reward, usage, digest | Map items need an envelope optimizers can ingest |
| 5 | Evaluation stages | Seed → minibatch → full train → heldout | One workflow run ≈ one stage unless phased entrypoints |
| 6 | Evidence-grounded generation | Retrieve structured workspace; cite files | Trace annotate → proposer workspace files |
| 7 | Schema-gated outputs | LLM → schema → durable record | Validate before ledger write when schema set |
| 8 | Failure taxonomy | Class drives policy | Promote viz fail_annotation to typed failures |
| 9 | Substrate isolation | local/docker × sandbox × auth | Keep knobs orthogonal on CodexModel |
| 10 | Container-as-oracle | Env grades; model doesn’t self-score | DungeonGrid in-process env OK; don’t self-grade quest |
| 11 | Budget-aware search | Multi-dim constrained optimization | Outer loop = optimizers; single-run = jesterky budgets |
| 12 | Trace digesting | Compress before proposer context | `gepa_trace_annotate` is the compressor |

---

# 13. Strong patterns from optimizers code

Primary home: `optimizers/rust/crates/synth_optimizer_platform` (+ `synth_gepa`, `synth_mapo`).

### Platform vs algorithm

Platform owns config, container, cache, events, limits, workspace SQLite, artifacts, evidence, levers. Algorithms only own search policy.

### LimitEngine (ancestor of jesterky budgets)

- Pure projection; spent + reserved  
- Closed kinds including cost and train/heldout rollouts  
- Forecast confidence bands  
- `schema_version: limit_engine.v1`  
- Separate stopper (`deferred_budget` honesty)  

### Chinese wall

Graders and heldout on outcome side only. Proposer sees summaries. Container is scoring authority. Release metadata: `effortbench_cookbook_chinese_wall = grader_only`.

### Proposer workspace (the real integration API)

```text
proposer_workspaces/generation_N/
  state/proposer_failure_summary.json
  state/proposer_examples.json
  state/proposer_repair_hints.json
  state/rollouts.json
  state/scores.json
  state/evidence_frames.json
  proposal/manifest.json   # schema_version + evidence citations
```

### Evidence frames & trace annotations

`TraceAnnotation`, `EvidenceFrame`, `VerifierJob` — all `schema_version`’d.

### Normalized event feed

Strip timestamps, generated ids, local paths, volatile sessions, host ports → `events.normalized.jsonl`.

### Candidate graph + levers

`LeverKind` (prompt, agents.md, skill, tool policy, verifier rubric, …), deltas, acceptance, frontier, plan links.

### Container contract

`/health` `/metadata` `/task_info` `/program` `/dataset` `/rollout`.

### Config by durable nouns

`[run] [container] [dataset] [candidate] [policy] [proposer] [gepa] [cache]`. No silent risky defaults. Profiles smoke/default/long + budget math.

### Auth / substrate separation

| Knob | Meaning |
|---|---|
| Policy credentials | Student / rollout |
| Proposer credentials | Codex app-server |
| `runtime_substrate` | local vs docker |
| `sandbox_mode` | In-agent FS policy |

### Hosted client shape

`submit → wait/watch → event_backfill → get_state_slice → receipt / evidence-packet`.

### Highest-value borrowings

1. Proposer workspace files as hook  
2. EvidenceFrame/TraceAnnotation-shaped outputs  
3. Normalized event export  
4. Fill score/signal + process-tree inputs  
5. Stopper vocabulary  
6. Reserved budget  
7. Invariant reports  
8. Typed evidence packets for launch  

---

# 14. Adding jesterky to optimizers (GEPA/GELO/MAPO)

### Operator decision

**First consumer after M2 CLI is M5a: mass trace processing for GEPA/GELO** — not Stack or hosted. Annotate rollout batches, cluster failure themes, feed richer proposer context.

### Today vs target

| Surface | Today | Target |
|---|---|---|
| GEPA | Env/container rollouts; flat failure summaries | After each generation: jesterky annotate → `state/jesterky_*` in proposer workspace |
| GELO | Theme board from env rollouts | Theme registry with evidence_refs → jesterky manifest addrs |
| MAPO | Env multi-agent policy search; DungeonGrid Plus launch gate | Later: evaluate against jesterky multi-agent workflow traces |
| Workflow optimizer (M5) | — | Summarize/classify/score/diff process trees |
| M6 loop | Separate products | Verifier-driven bundle improvement on held-out signal |

### Topology sketch (implemented as examples)

```text
expand(rollout_batch)
  → map[trace_shard]: actor trace_classifier
  → reduce: program theme_cluster
  → actor/program: context_writer (wall-safe)
```

Specs: `examples/gepa_trace_annotate.json`, `examples/gelo_trace_annotate.json`.  
Proof: `proof/craftax_trace_annotate/*` (WIP).

### GEPA hook acceptance

| Test | Pass criterion |
|---|---|
| Ingest | GEPA `generation_N/` via `--args` |
| Scale | ≥64 shards; conservation + termination |
| Replay | Manifest replays; theme registry stable |
| GEPA hook | Proposer reads `state/jesterky_proposer_context.json` (or `.md`) without wall leak |
| GELO hook | `gelo watch --slice themes` evidence_refs → jesterky addrs |
| A/B | Craftax arm B heldout > arm A **or** documented comparison |

### Craftax GEPA A/B (M5a proof)

| Arm | Trace processing | Proposer context |
|---|---|---|
| A baseline | None | failure_summary, examples, repair_hints only |
| B jesterky | After train rollouts: `jesterky run gepa_trace_annotate` | Arm A + theme_registry + annotations + proposer_context |

Harness: `evals/stackeval/effort_bench/craftax-agent-hillclimb.toml`.  
Primary metric: `best_heldout_mean_reward` on heldout registry.

### Implementation order (WS8)

1. Specs + schemas (partially done WIP)  
2. Adapter: generation dir → `--args`  
3. Optimizers hook: copy `jesterky_*` into proposer workspace pre-round  
4. Run arm A  
5. Run arm B  
6. Document comparison; decide default path  

### Wall-safe rule (non-negotiable)

Annotations may summarize **outcomes and public trace fields** only — no heldout labels, no optimizer-internal selection leak. Verifier grades outcomes; jesterky structures evidence for the proposer.

### M5 Workflow Optimizer (product, not jesterky crate)

Depends on filled process trees. Acceptance: ingest via contract types; process-tree ops; typed diff; wall-safe; local + hosted URI.

### M6 GEPA/GELO/MAPO loop

- GEPA: frontier improvement on held-out **replay score** of workflow bundles  
- GELO: hosted job refs jesterky run id; board shows workflow lever candidates  
- MAPO: heldout comparison uses jesterky multi-agent outcomes (after env-rollout sign-off)  

DoD: Chinese wall, measured held-out improvement, ZVPO where applicable, receipts per hosted-optimizers checklist.

### What optimizers repo must gain

- [ ] Dependency on `jesterky-contract` (types only) for manifest parse  
- [ ] Optional generation hook: run CLI or library entry to annotate  
- [ ] Materialize `state/jesterky_theme_registry.json`, `jesterky_trace_annotations.jsonl`, `jesterky_proposer_context.md`  
- [ ] GELO themes slice: evidence_refs to addrs  
- [ ] Docs in GEPA skill + hosted-optimizers evidence  
- [ ] No scrape of flat logs — typed process tree / evidence frames  

---

# 15. Stack integration (M3)

### Ship

Workflows as a Stack primitive: register / launch / inspect / replay / compare / visualize. Staging → owner A+ → prod.

### Acceptance tests

| Test | Pass criterion |
|---|---|
| Register | Persist topology JSON + metadata → stable workflow id |
| Launch | MCP or TUI launches local/remote run; run id ↔ Stack session/Effort ledger |
| Inspect | Status, event count, terminal state, manifest path/URI without log scrape |
| Replay | Triggers `jesterky replay` (or API); fidelity matches local CLI |
| Compare | Diff two manifests on outcome + selected trace fields |
| Visualize | ProcessNode tree (terminal-tree parity or TUI adapter) |
| Staging proof | Quality-scan from Stack; receipt with run id + commit SHA |
| Prod gate | Owner A+; no Stack minor bump without ask |

### Explicitly not M3

- Mermaid/box-graph viz (parked until Stack TUI earns it)  
- Replacing SMR/Factory workflows — jesterky is a **new** primitive **alongside** Efforts/SMR  

### Status

**Zero** `jesterky` references in `stack` when reviewed. Depends on M2 (✅). Can parallel M4.

### Prerequisites for Stack to not suck

- Outward event stream or durable manifest URI  
- Filled process tree for inspect/compare  
- Conservation/invariant reports for compare  
- Stable run id + org identity (aligns with M4)  

---

# 16. Hosted Cloud + synth-ai (M4)

### Ship

- Rust hosted runner (`jesterky-core` behind HTTP/gRPC)  
- `synth-ai` thin client GA  
- Same event/artifact shape as local  
- Real run identity + org scoping + billing (retire `X-Scan-Run-ID` hack)  

### Acceptance tests

| Test | Pass criterion |
|---|---|
| API | Submit spec → poll/stream events → fetch manifest; OpenAPI in Mintlify |
| Conformance | Hosted stream validates against schema; identical to local on same recorded outputs |
| Auth | Per-org API key; cross-org denied |
| Identity | Run id + org id on every event |
| Billing | Per-run token/cost on staged fixture |
| Client | Contract types only — **no embedded runtime** |
| Prod smoke | Hosted quality-scan → local `jesterky replay` |
| Launch gate | `synth-dev/deployment/launch_checklist.md` if blog promotes hosted |

### Status

Not started. No backend/synth-ai wiring.

### SSE placement

Host `EventSink` → SSE/NDJSON stream of `Event`. Core unchanged.

---

# 17. SMR + ReportBench

### Two different “SMR” touchpoints

#### A. SMR as infra (already used)

- DeepSeek / Responses↔chat proxy for codex routes  
- Budget/progress patterns historically mirrored into jesterky budgets  
- Proof notes: proxy base_url must be `…/api/v1`, not `…/v1`  

#### B. SMR ReportBench as **workflow application** (blog example #4)

**Locked blog role:** map over SMR/ReportBench run traces → rubric verdicts → effort-ready report.

| Item | Status |
|---|---|
| Spec | `examples/smr_reportbench_trace_evaluate.json` — exists; maps traces through a ReportBench outcome rubric and aggregates verdicts |
| Proof | `proof/smr_reportbench_trace_evaluate.*` — missing; fake AT-1 only in `proof/RELEASE_ABLATION_STATUS.md` |
| Harness refs | `evals/reportbench/lanes/readme_smoke/` + SMR hello-world E2E (`run_smr_hello_world_e2e.py`) |
| Blog honesty | Safe as wired/next-rung only; no ReportBench uplift claim until a real trace dir + with/without scorer runner exists |

### Target shape (to author)

```text
expand(reportbench_runs)
  → map: actor rubric_grader (per run/trace shard)
  → reduce: program effort_report_aggregate
  → optional: matrix / artifact emit for Effort ledger
```

### Acceptance (when built)

- [ ] Spec validates  
- [ ] Fake actor dry run + replay  
- [ ] Live or recorded fixture proof under `proof/`  
- [ ] Wall-safe rubric outputs (outcomes only)  
- [ ] Optional: Effort/SMR ledger can link manifest URI  
- [ ] Blog § applications lists command from `proof/README.md`  

### Relationship to Stack / SMR product

jesterky does **not** replace SMR. It is a substrate that can **grade SMR/ReportBench traces** and later sit **beside** SMR paths in Stack. MAPO/SMR env gates remain their own products.

### Waiver option

If blog ships without #4: document waiver in blog + proof README; move SMR evaluate to cookbook follow-on.

---

# 18. Workloads, evals, proof packet

### Workload catalog

| Workload | Spec / code | Status |
|---|---|---|
| Quality scan (code) | `quality_scan.json` | M2 reference; live DeepSeek proven |
| Quality min | `quality_min.json` | Fake E2E in proof/ |
| Blog quality scan | `quality_scan_blogs.json` | Spec; live proof owed; absolute default path |
| Docs quality scan | `quality_scan_docs.json` | Spec; not packetized |
| GEPA trace annotate | `gepa_trace_annotate.json` | Spec + Craftax proof WIP |
| GELO trace annotate | `gelo_trace_annotate.json` | Same |
| DungeonGrid 4p | `dungeongrid_4p.json` | LLM short runs + budgets; not 100-turn mloky |
| OBLIQ math | `obliq_math_verify.json` + modes | Measured flash/pro; docs/OBLIQ.md |
| SMR ReportBench | missing | Not wired |
| Essay skeleton | — | mloky only; not ported |

### OBLIQ difficulty ladder

| Mode | Meaning | Signal |
|---|---|---|
| `verify` | Gold-infused pool | Ceiling ~0.99 both models |
| `hard_verify` | 1 gold + near-misses | Pro > flash (~0.95 vs 0.75) |
| `retrieve` | Lexical top-k only | Collapses (~0.1) |
| `retrieve_hard` | No lexical golds | Hardest |

Soft `verdict` in aggregate is **not** run termination.

### Proof packet sections (`proof/README.md`)

1. Install from clean checkout  
2. Fake E2E quality_min  
3. Contract schema + drift  
4. Publish dry-run  
5. mloky parity gate  
6. Live model E2E (manual)  
7. Blog quality scan (pending live)  
8–9. GEPA/GELO annotate (measured local replay)  
10. DungeonGrid 4p  
11. OBLIQ math  
12. SMR ReportBench (not wired)  

---

# 19. Testing & E2E

### Present

- Contract: drift, roundtrip, conformance  
- Core: nodes, sessions, control flow, args seeding, replay fidelity  
- Quality: scan + mloky outcome parity  
- CLI tests  
- Live codex tests `#[ignore]`  
- Deterministic fake E2E in `proof/`  
- Measured workloads in WIP proof/  

### Missing

- CI (no `.github`)  
- Live-proof optional/nightly job  
- Committed live quality_scan / blogs manifests  
- Automated OBLIQ threshold tests  
- Craftax GEPA A/B  
- Hosted round-trip  
- Stack register→replay  
- Optimizer consume-path e2e  
- Formal orphaned-codex capture  

**Replay ≠ product e2e.** Replay proves orchestration; not that optimizers/Stack/SMR consume output.

### Parity gate honesty

Projects mloky + jesterky onto `RunOutcome`: conservation (jobs started == completed == in report) + termination. **Not** event-byte equality (different vocabularies). Roadmap originally wanted stream equality — still open as stronger fixture.

---

# 20. Content / launch L3

| Artifact | Status |
|---|---|
| Blog `feature_release` draft | ✅; needs proof updates + follow text fix |
| Changelog 0.1.0 | ✅ |
| Mintlify quickstart/cookbook/reference | 🟡 uncommitted in `docs` |
| Proof page frontend | ⛔ |
| Blog checklist / release standards | ⛔ not passed |
| Versioning policy page | ⛔ |

**Announce-after-ship:** blog Try-it matches Mintlify; packages already published; every claim → proof command.

**Routing:** blog = narrative; Mintlify = procedural truth; proof = evidence.

---

# 21. Master checklist

### Substrate / code

- [ ] Commit budgets, follow, workloads or park explicitly  
- [ ] Publish 0.1.1/0.2 if demos need WIP  
- [ ] Remove absolute blog/docs paths  
- [ ] Fill ProcessNode inputs + score/signal  
- [ ] Schema-validate actor outputs when schema declared  
- [ ] `--events-out` NDJSON  
- [ ] Dual-write emit cleanup  
- [ ] Mailbox: wire or hide  
- [x] Goals engine v1 (ledger pred + metric + terminate + fail-on-unmet) — `goal.rs`, runner eval, `docs/GOALS.md`, `examples/goal_quality_gate.json`  
- [ ] Budget reserve + stop vocabulary  
- [ ] Invariant report on every map/reduce run  
- [ ] Stronger parity fixture  

### CI / OSS

- [ ] `.github` CI: `cargo test --workspace` + schema drift  
- [ ] Josh: public GitHub org/repo  
- [ ] Fix crate metadata if GitHub still private  
- [ ] Trusted publishing / release assets (optional next)  
- [ ] mloky freeze coordination  

### Optimizers

- [ ] GEPA/GELO specs green on real generation dir  
- [ ] Materialize `state/jesterky_*` into proposer workspace  
- [ ] GELO themes evidence_refs  
- [ ] Craftax A/B or documented comparison  
- [ ] `jesterky-contract` types in optimizers where needed  
- [ ] MAPO jesterky row (later)  

### Stack

- [ ] Register / launch / inspect / replay / compare  
- [ ] Staging quality-scan receipt  
- [ ] Owner A+ → prod  

### Hosted

- [ ] Runner service + OpenAPI  
- [ ] Auth, identity, billing  
- [ ] `synth-ai` workflows namespace  
- [ ] Local replay of hosted manifest  

### SMR

- [ ] Author `smr_reportbench_trace_evaluate` **or** document blog waiver  
- [ ] Proof packet section 12  
- [ ] Optional Effort ledger link  

### Content

- [ ] Live blogs quality proof  
- [ ] Update blog applications + limitations (follow wired)  
- [ ] Land Mintlify pages  
- [ ] Frontend proof page  
- [ ] Pass blog + release + cookbook checklists  
- [ ] Flip blog live after Josh gates  

---

# 22. Doc index

### This repo

| Doc | Role |
|---|---|
| `README.md` | Crate map, quickstart |
| `CHANGELOG.md` | 0.1.0 |
| `docs/BUDGETS.md` | Budget semantics |
| `docs/OBLIQ.md` | OBLIQ workload |
| `proof/README.md` | Proof commands |
| `blog/jesterky-launch.md` | Feature release draft |
| `HANDOFF_jesterky_release_and_blog_plan.md` | Release plan + M3–M6 |
| `HANDOFF_jesterky_limits_and_goals_engine.md` | Budgets + goals charter |
| `HANDOFF_jesterky_process_tree_io.md` | **NEXT CORE** — fill ProcessNode inputs/outputs/scores (unblocks optimizer hook) |
| `HANDOFF_jesterky_core_residuals.md` | Remaining core logic: dual-write/mailbox/paths (A), events-out/typing/output-schema (B), goals-v2/budget-reserve/invariants (C). ALL DONE (96 tests). |
| `HANDOFF_jesterky_integrations.md` | **END-TO-END** — all integrations (optimizers/Stack/Hosted/SMR) + ship hygiene, authored in consumer repos; §0 = the jesterky contract surface they build against |
| `HANDOFF_jesterky_round6_live_scan.md` | DeepSeek live |
| `HANDOFF_jesterky_terminal_viz.md` | Viz slices |
| `HANDOFF_jesterky_round4_codex.md` | M1 ship |
| **`jesterky_notes.md`** | **This file** |

### Sibling

| Doc | Role |
|---|---|
| `mloky/ROADMAP_jesterky.md` | M0–M6 |
| `mloky/HANDOFF_jesterky_rust_rebuild.md` | ADRs |
| `mloky/HANDOFF_workflows_status.md` | mloky GREEN |
| `mloky/TOPOLOGY_EXPRESSIVITY_V2_PLAN.md` | Claude-scope; no JS |
| `workflows/BUILD.md` | Applications catalog |
| `optimizers/docs/hosted-optimizers.md` | Hosted GEPA/GELO |
| `optimizers/skills/gepa/SKILL.md` | Proposer workspace |
| `optimizers/rust/crates/synth_optimizer_platform/` | LimitEngine, evidence, workspace |
| `Jstack/.../release_launch_standards.md` | L1/L3 |
| `Jstack/.../blog_post_quality_checklist.md` | Blog gates |
| `synth-dev/deployment/launch_checklist.md` | Prod if hosted promoted |

---

# 23. Bottom line

**Core orchestration is good:** Addr clock, pure/impure split, host seam, closed taxonomy, replay, packages shipped.

**Limit/budget engine is real (WIP):** typed JSON, pure projection, progress + time-to-cap ETA, aligned with optimizers LimitEngine / SMR progress-toward-limits — still needs publish, reserve, stopper vocabulary.

**Goals/work product engine — v1 SHIPPED (uncommitted):** semantic termination as the dual of budgets. Pure `GoalEngine` (ledger_pred + metric_threshold), runner-evaluated against the ledger, early success wrap-up (skip remaining entrypoints on met), fail-on-unmet. `GoalSnapshot` on `RunManifest.goals`; CLI goal line + `--args.goals` overlay; `docs/GOALS.md`; `examples/goal_quality_gate.json`. Follow-ups: in-flight cancel, `finalize` execution, `work_product_ready`/`signal_flag` kinds, in-panel goal line.

**Falling short on:** consumer data plane (process-tree I/O, scores, outward stream), typing honesty, release hygiene (GitHub, CI, dual version reality), and platform wiring.

**Integrations:**

| Surface | Status |
|---|---|
| Optimizers GEPA/GELO | Specs exist; **hooks not wired**; M5a is first app |
| Stack | Fully scoped (M3 acceptance); **zero code** |
| Hosted / synth-ai | Scoped (M4); **zero code** |
| SMR ReportBench | Blog-locked example; **spec missing** (or waive) |
| SMR proxy infra | Used for live DeepSeek path |

**Steal from optimizers:** typed durable records, Chinese walls, limit/stopper honesty, proposer workspace contracts, container-as-grader, normalized evidence, schema_version discipline — not search internals.

**Systems upgrade path:** make every run **measurable, wall-safe, and joinable** (sensor/evidence style) without needing SSE or full SQLite workspace on day one; wire optimizers via **workspace files**; Stack via register/launch/manifest; SMR via ReportBench map-reduce or explicit waiver.

---

*Prefer this file + `docs/BUDGETS.md` + `HANDOFF_jesterky_release_and_blog_plan.md` + `HANDOFF_jesterky_limits_and_goals_engine.md` over chat scrollback.*
