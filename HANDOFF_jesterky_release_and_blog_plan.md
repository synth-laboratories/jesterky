# Handoff — jesterky release + blog plan (for eng planning)

**Date:** 2026-07-08. **Audience:** engineer building the integrated release +
content plan. **Goal:** one doc that lists every authoritative source, current
state, open workstreams, gates, and how the public story ships.

**Supersedes:** the interrupted "jesterky release-blog" task noted in
`Jstack/.jstack/daily_notes/2026-07-08/HANDOFF_synth_blog_strategy_20260708.md`
(no plan was created there — this is it).

---

## At a glance — done · next

**Head:** `main` @ `f39d8da` (2026-07-08). Substrate + packages shipped; story held on Josh.

### Release blog examples (locked — operator 2026-07-08)

Four workflows anchor the feature_release post. Substrate first; applications at
the end — each with spec path, proof command, and honest readiness tier.

| # | Application | Spec (target) | Blog role | Status |
|---|---|---|---|---|
| 1 | **Blog quality scan** | `examples/quality_scan_blogs.json` | Hero demo — map fanout over published MDX, verdict matrix | 🟡 Spec landed; live proof + manifest TBD |
| 2 | **GEPA trace annotate** | `examples/gepa_trace_annotate.json` | Optimizer traces → theme clusters → proposer context | ⛔ To author |
| 3 | **GELO trace annotate** | `examples/gelo_trace_annotate.json` | Same corpus → theme detection / saturation signals | ⛔ To author |
| 4 | **SMR ReportBench trace evaluate** | `examples/smr_reportbench_trace_evaluate.json` | Map over SMR/ReportBench run traces → rubric verdicts + effort report | ⛔ To author |

**Harness refs:** (1) `frontend/content/blog` via `blog.expand`; (2–3) GEPA/GELO
`generation_N/` rollout dirs + `optimizers/skills/gepa/SKILL.md`; (4)
`evals/reportbench/lanes/readme_smoke/` + SMR hello-world E2E traces
(`run_smr_hello_world_e2e.py`).

**Not in the release blog:** docs quality scan, DungeonGrid, essay skeleton,
Craftax GEPA A/B (separate proof / engineering note later).

### Execution plan (operator — locked order)

Do these in sequence. **Blog is step 3** — after GEPA/GELO and DungeonGrid are tested locally.

| Step | What | WS / milestone | Exit |
|---|---|---|---|
| **1** | **Test workflows in GEPA + GELO** | WS8 | `gepa_trace_annotate` + `gelo_trace_annotate` specs run locally; manifests replay; optimizers consume `state/jesterky_*` or equivalent hook proven on a real generation dir |
| **2** | **Test workflows on DungeonGrid policy** | WS8 + workload port | jesterky topology runs DungeonGrid 4p (mloky parity path): long-horizon sessions, replay ok; policy gap framed honestly — runtime proof, not objective solved |
| **3** | **Ship the blog post** | WS5 | Four locked examples proofed · Mintlify L3 · Josh: GitHub public → review → `status: live` |
| **4** | **GEPA + GELO in prod/OSS** | WS9 | Trace-annotate workflows wired into hosted + local optimizer paths in `optimizers`; launch evidence per `hosted-optimizers.md` |
| **5** | **Hosted workflows — Synth Cloud** | WS7 / M4 | Rust runner behind backend API; conformance-identical streams; prod smoke + `launch_checklist.md` |
| **6** | **Hosted SDK — `synth-ai`** | WS7 / M4 client | `workflows` namespace: spec-in / manifest-out; no embedded runtime |

### Then the other stuff (not on the critical path above)

- Stack cockpit — register / launch / inspect / replay / compare (WS6 / M3)
- SMR ReportBench trace evaluate — blog example #4 if not done in step 3; else cookbook follow-on
- Craftax GEPA A/B — heldout proof using `gepa_trace_annotate` in anger
- Workflow optimizer product (WS8b / M5), MAPO trace wiring, essay skeleton
- Docs quality scan, Loopy/event triggers, terminal viz `--follow` (WS4)
- GitHub org public is a **step 3 gate**, not step 1

### Done ✅

- M0 contract — schema, drift guard, crates.io + PyPI types @0.1.0
- M1 runtime — fake quality scan, replay, CLI (`run` / `replay` / `validate` / `schema` / `visualize`)
- M2 live scan — DeepSeek proxy, concurrency 4, replay ok (operator-proven)
- mloky parity gate — outcome-layer conformance (`conformance.rs`)
- OSS publish — all 6 Rust crates + PyPI `jesterky` @0.1.0 live
- Fake E2E proof — `proof/quality_min.manifest.json` committed
- Blog draft — `blog/jesterky-launch.md` (`status: draft`, `feature_release`)
- Changelog — `CHANGELOG.md` @0.1.0

### Step 1 — Test workflows in GEPA + GELO (current)

- [ ] Author `examples/gepa_trace_annotate.json` + `examples/gelo_trace_annotate.json` + schemas
- [ ] Rollout ingest adapter — GEPA/GELO `generation_N/` → jesterky `--args`
- [ ] Local test run on real optimizer trace dir → manifest → `jesterky replay` ok
- [ ] GEPA proposer reads annotated context; GELO `themes` slice shows jesterky evidence refs
- [ ] Capture proof under `proof/gepa_trace_annotate.*`, `proof/gelo_trace_annotate.*`

### Step 2 — Test DungeonGrid policy on jesterky

- [ ] Port mloky DungeonGrid 4p topology to jesterky spec (see `mloky` dungeongrid programs + `HANDOFF_workflows_status.md`)
- [ ] Live run: DeepSeek, sessions/mailbox, replay passes (mloky ref: 100 turns, 4p, ~23 min)
- [ ] Do **not** claim objective solved — runtime + replay proof only
- [ ] Capture manifest + replay log under `proof/dungeongrid_*`

### Step 3 — Blog post (after steps 1–2)

