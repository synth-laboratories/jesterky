# Handoff — jesterky release + blog plan (for eng planning)

**Date:** 2026-07-08. **Audience:** engineer building the integrated release +
content plan. **Goal:** one doc that lists every authoritative source, current
state, open workstreams, gates, and how the public story ships.

**Supersedes:** the interrupted "jesterky release-blog" task noted in
`Jstack/.jstack/daily_notes/2026-07-08/HANDOFF_synth_blog_strategy_20260708.md`
(no plan was created there — this is it).

---

## Executive summary

| Layer | Status | Next ship event |
|---|---|---|
| **Code (runtime)** | M1 done; M2 mostly implemented, live proof pending | Prove live DeepSeek quality scan + replay |
| **OSS publish** | Not shipped | M0 `jesterky-contract` 0.1 → M1 `jesterky` 0.1 + CLI |
| **Terminal viz** | Post-hoc panel landed (`visualize`); live `--follow` open | PR2 in terminal viz handoff |
| **Public story** | Not drafted | `feature_release` blog + changelog + cookbook + proof |

**Execution log (this push):**
- **2026-07-08 · WS0 done** — landed the per-actor output-schema WIP + codex
  `--ephemeral`/`--output-schema`/stdin-null hardening + strict no-tool scanner
  prompts (`95dad43`); tree clean, `cargo test --workspace` green (2 live-codex
  ignored). Cadence: codex `gpt-5.5` xhigh=draft / medium=grind, one reviewed
  cycle = one commit, revert-on-break. Order: WS0→(WS1∥WS2∥WS4)→WS3→WS5→WS6.
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

**⚠ Josh-gated (irreversible / outward-facing — do NOT auto-execute):**
`cargo publish` to crates.io · PyPI upload of the `jesterky` package · making the
`github.com/jesterky` org+repo public · flipping the launch blog live. Everything
above is driven to publish-READY; these four are the finish-line triggers.

**PUBLISH STATUS (2026-07-08, Josh authorized "publish crates now"):**
- ✅ crates.io: `jesterky-contract`, `-core`, `-actor`, `-model`, `-quality` @0.1.0 LIVE.
- ⏳ `jesterky-cli` @0.1.0 — hit crates.io new-crate rate limit (429); auto-retry
  scheduled for 13:47:52 GMT (background). Verify: `curl crates.io/api/v1/crates/jesterky-cli`.
- ⛔ PyPI `jesterky` — wheel built (`python/dist/`), upload BLOCKED: Josh authorized
  crates.io only, not PyPI. Awaiting explicit PyPI okay → `cd python && uv publish`.
- ⛔ `github.com/jesterky` org+repo public — not done (manual).
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
| **M5–M6** | Optimizer products (not `jesterky-*` crates) | Process-tree consumption; verifier loop |

Roadmap quote: *"Nothing **real** ships until M2, but M0 contract and M1 OSS
crate with fake actors are genuine early releases."*

---

## Current code state (2026-07-08)

Latest commits on `jesterky` (check `git log` — this handoff written at
`c967fb7`):

| Area | Status | Notes |
|---|---|---|
| **Contract types + schema** | Done | `jesterky.schema.json`, `jesterky.manifest.schema.json`, round-trip + drift tests |
| **Core runner** | Done | All node kinds, Addr clock, serial/parallel map, sessions, limits, mailbox (unused) |
| **Replay** | Done | `ReplayActor`, fidelity check (wall_ms stripped) |
| **CLI** | Done | `run`, `replay`, `validate`, `schema`, `visualize` |
| **Quality workload** | Done | `jesterky-quality`: expand → map 8 → aggregate; `examples/quality_scan.json` |
| **Real actor** | Done | `ModelActor` + `CodexModel`; `--actor codex`, `--model`, `--codex-home`, `--cd` |
| **Args seeding** | Done | `--args` → ledger (e.g. `target` for scan) |
| **Terminal viz** | **Partial** | `jesterky visualize` + btop-style `RunView` panel post-hoc (`adapt_manifest`). Live `--follow` not wired. |
| **M0 publish** | **Not done** | crates.io, PyPI types, Python codegen package |
| **M1 parity gate** | **Not done** | mloky `seq` → jesterky `Addr` mapping + fixture |
| **M2 live proof** | **Not done** | Real proxy scan + replay (`round6` handoff) |
| **Packaging** | **Not done** | GH release, `cargo install`, versioning policy page |

**Uncommitted at handoff time (verify before planning dates):**
`quality_summary.schema.json`, `quality_verdict.schema.json`, terminal viz
handoff file, possible WIP on cli/model/quality.

---

## Content / blog routing (jesterky)

From blog strategy — **internal type for this ship:**

| Artifact | Internal type | External home | Owner in plan |
|---|---|---|---|
| Main post | `feature_release` | Blog / Product | Eng + content |
| Version delta | `release_note` | Changelog | Eng |
| Runnable path | `cookbook` | Docs / Examples | Eng |
| Evidence | `proof_page` | Resources / Proof | Eng |
| Optional deep dive | `engineering_essay` | Blog / Engineering | Optional second post |

**Routing rule:** `feature_release` + `release_note` + docs (if API/workflow
changes) + proof.

**Post shape (blog checklist spine):**

1. Problem and why it matters now
2. Mechanism we built (deterministic workflow substrate, quality scan)
3. Evidence and where it fails (≥1 limitation)
4. How to try it (CLI commands, truthful readiness)
5. **Applications at the end** — Stack workflows, hosted optimizers, GEPA over
   traces, managed research, agent infrastructure