- [ ] Blog quality scan live proof — `proof/quality_scan_blogs.live.manifest.json`
- [ ] SMR ReportBench trace evaluate spec + proof (blog example #4) — or document waiver
- [ ] Mintlify cookbook (hero: blog scan) + reference + quickstart
- [ ] Proof page — `frontend` `/resources/proof/jesterky-workflows`
- [ ] Blog checklist + rendered review
- [ ] Josh: `github.com/jesterky` public → flip `status: draft` → `live`

### Steps 4–6 — Prod / hosted (after blog)

See WS9 (optimizer prod), WS7 (hosted backend + `synth-ai` SDK).

**Product catalog:** see **Workflow applications** below — sourced from
`workflows/BUILD.md` + mloky Synth Workflows thread (`019f3ea0-2c10-7ae1-8230-16dc9734319c`).

---

## Workflow applications (product catalog)

**Primary thread:** Review mloky workflow status — id `019f3ea0-2c10-7ae1-8230-16dc9734319c`.

**Authoritative sources (Synth Workflows product notebook, not Stack tweet/blog):**

| Doc | Path |
|---|---|
| Product/build notebook | `../workflows/BUILD.md` |
| Runtime status | `../mloky/HANDOFF_workflows_status.md` |
| Topology expressivity / Claude comparison | `../mloky/TOPOLOGY_EXPRESSIVITY_V2_PLAN.md` |
| Overnight research sweep (24 jobs) | `../mloky/evals/dynamic_workflow_research/overnight_jobs.json` |

### What people use workflows for

| Application | Example / notes | jesterky status | Milestone |
|---|---|---|---|
| **Quality scans** — blogs, docs, product surfaces | `quality_scan.json`, `quality_scan_docs.json`, `quality_scan_blogs.json` | 🟡 M2 proven (live + fake); docs/blogs variants in flight on `feature/jesterky-terminal-viz` | M2 ✅ → launch examples |
| **DungeonGrid / GameBench** — long-horizon agent sessions | mloky 100-turn 4p proven; policy gap honest | ⛔ **Step 2** — port to jesterky | Before blog |
| **Optimizer trace review** — GEPA/GELO | `gepa_trace_annotate`, `gelo_trace_annotate` | ⛔ **Step 1** — test in optimizers locally | Before blog |
| **Effort report workflows** — experiments, artifacts, reports, blogs | Real Synth efforts | ⛔ Then other stuff | Post step 6 |
| **Hosted workflows** — backend API, `synth-ai` client | Same contract local/hosted | ⛔ **Steps 5–6** | After blog |
| **Workflow optimizer** — prompts, routing, rubrics, reducers, loop guards, models | Policy bundle over topology | ⛔ Then other stuff | WS8b |
| **Stack integration** — register, launch, inspect, replay, compare, export | Terminal tree → Stack view | ⛔ Then other stuff | WS6 |
| **Essay skeleton** — essay→skeleton→essay, GANs/LWE research | mloky registered programs exist | ⛔ Then other stuff | — |
| **Docs-quality Codex swarms** — fixed or dynamic auditor fanout | `quality_scan_docs.json` | 🟡 Spec + host config landed; not proof-packetized | M2+ |
| **Event/data-triggered automations** — Loopy/Zakin “run when data changes” | `../loopy` code-first automations | ⛔ Not started | Post-M4; sensors TBD |

**Launch examples (locked for release blog — operator 2026-07-08):**

| # | Application | Spec | Proof path |
|---|---|---|---|
| 1 | Blog quality scan | `examples/quality_scan_blogs.json` | `proof/quality_scan_blogs.*` |
| 2 | GEPA trace annotate | `examples/gepa_trace_annotate.json` | `proof/gepa_trace_annotate.*` |
| 3 | GELO trace annotate | `examples/gelo_trace_annotate.json` | `proof/gelo_trace_annotate.*` |
| 4 | SMR ReportBench trace evaluate | `examples/smr_reportbench_trace_evaluate.json` | `proof/smr_reportbench_trace_evaluate.*` |

Docs quality scan, DungeonGrid, essay skeleton, and Craftax A/B are **out of scope**
for the release blog (may ship as follow-on cookbooks or engineering notes).

### Competitive frame

| System | Strength | Synth / jesterky angle |
|---|---|---|
| **Claude dynamic workflows** | Flexible JS scripting, loops, branching, subagents | **JSON topology-as-artifact** — reviewable, diffable, replayable; not arbitrary script as source of truth |
| **Loopy / Zakin** | Code-first automations, Markdown/YAML, sensors/webhooks, typed outputs | Stronger event-stream / replay / optimizer path; Stack + effort artifacts |
| **Slate** | Swarm-native terminal UX, parallel agents | Topology-as-artifact + optimizer compatibility + research/eval workflows |
| **Synth Workflows (jesterky)** | Pinned contract, Addr clock, replay, optimizer-first schema, Stack integration | Cover Claude's *outcomes* via topology + registered ops — see JS decision below |

**Claude scope target** (from `TOPOLOGY_EXPRESSIVITY_V2_PLAN.md`): saved workflow commands, structured args, reviewable generated JSON, reusable agents/program ops, map/reduce, bounded fix-until-pass, verifier/adversarial/voting/ranking, background inspection, pause/resume/stop, isolated worktrees, hooks, caps, arbitrary **bounded** coordination graphs — **without** making JSON a scripting language.

### JS scripting support — decision (defer inline JS)

**Question:** should jesterky add JavaScript scripting like Claude dynamic workflows?

**Default: no arbitrary JS embedded in workflow JSON** — same discipline as mloky V2 and jesterky M0:

| Approach | Verdict | Rationale |
|---|---|---|
| **Inline JS/Python/shell in topology JSON** | ⛔ **Reject** | Breaks replay, optimizer schema, reviewability; workflow JSON becomes unauditable source |
| **Topology combinators** (`map`, `for_each`, `while`, `session_group`, …) | ✅ **Have / extend** | jesterky-core already ports V2 node kinds; covers loops/branching as declarative graph |
| **Registered `program` operations** (Rust, versioned contract) | ✅ **Primary escape hatch** | `jesterky-quality` hosts (`blog`, `docs`, `host`); new ops e.g. `trace.classify.v1`, `optimizer.ingest_gepa.v1` |
| **Compile-to-JSON authoring layer** | 🟡 **Optional later** | NL or JS *generates* reviewed topology JSON — artifact on disk is still JSON, not live script |
| **Sandboxed JS/WASM as registered op** | 🟡 **YAGNI** | Only if a concrete consumer needs in-process dynamic logic; same bar as rejected pyo3 runtime |

**Borrow from Claude without JS:** subagents → `map` + actors; branching → topology guards + `while`; intermediate results → ledger + `ArtifactEmitted`; large graphs → bounded coordination nodes + caps in runplan.

**Action:** log with `jsk decision` if impact ≥ med when a consumer blocks on JS. Until then, M5a (`gepa_trace_annotate`) ships as **topology + Rust program ops**, not a script surface.

**Open question #12:** event-triggered automations (Loopy lane) — sensors/webhooks are a separate product surface from jesterky substrate; do not conflate with M2 ship.

---

## Executive summary

**Head:** `main` @ `f39d8da` (2026-07-08). **Goal cleared** — launch substrate +
packages shipped; story held on Josh gates.

| Layer | Status | Next ship event |
|---|---|---|
| **Code (runtime)** | M1 + M2 **done** — live DeepSeek scan + replay proven | Tune scanner prompt; commit live manifest to `proof/` |
| **OSS publish** | **Shipped** — 6 crates @0.1.0 + PyPI `jesterky` @0.1.0 | `cargo install jesterky-cli` smoke on clean machine |
| **Terminal viz** | Post-hoc `visualize` landed; live `--follow` open | PR2 in terminal viz handoff (post-launch OK) |
| **Public story** | **Drafted** — blog + changelog + `proof/` fake packet | Josh: GitHub public → Mintlify cookbook/ref → review blog → `status: live` |
| **Post-launch platform** | **Step 1 current** — GEPA/GELO test → DungeonGrid → blog → prod | See execution plan table at top |

**Execution log (this push):**
- **2026-07-08 · WS0 done** — landed the per-actor output-schema WIP + codex
  `--ephemeral`/`--output-schema`/stdin-null hardening + strict no-tool scanner
  prompts (`95dad43`); tree clean, `cargo test --workspace` green (2 live-codex
  ignored). Cadence: codex `gpt-5.5` xhigh=draft / medium=grind, one reviewed
  cycle = one commit, revert-on-break. Operator order: **1 GEPA/GELO test → 2 DungeonGrid → 3 blog → 4 optimizers prod → 5 hosted Cloud → 6 synth-ai SDK → other**.
- **2026-07-08 · WS1 publish-ready** — `f36eda9` all six crates carry shared
  `[workspace.package]` metadata (v0.1.0, MIT OR Apache-2.0, repo/homepage/authors),
  keywords+categories, version-pinned internal deps, root LICENSE-MIT/LICENSE-APACHE.
  `cargo publish --dry-run -p jesterky-contract` green. `cd5555a` Python client-only
  types package (`python/jesterky`, pydantic v2 codegen'd from the pinned schema via
  `gen.sh`) — imports, validates all example specs, builds sdist+wheel. Note: codex
  timed out at the 9m40s bash wall mid-think (known xhigh failure mode); did this
  bounded, fully-specified slice by hand instead.
- **2026-07-08 · WS2 install + fake-E2E proven** — `cargo install --path
  crates/jesterky-cli` yields a clean ~3 MB `jesterky` binary; `run examples/quality_min.json
  --actor fake` → completed (5 events, 1 recorded); `replay` → ok. Captured in `proof/`.
  Still owed for WS2: the mloky parity gate (needs the parity-mapping judgment + oracle).
- **2026-07-08 · WS2 parity gate GREEN** (`8d48d02`) — `crates/jesterky-quality/tests/conformance.rs`.
  Mapping judgment: mloky & jesterky don't share an event vocabulary (mloky domain events vs
  Addr-keyed contract), so parity is asserted at the transferable **outcome** layer, NOT event
  bytes — conservation (`jobs_started==jobs_completed==jobs_in_report`) + termination (completed/
  all-ok). Oracle = a real recorded mloky run checked in as `fixtures/mloky_scan_reference.jsonl`
  (8 jobs); test validates the oracle THEN asserts jesterky matches. `cargo test -p jesterky-quality
  --test conformance` → 2 ok.
- **2026-07-08 · WS5 content DRAFTED** (`1bc2f01`) — `blog/jesterky-launch.md` (feature_release,
  feature-first, every claim → a `proof/` command, ≥1 honest limitation, `status: draft`) +
  `CHANGELOG.md` (0.1.0 across the three trains). Verified the blog Try-it chain end-to-end
  (run/visualize/replay on quality_scan: 52 events, 9 recorded, replay ok).
- **2026-07-08 · WS1+WS2 PUBLISHED** (`bc4d054`…`f39d8da`) — all six Rust crates @0.1.0 on
  crates.io; PyPI `jesterky` @0.1.0 live. `scripts/publish.sh` runbook landed. crates.io
  rate-limited new crates (~5 then 429); `jesterky-cli` cleared on auto-retry. PyPI via `twine`
  + `~/.pypirc` (`uv publish` wanted token env/OIDC).
- **2026-07-08 · WS3 live E2E PROVEN (operator + eng)** — DeepSeek proxy scan works at
  **concurrency 4** (~30s): `status=completed`, 52 events, 9 recorded, `replay ok`. Fixes that
  unlocked parallel: `stdin=null` on codex subprocess, `--ephemeral`, per-actor
  `--output-schema`, tightened no-tool prompts (`95dad43`). **Caveats:** (1) committed `proof/`
  still has fake E2E only — live manifest not checked in; (2) strict no-read prompts yield
  label-only/hollow verdicts (substrate proof ≠ real audit); (3) use absolute `--args` target
  paths, not `~/…`; (4) optional next: bounded-read prompt + capture manifest under `proof/`.

**⚠ Josh-gated (irreversible / outward-facing — do NOT auto-execute):**
`cargo publish` to crates.io · PyPI upload of the `jesterky` package · making the
`github.com/jesterky` org+repo public · flipping the launch blog live. Everything
above is driven to publish-READY; these four are the finish-line triggers.

**PUBLISH STATUS (2026-07-08, Josh authorized "publish crates now"):**
- ✅ crates.io: `jesterky-contract`, `-core`, `-actor`, `-model`, `-quality` @0.1.0 LIVE.
- ✅ `jesterky-cli` @0.1.0 LIVE — cleared the rate limit and published on auto-retry.
  All six crates now live on crates.io.
- ✅ PyPI `jesterky` @0.1.0 LIVE → https://pypi.org/project/jesterky/0.1.0/ (Josh
  authorized; uploaded via `twine` using `~/.pypirc` — `uv publish` needs a token env).
- ⛔ `github.com/jesterky` org+repo public — HELD by Josh ("no github just yet").
- ⛔ blog `status: draft` → live — not flipped (manual).

**Blog pattern (operator preference):** ship as a **feature release** — lead with
what shipped and how it works; **mention applications at the end** (Stack,
hosted optimizers, research engineering, agent infra). Not a launch post unless
jesterky is reframed as a major new product surface.

---

## Authoritative doc index

Read these in order when building the plan.

### Jesterky — product & technical

| Doc | Path | What it covers |
|---|---|---|
| **Roadmap (M0–M6)** | `../mloky/ROADMAP_jesterky.md` | Milestones, release trains, DoD, critical path |
| **Rebuild ADR / design** | `../mloky/HANDOFF_jesterky_rust_rebuild.md` | Contract shape, Addr clock, core/host seam, pyo3 superseded |
| **README + quickstart** | `README.md` | Crate map, CLI commands, remaining gates |
| **Round 4 ship state** | `HANDOFF_jesterky_round4_codex.md` | M1 fake-actor surface, what shipped in cycle 3–5 |
| **M2 live scan** | `HANDOFF_jesterky_round6_live_scan.md` | DeepSeek proxy setup, live run commands, debug points |
| **Terminal viz** | `HANDOFF_jesterky_terminal_viz.md` | mloky parity viz slices (PR1–3), gaps vs today |
| **This doc** | `HANDOFF_jesterky_release_and_blog_plan.md` | Integrated release + content plan inputs |
| **Workflows product notebook** | `../workflows/BUILD.md` | Applications catalog, competitive frame, launch examples |
| **Topology V2 / Claude scope** | `../mloky/TOPOLOGY_EXPRESSIVITY_V2_PLAN.md` | Expressivity target; no inline scripting in JSON |
| **Overnight workflow research** | `../mloky/evals/dynamic_workflow_research/overnight_jobs.json` | 24-job architecture sweep |

### mloky — reference oracle (conformance + viz + workload)

| Doc | Path | What it covers |
|---|---|---|
| **Workflow status** | `../mloky/HANDOFF_workflows_status.md` | mloky GREEN surfaces, terminal-first viz decision |
| **Swarm visualization** | `../mloky/HANDOFF_swarm_visualization.md` | North-star terminal tree shape, mloky renderer map |
| **DeepSeek quality swarm** | `../mloky/HANDOFF_deepseek_quality_swarm.md` | Proxy chain, model block, scan topology |
| **Quality scan workflow** | `../mloky/evals/quality_scan/quality_scan.workflow.json` | mloky topology to parity against |
| **Terminal renderer (code)** | `../mloky/src/mloky/rendering/terminal_tree.py` | `RunView` / adapters / ANSI layout to port |
| **Follow CLI (code)** | `../mloky/scripts/run_visual.py` | `--follow` live redraw pattern |
| **Scan runner (code)** | `../mloky/scripts/scan_quality.py` | Live quality-scan panel during map-reduce |

### Synth blog & content strategy

| Doc | Path | What it covers |
|---|---|---|
| **Blog strategy handoff** | `../Jstack/.jstack/daily_notes/2026-07-08/HANDOFF_synth_blog_strategy_20260708.md` | External/internal taxonomy, routing, metadata, gates |
| **Blog strategy (full draft)** | `../Jstack/.jstack/daily_notes/2026-07-07/synth_blog_strategy.md` | Audience lanes, templates, distribution matrix |
| **OpenAI articles research** | `../Jstack/.jstack/daily_notes/2026-07-07/openai_articles.md` | Reference audit inputs |

### Quality gates (must wire into plan)

| Doc | Path | What it covers |
|---|---|---|
| **Blog post checklist** | `../Jstack/.jstack/quality/blog_post_quality_checklist.md` | Pre-publish blog gates (BP-A/B/C…) |
| **Release & launch standards** | `../Jstack/.jstack/quality/release_launch_standards.md` | Announce-after-ship, versioning, L1/L3 blockers |
| **Cookbook standards** | `../Jstack/.jstack/quality/cookbook_submission_standards.md` | Runnable recipe bar for docs/examples |
| **Owned content funnel** | `../Jstack/.jstack/growth/channels/owned-content.md` | Campaign evidence, ODR-18/20, instrumentation |

### Prod launch (if blog touches backend/frontend)

| Doc | Path | What it covers |
|---|---|---|
| **Prod launch checklist** | `../synth-dev/deployment/launch_checklist.md` | P0 blockers before prod merge/deploy |

### Platform consumers (M3–M6 — post-M2)