**Announce-after-ship (release standards — L3 blocker):** blog links a nightly /
GH release that already contains the feature; cookbook runs against published
package version.

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

### WS1 — M0 contract publish (can parallel WS3)

- [ ] `jesterky-contract` 0.1.0 → crates.io
- [ ] Python types package → PyPI (codegen from `jesterky.schema.json`)
- [ ] `github.com/jesterky` org + repo public
- [ ] Conformance suite seeded from mloky `.run_logs/*_events.jsonl`
- [ ] mloky runtime freeze (bugfixes only) — coordinate with mloky owner
- [ ] Written versioning policy page (release standards: axis B)

**Blocks:** nothing for WS2 blog content, but blog should cite published versions.

### WS2 — M1 runtime publish

- [ ] `jesterky` 0.1 + CLI binary GH release / `cargo install`
- [ ] **mloky parity gate** — judgment on `seq`→`Addr`, fixture, green test
- [ ] Trusted publishing + attestations (release standards registry section)
- [ ] Changelog entry with upgrade command

**Blocks:** WS4 blog "how to try it" at measured tier.

### WS3 — M2 live proof (blog evidence packet)

- [ ] Stand up proxy `config.toml` (`round6` handoff)
- [ ] Live run: `jesterky run examples/quality_scan.json --actor codex …`
- [ ] Replay passes on live manifest
- [ ] Capture proof artifacts: manifest JSON, event count, replay command output,
      optional `jesterky visualize` screenshot
- [ ] Orphaned `codex` reap verified (M2 DoD)
- [ ] Relax/tune `min_success`, JSON extraction if DeepSeek flakes

**Blocks:** WS4 blog at `claim_tier: measured`.

### WS4 — Terminal viz (nice for blog/demo, not strict blocker)

See `HANDOFF_jesterky_terminal_viz.md`.

| Slice | Deliverable | Blog impact |
|---|---|---|
| PR1 | Done-ish: `visualize` + btop panel post-hoc | Screenshot for proof page |
| PR2 | `run --follow` live redraw | Demo GIF / live blog moment |
| PR3 | Per-shard progress during real codex | Parity with mloky live panel |

### WS5 — Content bundle (after WS2+WS3 minimum)

- [ ] **Blog** — `feature_release` draft; feature first, applications last
- [ ] **Changelog** — `release_note` with PR links, upgrade command, reserved
      `## Breaking` (usually absent)
- [ ] **Cookbook** — `quality_scan.json` E2E: fake + live paths; pinned package
      version; troubleshooting (proxy, `SYNTH_API_KEY`, `min_success`)
- [ ] **Proof page** — manifest hash, replay fidelity command, conformance ref
- [ ] Pass blog checklist + release standards + cookbook standards
- [ ] Rendered review mobile + desktop on prod URL

**Open (from blog strategy):** which repo owns website content — likely
`frontend`; confirm before editing.

### WS6 — Post-release (not blog blockers)

- M3 Stack Workflows integration
- M4 hosted runner
- M5/M6 optimizer consumption of `ProcessNode` traces

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

---

## Suggested plan timeline (eng to refine)

| Phase | Workstreams | Output |
|---|---|---|
| **P0 — Prove** | WS3 | Live manifest + replay log; proof folder |
| **P1 — Package** | WS1 + WS2 | crates.io/PyPI/GH release; parity test green or explicitly waived with ADR |
| **P2 — Content** | WS5 | Blog + changelog + cookbook + proof page draft |
| **P3 — Ship story** | WS5 gates | Publish content after P1 artifacts exist (L3) |
| **P4 — Polish** | WS4 PR2/3 | Live viz for follow-up post or demo refresh |

Parallelize P0 and WS1 where different owners.

---

## Open questions (add to plan as decisions)

| # | Question | Default / note |
|---|---|---|
| 1 | Is jesterky `launch_post` or `feature_release`? | **`feature_release`** per operator; apps at end |
| 2 | Bundle M0+M1 publish with blog or ship contract quietly first? | Roadmap: M0 can ship before M2 blog |
| 3 | mloky parity — hard blocker for blog or waivable with dev_evidence tier? | Release standards: prefer measured; document waiver if skipped |
| 4 | Canonical `product_area` nav label? | Stack / Open Research / Agent Infrastructure — confirm |
| 5 | Content repo owner? | Likely `frontend` |
| 6 | Changelog canonical source — website vs docs? | Blog strategy open question |
| 7 | Benchmark/proof public at ship or staged? | Blog strategy open question |

Log decisions with `jsk decision` when impact ≥ med.

---

## What NOT to put in the plan (explicitly out of scope)

- mloky flat-`seq` parity **mapping design** as part of blog work (it's M1 DoD,
  separate engineering judgment)
- M3 Stack UI / M4 hosted runner (mention as applications only)
- pyo3 Python runtime
- mermaid/box-graph viz (parked in mloky)
- Prod deploy of backend/frontend unless blog promotion requires it — then run
  `launch_checklist.md`

---

## Quick links for plan doc paste

```
Technical
  mloky/ROADMAP_jesterky.md
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
```

---

## Debrief note

The jesterky **release-blog** task was started 2026-07-08, interrupted by the
blog strategy handoff request, and had produced no plan. This document is the
planning input eng should merge into whatever tracker/goal spec they use next.