| Doc | Path | What it covers |
|---|---|---|
| **Stack roadmap** | `../Jstack/.jstack/product/roadmaps/stack.md` | Stack phases; workflow server sketch |
| **Hosted optimizers** | `../optimizers/docs/hosted-optimizers.md` | GEPA/GELO/MAPO hosted launch evidence |
| **MAPO crate** | `../optimizers/rust/crates/synth_mapo/README.md` | Multi-Agent Policy Optimizer; beta entrypoints |
| **Mintlify docs repo** | `../docs/docs/docs.json` | Nav + routing for cookbook/reference pages |
| **Docs quality scan** | `examples/quality_scan_docs.json` | Audit Mintlify MDX before jesterky docs ship |
| **Craftax GEPA hillclimb** | `../evals/stackeval/effort_bench/craftax-agent-hillclimb.toml` | M5a A/B harness + budget floor |
| **GameBench Craftax Rust** | `../gamebench/tasks/craftax-singleplayer/HANDOFF_RUST.md` | Gold service + ReAct adapter contract |
| **GEPA proposer skill** | `../optimizers/skills/gepa/SKILL.md` | Proposer workspace files M5a extends |

---

## Release model (from roadmap)

### Three independent version trains

| Train | Packages | First public ship |
|---|---|---|
| **contract** | `jesterky-contract` Rust + Python types (codegen from schema) | **M0** — leads everything |
| **runtime** | `jesterky` core, CLI, `jesterky-actor`, `jesterky-model`, `jesterky-quality` | **M1** fake actors → **M2** real |
| **client** | `synth-ai` HTTP client | **M4** hosted |

**Decided:** no pyo3. Python = contract types only. Local run = Rust CLI. Hosted =
Rust service (M4).

**Non-waivable gate (every release):** conformance suite green; from M2 onward,
mloky parity on agreed fixture.

### Milestone → public ship map

```
M0 contract ──┬── M1 core ──┬── M2 real actors ──┬── M3 Stack ──┐
              │             │                    └── M4 Cloud ──├── M5 optimizer ── M6 GEPA/GELO
```

| Milestone | OSS ship | DoD highlight |
|---|---|---|
| **M0** | `jesterky-contract` 0.1.0 crates.io + PyPI; published schemas; `github.com/jesterky` org | mloky streams validate; mloky frozen to bugfixes |
| **M1** | `jesterky` 0.1 crate + `jesterky` CLI binary | Fake quality scan replay byte-identical; **mloky parity gate** |
| **M2** | Nightly dev runtime; real 8-verdict scan | Live run matches mloky V2; no orphaned codex; proxy in image |
| **M3** | Stack Workflows primitive | register→launch→inspect→replay→compare |
| **M4** | Hosted runner + `synth-ai` client GA | Hosted streams conformance-identical to local |
| **M5a** | Optimizer trace pipeline (GEPA/GELO annotate) | Craftax GEPA A/B wins OR documented comparison; replayable annotation manifests |
| **M5** | Workflow Optimizer product | Process-tree summarize/classify/diff (productized) |
| **M6** | GEPA/GELO/MAPO verifier loop | Verifier-driven bundle improvement on held-out signal |

Roadmap quote: *"Nothing **real** ships until M2, but M0 contract and M1 OSS
crate with fake actors are genuine early releases."*

**Integration status (2026-07-08):** `stack`, `backend`, and `optimizers` contain
**zero** `jesterky` references. M3–M6 are roadmap-planned only; the 0.1.0 ship is
substrate + content, not platform wiring.

### M3 — Stack Workflows primitive

**Ship:** Workflows visible and operable in Stack cockpit; staging → owner A+ → prod.

**Depends on:** M2 host actor proven (✅). Can parallel with M4. Terminal viz
(`WS4`) is demo prep, not Stack integration.

| Acceptance test | Pass criterion |
|---|---|
| **Register** | Stack persists a jesterky workflow spec (topology JSON + metadata) and returns a stable workflow id |
| **Launch** | `stack_*` MCP or TUI path launches a local or remote jesterky run from a registered spec; run id correlates to Stack session/Effort ledger |
| **Inspect** | Stack shows run status, event count, terminal state, and manifest path/URI without scraping raw logs |
| **Replay** | Stack triggers `jesterky replay` (or equivalent API) on a finished manifest; fidelity matches local CLI |
| **Compare** | Stack diffs two manifests on outcome + selected trace fields (conservation, termination, scores) |
| **Visualize** | Stack renders `ProcessNode` tree (terminal-tree parity or Stack TUI adapter) for finished runs |
| **Staging proof** | Real quality-scan run launched from Stack on staging; receipt captured with run id + commit SHA |
| **Prod gate** | Owner-calibrated A+; no Stack minor bump without explicit ask |

**Not in M3:** mermaid/box-graph viz (parked in mloky); SMR/Factory workflow
replacement — jesterky is a new Stack primitive alongside existing Efforts/SMR paths.

### M4 — Hosted workflows (prod)

**Ship:** Rust hosted runner (`jesterky-core` behind HTTP/gRPC); `synth-ai` thin
client GA; prod deploy.

**Depends on:** M2 conformance + M0 contract types in `synth-ai`. Can parallel with M3.

| Acceptance test | Pass criterion |
|---|---|
| **API surface** | Submit spec → poll/stream events → fetch manifest; OpenAPI documented in Mintlify |
| **Conformance** | Hosted event/artifact stream byte-validates against `jesterky.schema.json`; round-trip identical to local run on same spec + recorded outputs |
| **Auth** | Per-org API key enforced; cross-org manifest access denied |
| **Identity** | Run id + org id on every event; retire `X-Scan-Run-ID` billing hack |
| **Billing** | Per-run token/cost attribution correct on a staged billing fixture |
| **Client** | `synth-ai` submits hosted run using contract types only — **no embedded runtime** |
| **Prod smoke** | Hosted quality-scan completes on prod API; manifest replays locally with `jesterky replay` |
| **Launch gate** | `synth-dev/deployment/launch_checklist.md` P0 blockers cleared if blog promotes hosted path |

### M5 — Workflow Optimizer (trace consumption)

**Ship:** Optimizer product that consumes jesterky runs as **process objects** —
summarize, classify, score, diff — on hosted + local manifests.

**Depends on:** M2 `ProcessNode` trace stable; M4 for hosted-scale runs (local-only
proof acceptable for first ship).

| Acceptance test | Pass criterion |
|---|---|
| **Ingest** | Optimizer reads a finished manifest + event stream via `jesterky-contract` types (no log scraping) |
| **Process tree** | Summarize/classify/score operates on `ProcessNode` nodes, not flat JSONL grep |
| **Diff** | Two runs of same spec produce a typed diff (outcome, per-shard scores, failure class) |
| **Wall-safe** | Optimizer signals use wall-safe fields only; no Chinese-wall violation |
| **Hosted + local** | Same ingest path for local manifest file and M4 hosted manifest URI |
| **Proof** | Held-out manifest pair with documented diff output checked into evidence packet |

**Products:** not `jesterky-*` crates — separate optimizer surface that **depends on**
`jesterky-contract`.

### M6 — GEPA / GELO / MAPO over workflow traces

**Ship:** Verifier-driven improvement loop closing on real workflow/policy bundles.

**Depends on:** M5 trace consumption; existing GEPA/GELO/MAPO optimizer infra
(today: container/env rollouts, **not** jesterky manifests).

| Surface | Today (pre-M6) | M6 target | Acceptance test |
|---|---|---|---|
| **GEPA** | Prompt optimization over eval containers; local OSS + hosted on api.usesynth.ai | Consumes jesterky manifest as training/eval example; proposes workflow/policy levers | GEPA run ingests manifest; frontier improvement measured on held-out replay score |
| **GELO** | Go-Explore hosted-only (public); env-backed prompt-space search | Trace-guided exploration over workflow bundles | Hosted GELO job references jesterky run id; board slice shows workflow lever candidates |
| **MAPO** | Multi-Agent Policy Optimizer (`synth_mapo`); protocol/role/shared-context search over env rollouts; entrypoints in `optimizers-beta` until sign-off | MAPO candidates evaluated against jesterky-recorded multi-agent workflow traces (not just DungeonGrid env) | MAPO heldout comparison uses jesterky manifest outcomes; champion beats baseline on agreed metric |

**M6 gate / DoD (all optimizers):**
- Verifier grades **outcomes only** (Chinese wall intact)
- Measured bundle improvement on held-out signal
- ZVPO gold I/O pairs bootstrap verifiers where applicable
- Receipt-backed evidence per `optimizers/docs/hosted-optimizers.md` launch checklist

**MAPO note:** MAPO is not a separate jesterky milestone — it ships under the M6
optimizer loop as the multi-agent policy variant. DungeonGrid Plus remains MAPO's
current launch gate; jesterky wiring is a follow-on acceptance row.

### M5a — Optimizer trace pipeline (first application after direct workflows)

**Operator decision (2026-07-08):** the first consumer after M2 CLI workflows is
**not** Stack or hosted — it is **mass trace processing for GEPA and GELO**:
annotate rollout batches, cluster failure themes, and feed richer proposer context.
M3/M4 can run in parallel but do not gate this slice.

**Ship:** jesterky workflows that ingest optimizer rollout traces and emit
structured annotation artifacts consumable by GEPA proposer workspaces and GELO
theme engines — all via local CLI (no Stack/hosted required).

**Depends on:** M2 substrate + `jesterky-contract` types only. Reuses existing
GEPA/GELO rollout exports (`events.jsonl`, `proposer_failure_summary.json`,
reflection rows).

| Consumer | What jesterky adds today | Workflow output (target artifacts) |
|---|---|---|
| **GEPA proposer** | Flat `proposer_failure_summary.json` + `proposer_examples.json` rows | Per-trace annotations, theme clusters, shard-level failure taxonomy, **`jesterky_proposer_context`** bundle merged into proposer workspace (`state/jesterky_*.json`) |
| **GELO theme engine** | Theme board from env rollouts + verifier agents | Mass-annotated trace corpus, saturation signals, **`theme_registry`** with evidence links to replayable trace shards |

**Topology sketch (to implement as `examples/gepa_trace_annotate.json`):**

```
expand(rollout_batch) → map[trace_shard]:
    actor: trace_classifier   # theme, failure_class, action_pattern, reward_band
  → reduce:
    program: theme_cluster    # merge shards → ranked themes + exemplar addrs
  → actor: context_writer     # proposer-facing narrative + structured excerpts (wall-safe)
```

**Wall-safe rule:** annotations may summarize rollout **outcomes and public trace
fields** only — no heldout labels, no optimizer-internal paths that leak
selection signal across the Chinese wall. Verifier grades outcomes; jesterky
structures evidence for the proposer.

| Acceptance test | Pass criterion |
|---|---|
| **Ingest** | Workflow accepts a GEPA generation directory (`events.jsonl` + reflection exports) via `--args` ledger seed |
| **Scale** | Map over ≥64 trace shards (Craftax train batch) completes with conservation + termination |
| **Replay** | Annotation manifest replays byte-faithfully; re-run produces identical theme registry |
| **GEPA hook** | Optimizers proposer reads `state/jesterky_proposer_context.json` (or `.md`) without workspace leak check failures |
| **GELO hook** | `gelo watch --slice themes` shows themes whose `evidence_refs` point at jesterky manifest addrs |
| **A/B proof** | Craftax arm B beats arm A on primary metric OR documents why not (see below) |

### M5a proof — Craftax GameBench Rust GEPA A/B (with / without trace workflow)

**Hypothesis:** processing GEPA rollout traces through a jesterky annotation
workflow gives the proposer **richer, clustered context** → better prompt proposals
→ higher heldout reward on GameBench Craftax Rust.

**Harness (existing):** `evals/stackeval/effort_bench/craftax-agent-hillclimb.toml`
- Env: GameBench Craftax **Rust gold** (`gamebench/tasks/craftax-singleplayer/gold_rust`)
- Adapter: ReAct synth task-app Docker container (not bare env server)
- Optimizer: local GEPA (`optimizer.gepa.local`)
- Primary metric: `best_heldout_mean_reward` vs `seed_heldout_mean_reward`
- Heldout: seeds 501–564 (`gamebench-rust-v1` registry)
- Budget floor: ≥4 generations, ≥512 rollouts, ≥6 proposals/gen (per effort config)

| Arm | Trace processing | Proposer context |
|---|---|---|
| **A — baseline** | None (status quo) | `proposer_failure_summary.json`, `proposer_examples.json`, `proposer_repair_hints.json` only |
| **B — jesterky** | After each generation's train rollouts: `jesterky run examples/gepa_trace_annotate.json --args '{"rollout_dir":"…/generation_N"}' --out …/trace_annotate.manifest.json` | Arm A files **plus** `state/jesterky_theme_registry.json`, `state/jesterky_trace_annotations.jsonl`, `state/jesterky_proposer_context.md` materialized into proposer workspace before proposer round |

**Controlled variables (must match across arms):**
- Same `gepa_config_path` / preset / proposer model
- Same train + heldout seed registries
- Same container build (Rust gold port 8098 + ReAct adapter)
- Same budget floor (`craftax-agent-hillclimb.toml` `[budget_floor]`)
- Same EffortBench packet structure for evidence comparison

**Primary success criterion:** Arm B `best_heldout_mean_reward` > Arm A on the
same heldout registry, with `prompt_accepted=true` and full
`local_gepa_evidence.json` floor fields populated.

**Secondary signals (explain mechanism if primary is flat):**
- Proposer acceptance rate (proposals accepted / proposed)
- Generations to first heldout improvement over seed
- Theme cluster count + intra-cluster coherence (manual or rubric on `theme_registry`)
- Proposer input tokens vs heldout delta (efficiency)

**GELO follow-on (same trace corpus, phase 2):**
- Run identical annotated corpus through a GELO-oriented jesterky workflow
  (`examples/gelo_theme_annotate.json` — TBD) targeting theme detection/saturation
- Compare `gelo watch --slice themes` board quality: tentative → mature theme
  promotion rate, saturation threshold hits, objective_score progression
- Does not block Craftax GEPA A/B; runs after arm A/B completes

**Evidence packet (attach to plan / Effort finding):**

```
proof/craftax_gepa_ab/
  arm_a/local_gepa_evidence.json
  arm_b/local_gepa_evidence.json
  arm_b/trace_annotate.manifest.json   # replayable
  comparison.md                      # seed vs best, heldout means, secondary signals
  commit_sha + gepa_run_ids
```

**Implementation order (WS8):**
1. Author `examples/gepa_trace_annotate.json` + output schemas (verdict + theme registry)
2. Adapter: GEPA generation dir → jesterky `--args` inputs (script or small Rust bin)
3. Optimizers hook: copy `jesterky_*` artifacts into proposer workspace pre-round
4. Run arm A (baseline) to completion on Craftax hillclimb packet
5. Run arm B with workflow between generations
6. Document comparison; decide whether to promote trace pipeline to default GEPA path

### Milestone dependency graph (full ladder)

```
M0 contract ──┬── M1 core ──┬── M2 direct workflows (CLI) ──┬── M5a optimizer traces ★ FIRST APP
              │             │                               │     (GEPA/GELO annotate)
              │             │                               ├── M3 Stack ────────┐
              │             │                               ├── M4 Hosted prod ──├── M5 product ── M6 verifier loop
              │             │                               │                    │
              │             └── ★ P2–P3 content (blog + Mintlify + proof) — fires at M2
              └── optimizer-schema review in M0 (feeds M5a/M5/M6)
```

---

## Content surfaces & distribution (feature release + Mintlify)

Per Synth blog strategy (`HANDOFF_synth_blog_strategy_20260708.md`): **blog =
narrative; Mintlify = procedural truth.** They ship as a bundle for M2, not as
one artifact.

### Routing for this ship (`feature_release`, not `launch_post`)

| Artifact | Internal type | Canonical surface | Repo / path | Milestone gate | Status |
|---|---|---|---|---|---|
| **Main post** | `feature_release` | Blog / Product | `frontend` ← `blog/jesterky-launch.md` | M2 proof + packages live; blog checklist pass | 🟡 Draft |
| **Version delta** | `release_note` | Changelog | `jesterky/CHANGELOG.md` (+ website mirror TBD) | Same | ✅ 0.1.0 |
| **Runnable recipe** | `cookbook` | **Mintlify Docs / Examples** | `docs` repo (e.g. `examples/jesterky-quality-scan`) | Pinned `cargo install jesterky-cli --version 0.1.0`; fake path required; live path optional | ⛔ Open |
| **CLI + contract reference** | `machine_readable_docs` | **Mintlify Docs / Reference** | `docs` repo (CLI commands, schema, manifest shape) | Matches published 0.1.0 CLI surface | ⛔ Open |
| **Quickstart guide** | `tutorial` or Docs / Guides | **Mintlify Docs / Guides** | `docs` repo | Navbar discoverability; links to cookbook | ⛔ Open |
| **Evidence packet** | `proof_page` | Resources / Proof | `frontend` `/resources/proof/jesterky-workflows` | Four locked examples + commands | 🟡 Fake min only |
| **Applications teaser** | (end of feature_release) | Blog only | Same post § "What it is for" | **Four locked examples** — blogs scan, GEPA/GELO trace annotate, SMR ReportBench trace evaluate | 🟡 Draft needs update |

### Where each surface fits in the timeline

| Phase | Content work | Blocks blog `status: live`? |
|---|---|---|
| **P2 — Content bundle (WS5)** | Draft blog ✅ · changelog ✅ · cookbook + Mintlify ref pages · proof page | Cookbook + proof page are **L3 announce-after-ship** blockers per release standards — Mintlify must reference published `0.1.0` before flip |
| **P3 — Ship story** | Josh: GitHub public → rendered review (blog + Mintlify mobile/desktop) → flip `status: live` | Yes — Josh gates |
| **P5+ — M3 content** | Optional `feature_release` or `product_update` when Stack primitive ships; Stack docs page in Mintlify | No — future post |
| **P6+ — M4 content** | Hosted API reference in Mintlify; `synth-ai` hosted-workflow guide; migration from local CLI | No — future post |
| **P7+ — M5a content** | Craftax A/B proof page or engineering note if measured win | No — evidence for optional follow-on post |

**Mintlify specifics:** docs live at `docs.usesynth.ai` (`docs` repo, Mintlify
`docs.json` nav). jesterky pages should land under a stable product section (nav
label TBD — see open question #4). The repo already has a `quality_scan_docs`
workload (`examples/quality_scan_docs.json`) for auditing Mintlify MDX against
family-2 docs standards — use it to keep jesterky docs pages honest before publish.

**Announce-after-ship (L3):** blog `Try it` commands must match Mintlify cookbook
steps and pinned package versions. README + crates.io/PyPI are sufficient for
install proof; Mintlify is the canonical **how-to** once the cookbook lands.

---

## Current code state (2026-07-08)

Latest commits on `main` @ `f39d8da` (tree clean):

| Area | Status | Notes |
|---|---|---|
| **Contract types + schema** | ✅ Done | `jesterky.schema.json`, drift guard, round-trip + conformance |
| **Core runner** | ✅ Done | All node kinds, Addr clock, serial/parallel map (concurrency 4 in scan) |
| **Replay** | ✅ Done | `ReplayActor`; fidelity = addr + kind + payload (wall_ms metadata) |
| **CLI** | ✅ Done | `run`, `replay`, `validate`, `schema`, `visualize`; published @0.1.0 |
| **Quality workload** | ✅ Done | `jesterky-quality` + `examples/quality_scan.json` + verdict/summary schemas |
| **Real actor** | ✅ Done | `CodexModel`: `--ephemeral`, per-actor `--output-schema`, `stdin=null`, `CODEX_HOME` |
| **Args seeding** | ✅ Done | `--args` → ledger (`target` for expand) |
| **Terminal viz** | 🟡 Partial | Post-hoc `visualize` + btop panel; live `--follow` not wired |
| **M0 publish** | ✅ Done | crates.io contract + schemas; PyPI client types @0.1.0 |
| **M1 parity gate** | ✅ Done | Outcome-layer gate (`conformance.rs`); **not** event-byte equality |
| **M2 live proof** | 🟡 Proven, not packetized | Operator runs pass; `proof/` lacks committed live manifest |
| **Packaging** | 🟡 Partial | crates.io + PyPI live; **GitHub org+repo held**; no GH release asset yet |
| **Python runtime** | ⛔ Out of scope | Client types only — **no pyo3** (decided) |
| **M3 Stack workflows** | ⛔ Not started | No `jesterky` refs in `stack` |
| **M4 Hosted prod** | ⛔ Not started | No hosted runner; no `jesterky` refs in `backend` |
| **M5a Optimizer traces** | ⛔ Planned — first app | Craftax GEPA A/B spec below; no workflow spec yet |
| **M6 GEPA/GELO/MAPO** | 🟡 Products exist separately | GEPA/GELO/MAPO live on containers; not wired to jesterky traces |
| **Mintlify docs** | ⛔ Not started | Cookbook + CLI reference pages open (WS5) |

---

## Content / blog routing (jesterky)

See **Content surfaces & distribution** above for the full Mintlify + blog map.
Summary for this ship:

| Artifact | Internal type | External home | Owner in plan |
|---|---|---|---|
| Main post | `feature_release` | Blog / Product (`frontend`) | Eng + content |
| Version delta | `release_note` | Changelog (+ Mintlify mirror TBD) | Eng |
| Runnable path | `cookbook` | **Mintlify Docs / Examples** (`docs` repo) | Eng |
| CLI + schema reference | `machine_readable_docs` | **Mintlify Docs / Reference** | Eng |
| Evidence | `proof_page` | Resources / Proof (`frontend`) | Eng |
| Optional deep dive | `engineering_essay` | Blog / Engineering | Optional second post |

**Routing rule:** `feature_release` + `release_note` + **Mintlify docs** (cookbook
+ reference when CLI/API surface ships) + proof.

**Post shape (blog checklist spine):**

1. Problem and why it matters now
2. Mechanism we built (deterministic workflow substrate, quality scan)
3. Evidence and where it fails (≥1 limitation)
4. How to try it (CLI commands, truthful readiness)
5. **Applications at the end** — four locked examples: blog quality scan,
   GEPA trace annotate, GELO trace annotate, SMR ReportBench trace evaluate
   (each → `proof/` command; Stack/hosted named as future, not shipped)

**Announce-after-ship (release standards — L3 blocker):** blog links packages
already published; Mintlify cookbook runs against pinned `jesterky-cli@0.1.0`;
proof page backs every measured claim.

### Suggested frontmatter (when content repo exists)

```yaml
type: feature_release
surface: blog
product_area: jesterky  # or "Open Research" / "Agent Infrastructure" — confirm canonical nav
claim_tier: measured     # live scan proof required for "measured"; else dev_evidence
proof: /resources/proof/jesterky-quality-scan-<date>
release: jesterky-runtime@0.1.0-dev.YYYYMMDD.N  # pin exact artifact
audience: builder
status: draft
owner: <name>
```

---

## Workstreams for the plan

Add these as plan rows/epics. Dependencies shown.

### WS1 — M0 contract publish

- [x] `jesterky-contract` 0.1.0 → crates.io
- [x] Python types package → PyPI (`jesterky` @0.1.0, codegen from schema)
- [ ] `github.com/jesterky` org + repo public — **HELD by Josh**
- [x] Conformance suite — mloky oracle fixture + outcome parity (`conformance.rs`)
- [ ] mloky runtime freeze (bugfixes only) — coordinate with mloky owner
- [ ] Written versioning policy page (release standards: axis B)

### WS2 — M1 runtime publish

- [x] `jesterky` 0.1 + CLI → crates.io (`cargo install jesterky-cli`)
- [x] **mloky parity gate** — outcome layer (conservation + termination), not `seq`→`Addr` bytes
- [ ] Trusted publishing + attestations (release standards registry section)
- [x] Changelog entry (`CHANGELOG.md` @0.1.0)

### WS3 — M2 live proof (blog evidence packet)

- [x] Stand up proxy `config.toml` (`/tmp/jesterky_codex_home` — see `round6` handoff)
- [x] Live run: `jesterky run examples/quality_scan.json --actor codex …` (4-wide, ~30s)
- [x] Replay passes on live manifest
- [ ] Capture proof artifacts under `proof/` (manifest JSON, replay log, commit SHA)
- [ ] Orphaned `codex` reap verified (M2 DoD) — `kill_on_drop` landed; formal capture pending
- [ ] Scanner prompt: bounded reads (optional) so verdicts are audits, not label-only meta-judgments
- [x] Parallel reliability — `stdin=null`, `--output-schema`, `--ephemeral` (`95dad43`)

**Remaining for measured tier:** commit live manifest to `proof/`; document absolute `--args` target.

### WS4 — Terminal viz (nice for blog/demo, not strict blocker)

See `HANDOFF_jesterky_terminal_viz.md`.

| Slice | Deliverable | Blog impact |
|---|---|---|
| PR1 | Done-ish: `visualize` + btop panel post-hoc | Screenshot for proof page |
| PR2 | `run --follow` live redraw | Demo GIF / live blog moment |
| PR3 | Per-shard progress during real codex | Parity with mloky live panel |

### WS5 — Content bundle (M2 ship — blog + Mintlify + proof)

**Locked release blog examples:** blog quality scan · GEPA trace annotate · GELO
trace annotate · SMR ReportBench trace evaluate (see top of doc).

- [x] **Blog** — `blog/jesterky-launch.md` drafted (`status: draft`, `type: feature_release`)
- [ ] **Blog § examples** — rewrite applications to the four locked workflows + proof commands
- [x] **Changelog** — `CHANGELOG.md` @0.1.0
- [ ] **Example specs** — `gepa_trace_annotate.json`, `gelo_trace_annotate.json`, `smr_reportbench_trace_evaluate.json`
- [ ] **Blog quality scan proof** — live manifest `proof/quality_scan_blogs.live.manifest.json`
- [ ] **Proof packet** — `proof/README.md` sections 7–10: one command per locked example
- [ ] **Mintlify cookbook** — hero: blog quality scan; secondary: three trace workflows
- [ ] **Mintlify reference** — CLI commands, manifest/schema overview
- [ ] **Mintlify quickstart** — install + first fake run (`quality_min` or blogs fake path)
- [ ] **Proof page** — `frontend` `/resources/proof/jesterky-workflows`
- [ ] Pass blog checklist + release standards + cookbook standards
- [ ] Rendered review mobile + desktop — **blog + Mintlify routes**
- [ ] Flip `status: draft` → `live` — **Josh only** (after GitHub hold lifted + L3 bundle green)

**Open (from blog strategy):** `frontend` for blog/proof; **`docs` repo for Mintlify** —
confirm nav label before editing.

### WS6 — M3 Stack Workflows

- [ ] Workflow spec registration API / Stack persistence model
- [ ] MCP + TUI: launch, inspect, replay, compare jesterky runs
- [ ] `ProcessNode` visualization in Stack (reuse terminal-tree adapter or port)
- [ ] Staging proof: quality-scan launched from Stack with receipt
- [ ] Owner A+ → prod promotion
- [ ] Optional content: Stack docs page in Mintlify + `product_update` post

### WS7 — Steps 5–6: Hosted workflows (Synth Cloud + `synth-ai`)

**Step 5 — Hosted backend (Synth Cloud):**
- [ ] Rust hosted runner service (`jesterky-core` behind HTTP/gRPC)
- [ ] OpenAPI + Mintlify hosted-workflow reference
- [ ] Per-org auth, run identity, billing (retire `X-Scan-Run-ID` hack)
- [ ] Conformance: hosted stream identical to local on fixture
- [ ] Prod smoke + `launch_checklist.md`

**Step 6 — Hosted SDK (`synth-ai`):**
- [ ] `workflows` namespace: submit spec, poll/stream events, fetch manifest
- [ ] Client uses contract types only — **no embedded runtime**
- [ ] SDK proof snippet in Mintlify + evidence packet

### WS8 — Steps 1–2: Test GEPA/GELO + DungeonGrid (before blog)

**Step 1 — GEPA + GELO trace workflows (current):**
- [ ] `examples/gepa_trace_annotate.json` + `examples/gelo_trace_annotate.json` + schemas
- [ ] Rollout ingest — GEPA/GELO `generation_N/` → jesterky `--args`
- [ ] Local test on real trace dir → manifest → replay ok
- [ ] GEPA proposer hook (`state/jesterky_*`); GELO themes slice with evidence refs
- [ ] Proof: `proof/gepa_trace_annotate.*`, `proof/gelo_trace_annotate.*`

**Step 2 — DungeonGrid policy (mloky parity):**
- [ ] Port DungeonGrid 4p topology to jesterky (mloky `workflow_v2_examples` / dungeongrid programs)
- [ ] Live run: DeepSeek, sessions, mailbox; replay passes
- [ ] Honest framing: runtime proof, objective not solved
- [ ] Proof: `proof/dungeongrid_*`
- [ ] SMR ReportBench trace evaluate — defer to step 3 blog prep or "other stuff"

**Harness paths:** `optimizers/skills/gepa/SKILL.md`, `mloky/HANDOFF_workflows_status.md` § DungeonGrid

### WS8b — M5 Workflow Optimizer (then other stuff)

- [ ] Manifest ingest via `jesterky-contract` (local file + hosted URI)
- [ ] Process-tree summarize / classify / score / diff
- [ ] Wall-safe signal extraction only
- [ ] Evidence packet with held-out manifest pair diff

### WS9 — Step 4: GEPA + GELO in prod/OSS

- [ ] Wire trace-annotate workflows into local + hosted optimizer paths (`optimizers`)
- [ ] Promote tested step-1 hooks to default GEPA/GELO proposer/theme paths
- [ ] Hosted launch evidence per `optimizers/docs/hosted-optimizers.md`
- [ ] Craftax GEPA A/B — **then other stuff** (heldout proof optional)

**Then other stuff (WS9 continued / backlog):** MAPO trace wiring, verifier loop, workflow optimizer product

---

## Proof packet checklist (for plan acceptance criteria)

Engineer should be able to attach these to the plan as exit criteria:

```bash
# 1. Fake E2E (deterministic, no network)
cargo test
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --out /tmp/qs.fake.manifest.json
cargo run -p jesterky-cli -- replay /tmp/qs.fake.manifest.json \
  --spec examples/quality_scan.json
cargo run -p jesterky-cli -- visualize /tmp/qs.fake.manifest.json \
  --spec examples/quality_scan.json

# 2. Live E2E (M2 — requires proxy + SYNTH_API_KEY)
cargo run -p jesterky-cli -- run examples/quality_scan.json \
  --actor codex --model deepseek/deepseek-v4-pro-direct \
  --codex-home /tmp/jesterky_codex_home --cd /path/to/repo \
  --args '{"target":"/path/to/repo"}' \
  --out /tmp/qs.live.manifest.json
cargo run -p jesterky-cli -- replay /tmp/qs.live.manifest.json \
  --spec examples/quality_scan.json

# 3. Publish smoke (after WS1/WS2)
cargo install jesterky-cli --version <pinned>
jesterky run examples/quality_scan.json --out /tmp/qs.installed.manifest.json
```

Store outputs under a proof path (e.g. Resources / Proof) with run_id, commit
SHA, and date.

### Post-M2 acceptance test index (M3–M6)

Detailed pass criteria live in **M3–M6 milestone sections** above. Summary exit
commands / proofs to attach per milestone:

| Milestone | WS | Primary acceptance proof |
|---|---|---|
| **M3 Stack** | WS6 | Staging receipt: Stack-launched quality-scan → manifest → replay ok; MCP inspect/compare on run id |
| **M4 Hosted prod** | WS7 | Prod API: submit spec → stream events → fetch manifest → local `jesterky replay` conformance match |
| **M5a Traces** | WS8 | Craftax GEPA A/B: arm B heldout mean > arm A OR documented comparison packet |
| **M5 Optimizer** | WS8b | Optimizer diff report on two manifests; wall-safe signal audit pass |
| **M6 GEPA** | WS9 | GEPA run with `manifest_uri` input; held-out replay score improvement vs baseline |
| **M6 GELO** | WS9 | Hosted `gelo submit` referencing jesterky run; `gelo watch --slice board` shows workflow levers |
| **M6 MAPO** | WS9 | `synth_mapo` heldout comparison using jesterky manifest outcomes (after env-rollout sign-off) |

---

## Suggested plan timeline (operator order — 2026-07-08)

| Phase | Step | Workstreams | Status |
|---|---|---|---|
| **P0 — Prove** | — | WS3 | ✅ Live run + replay; 🟡 commit manifests |
| **P1 — Package** | — | WS1 + WS2 | ✅ crates.io + PyPI |
| **P2 — Test GEPA/GELO** | **1** | WS8 | ⛔ **Current** |
| **P3 — Test DungeonGrid** | **2** | WS8 + workload | ⛔ Not started |
| **P4 — Blog** | **3** | WS5 | 🟡 Drafted; proof + Josh gates open |
| **P5 — Optimizers prod/OSS** | **4** | WS9 | ⛔ After blog |
| **P6 — Hosted Cloud** | **5** | WS7 backend | ⛔ After blog |
| **P7 — Hosted SDK** | **6** | WS7 `synth-ai` | ⛔ After P6 |
| **P8+ — Other** | — | WS6 Stack, WS8b, WS4 viz, … | ⛔ After step 6 |

**Finish line (step 3 — blog):** steps 1–2 green → proof packet + Mintlify L3 → Josh: GitHub public → `status: live`.

---

## Open questions (add to plan as decisions)

| # | Question | Default / note |
|---|---|---|
| 1 | Is jesterky `launch_post` or `feature_release`? | **`feature_release`** per operator; apps at end |
| 2 | Bundle M0+M1 publish with blog or ship contract quietly first? | Roadmap: M0 can ship before M2 blog |
| 3 | mloky parity — hard blocker for blog or waivable with dev_evidence tier? | Release standards: prefer measured; document waiver if skipped |
| 4 | Canonical `product_area` nav label? | Stack / Open Research / Agent Infrastructure — confirm for blog + Mintlify nav |
| 5 | Content repo owner? | `frontend` (blog, proof); **`docs` (Mintlify)** |
| 6 | Changelog canonical source — website vs Mintlify? | Blog strategy open question; mirror `CHANGELOG.md` at minimum |
| 7 | Benchmark/proof public at ship or staged? | Blog strategy open question |
| 8 | Mintlify nav placement for jesterky? | New product section vs Agent Infrastructure subsection — confirm before WS5 cookbook |
| 9 | MAPO jesterky wiring vs DungeonGrid launch gate? | MAPO ships env rollouts first (`optimizers-beta`); jesterky traces are WS9 / M6 follow-on |
| 10 | Optimizer traces before Stack? | **Yes (operator)** — M5a/WS8 is first application; Stack (M3) does not gate Craftax A/B |
| 11 | Craftax A/B primary metric? | `best_heldout_mean_reward` on `gamebench-rust-v1` heldout registry; secondary = proposer acceptance + theme coherence |
| 12 | Inline JS scripting in workflow JSON? | **Defer** — topology + registered Rust ops; compile-to-JSON layer optional later (`TOPOLOGY_EXPRESSIVITY_V2_PLAN.md`) |
| 13 | Loopy/event-triggered automations vs jesterky? | Separate lane — sensors/webhooks not M2 substrate; revisit post-M4 |

Log decisions with `jsk decision` when impact ≥ med.

---

## What NOT to put in the plan (explicitly out of scope for M2 blog)

- mloky flat-`seq` parity **mapping design** as part of blog work (it's M1 DoD,
  separate engineering judgment)
- **Implementing** M3–M6 platform wiring as part of the M2 blog sprint (document
  as future milestones WS6–WS9; mention at end of feature_release only)
- **Running** Craftax GEPA A/B (M5a) as part of M2 blog work — it is P5 post-story
  engineering (WS8), though blog may tease the hypothesis at the end
- pyo3 Python runtime
- mermaid/box-graph viz (parked in mloky)
- Prod deploy of backend/frontend unless blog promotion requires it — then run
  `launch_checklist.md`

---

## Quick links for plan doc paste

```
Technical
  workflows/BUILD.md
  mloky/ROADMAP_jesterky.md
  mloky/TOPOLOGY_EXPRESSIVITY_V2_PLAN.md
  mloky/HANDOFF_workflows_status.md
  mloky/HANDOFF_jesterky_rust_rebuild.md
  jesterky/README.md
  jesterky/HANDOFF_jesterky_round6_live_scan.md
  jesterky/HANDOFF_jesterky_terminal_viz.md

Content
  Jstack/.jstack/daily_notes/2026-07-08/HANDOFF_synth_blog_strategy_20260708.md
  Jstack/.jstack/daily_notes/2026-07-07/synth_blog_strategy.md
  Jstack/.jstack/quality/blog_post_quality_checklist.md
  Jstack/.jstack/quality/release_launch_standards.md
  Jstack/.jstack/quality/cookbook_submission_standards.md
  docs/  (Mintlify — cookbook, reference, quickstart for WS5)
  frontend/content/blog/  (feature_release post destination)
  optimizers/docs/hosted-optimizers.md  (M6 GEPA/GELO/MAPO launch evidence)
  optimizers/skills/gepa/SKILL.md  (proposer workspace — M5a hook target)
  evals/stackeval/effort_bench/craftax-agent-hillclimb.toml  (A/B harness)
  gamebench/tasks/craftax-singleplayer/HANDOFF_RUST.md  (Rust gold + adapter contract)
```

---

## Debrief note

The jesterky **release-blog** goal cleared 2026-07-08 after package publish.
Substrate + install path + live DeepSeek scan are real; the story is one explicit
Josh decision from live. Next engineering slice (optional, doesn't block install):
commit live manifest to `proof/`, tune scanner prompt for bounded reads, Mintlify
cookbook + reference (WS5 step 3). Operator order: **GEPA/GELO test → DungeonGrid → blog → optimizers prod → hosted Cloud → synth-ai SDK → other**.

### Live scan command (canonical — use absolute target)

```bash
export SYNTH_API_KEY=sk_dev_00000000000000000000000000000001
# one-time: materialize /tmp/jesterky_codex_home/config.toml (see round6 handoff)

cargo install jesterky-cli   # or: cargo install --path crates/jesterky-cli

jesterky run examples/quality_scan.json \
  --actor codex --model deepseek/deepseek-v4-pro-direct \
  --codex-home /tmp/jesterky_codex_home \
  --cd /Users/joshpurtell/Documents/GitHub/jesterky \
  --args '{"target":"/Users/joshpurtell/Documents/GitHub/jesterky"}' \
  --out proof/quality_scan.live.manifest.json

jesterky replay proof/quality_scan.live.manifest.json --spec examples/quality_scan.json
# expect: status=completed, 52 events, 9 recorded, replay ok
```
