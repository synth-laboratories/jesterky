# HANDOFF — jesterky integrations, end to end

Everything remaining to take jesterky from "finished substrate" to "integrated
product": optimizers (GEPA/GELO/MAPO), Stack (M3), Hosted (M4), SMR ReportBench,
plus the OSS ship hygiene that gates the public story.

**Read this first — the invariant that shapes all of it:** the dependency is
**one-way**. `optimizers`, Stack, Hosted, and SMR **consume** jesterky; jesterky
**never** depends on them. No optimizer/Stack/SMR type, crate, or import ever
enters a jesterky crate. Every integration below is authored in the *consumer*
repo against jesterky's manifest contract. If you find yourself editing a
`jesterky-*` crate to satisfy a consumer, stop — the need belongs on the other
side, or it's a genuine contract gap to raise explicitly.

**State of jesterky itself:** core is DONE (contract, runtime, replay, budgets,
goals v1+v2, process-tree I/O, output-schema validation, invariant report,
stop-reason, events-out). 96 tests green. Nothing in §1–§4 requires new jesterky
core logic — only §0 (the surface you build against) and §5 (ship hygiene).

---

## 0. The contract surface you build against

This is the entire API. Everything downstream reads a `RunManifest` (or drives
the CLI). Schemas: `jesterky.schema.json` (workflow), `jesterky.manifest.schema.json`
(manifest). Python bindings: `python/jesterky/{spec,manifest}.py` (regen via
`python/gen.sh`). Rust: link `jesterky-contract` and deserialize.

### CLI (the process boundary)

```
jesterky run <spec.json> [--args '<json>' | --args-file <f>] [--out <manifest>]
             [--events-out <ndjson|->] [--actor fake|codex] [--model <id>]
             [--codex-home <dir>] [--cd <dir>] [--run-id <id>]
             [--follow|--no-follow] [--width N]
jesterky replay <manifest> [--spec <spec>]
jesterky validate <spec>
jesterky visualize <manifest> [--spec <spec>] [--width N]
jesterky schema workflow|manifest
```

- `run` writes the manifest to `--out` and a `<manifest>.spec.json` sidecar.
- `--events-out` streams the canonical event log as NDJSON (one `Event` per line,
  set-equal to `manifest.events`) — the honest precursor to hosted SSE.
- `--actor fake` is deterministic/no-network (echoes inputs); `codex` drives the
  real model via `codex exec` (ChatGPT-bundle auth, never an API key).

### `RunManifest` (what consumers read)

| Field | Meaning for a consumer |
|---|---|
| `run_id`, `workflow_name`, `spec_hash` | identity; `spec_hash` pins the exact topology (ADR #5) |
| `args` | the run's seed inputs |
| `events[]` | canonical event log; order/identity = `Addr` (sort by it, never emission order) |
| `recorded[]` | one impure output per actor/resource call (for replay) |
| `trace` | **the optimizer artifact** — see below |
| `status` | `completed` \| `failed` |
| `stop_reason` | `completed` \| `node_failed` \| `goal_unmet` \| `budget_exhausted` — **read this, don't parse the failure string** |
| `budgets?` | progress + ETA projection (spent/reserved/committed) when caps declared |
| `goals?` | goal projection: `state`, `required_met`, per-goal progress, `terminated_early` |
| `invariants?` | manifest self-check (`all_ok` + named checks); a failing check = runner bug, not workload failure |

### `ProcessNode` trace — the optimizer-facing process tree (ADR #2)

Now fully populated (this is what the process-tree I/O work delivered):

```
ProcessNode { addr, label, inputs, outputs, score?, signal?, artifacts[], children[] }
```

- **root** (`workflow:<name>`): `inputs` = args, `outputs` = the settled ledger.
- **interior** (`map:…`, reduce, `[i]`): `inputs`/`outputs` from the node's I/O;
  map/for_each nodes carry `signal = {successes, total, attempted, cancelled}`.
- **leaf** (`actor:…`, `observe:…`, `step:…`): `inputs` = the resolved bound
  inputs, `outputs` = the recorded output, `score`/`signal` = optimizer slots (a
  reduce may surface a score; **code never invents a quality score** — house rule).

**The optimizer contract is the (inputs → outputs → score) triple per leaf, plus
scored reduces.** That is the whole reason the trace exists. Grade OUTCOMES only.

---

## 1. Optimizers — GEPA / GELO / MAPO  (repo: `optimizers/`)

**Goal:** an optimizer runs jesterky as its rollout substrate, reads the trace,
proposes an improvement, and a Craftax A/B shows the loop moves a metric.

**Where the logic lives:** entirely in `optimizers/` — the proposer workspace,
the trace adapter, the search loop. jesterky is a dependency (link
`jesterky-contract` for typed manifest reads, or shell out to the CLI).

**Build:**
1. **Trace adapter** — a function `RunManifest → proposer workspace files`
   (`state/jesterky_*`). Walk `trace`, emit per-leaf `(inputs, outputs, score,
   signal, addr, label)` rows the GEPA proposer already expects. Reference the
   existing workspace contract in `optimizers/skills/gepa/SKILL.md` and
   `optimizers/rust/crates/synth_optimizer_platform/` (LimitEngine, evidence,
   workspace) — match its shape, don't invent a new one.
2. **GEPA hook** — the proposer reads those files, mutates the prompt/spec, and
   re-runs jesterky. Acceptance: GEPA specs run green on a **real generation dir**
   and the proposer consumes `state/jesterky_*` without hand-massaging.
3. **GELO** — same adapter; surface `evidence_refs` from the trace (the `artifacts`
   refs + leaf addrs) into GELO themes.
4. **Craftax A/B (M5a proof)** — run the loop on the Craftax trace-annotate spec
   (`examples/gepa_trace_annotate.json` is the jesterky-side spec); show a scored
   before/after. This is the proof the integration is real, not just wired.
5. **MAPO row** — later; same adapter, add a MAPO entry.

**Wall-safe rule (non-negotiable):** the optimizer must not block the policy /
must be resumable; see the wall-safe note in `jesterky_notes.md` §14. Chinese
wall: the optimizer proposes; a verifier (not code, not the proposer) grades.

**Acceptance:** a scripted GEPA (and GELO) run that reads a jesterky manifest,
proposes, re-runs, and reports an A/B delta on Craftax. No jesterky crate changed.

**Dependencies:** none external — this is unblocked *today* (the trace is real).
Highest leverage of the four; do it first.

---

## 2. Stack (M3)  (repo: the Stack / stackd repo)

**Goal:** Stack drives jesterky as a first-class workflow surface — the five verbs.

**Where the logic lives:** the Stack repo (Rust `stackd` + its command layer).
jesterky is invoked as a subprocess (CLI) or linked as a crate.

**Build — the five verbs over jesterky:**
- **register** — take a jesterky spec, `validate_and_hash`, store it under a Stack
  id (reuse `jesterky validate` / `workflow_schema_json`).
- **launch** — `jesterky run` with args; capture the manifest + `--events-out`.
- **inspect** — render `status`/`stop_reason`/`goals`/`budgets`/`invariants` and
  the trace (reuse `jesterky visualize`).
- **replay** — `jesterky replay <manifest>` (deterministic re-drive; ADR #7).
- **compare** — diff two manifests' traces/scores (a Stack-side diff over the
  contract; jesterky provides the canonical `Addr` ordering to align on).

**Acceptance:** a staging quality-scan run registered→launched→inspected→replayed
→compared, producing a Stack-side proof. Then **owner A+ grade → prod** (held in
staging until A+, per the prod-scarcity rule — one finish-line push at a time).

**Prerequisites / unknowns to verify in the Stack repo:** how Stack stores
workflow artifacts, whether it wants the CLI or a linked crate, and the
"Effort"-noun boundary (do NOT reuse "Effort" for a hosted unit — that noun is
Stack-only; the hosted unit noun is Josh's to name). See `jesterky_notes.md` §15.

**Dependencies:** Josh-gated on the A+ grade for prod. Zero code today.

---

## 3. Hosted Cloud (M4)  (repos: backend + `synth-ai`)

**Goal:** run jesterky behind an API with conformance-identical streams; a
`synth-ai` client submits a spec and gets a manifest back.

**Where the logic lives:** backend (the runner service) + `synth-ai` (the client
namespace). jesterky is the runtime the service shells out to / embeds.

**Build:**
1. **Runner service + OpenAPI** — an endpoint that accepts a spec + args, runs
   jesterky, streams events (SSE — the `--events-out` NDJSON sink is the shape to
   lift), and returns the manifest. Streams must be **conformance-identical** to a
   local run (same `Addr` order, same events) so a hosted manifest replays locally.
2. **Auth / identity / billing** — standard backend concerns; bill via Autumn (not
   Stripe), per house rule.
3. **`synth-ai` workflows namespace** — spec-in / manifest-out client. Keep it a
   thin client over the API; no jesterky logic re-implemented in Python.
4. **Local replay of a hosted manifest** — the round-trip proof: pull a hosted
   manifest, `jesterky replay` it locally, assert identical.

**Acceptance:** submit via `synth-ai` → hosted run → pull manifest → local replay
matches. SSE stream matches the local `--events-out` stream.

**Prerequisites / unknowns:** backend service-type wiring (SYNTH_SERVICE_TYPE
touches 3 places), the SSE placement decision (§16), and the hosted-unit noun
(Josh). Larger than everything done so far combined.

**Dependencies:** Josh-gated (GitHub-public, prod promotion). Zero code today.

---

## 4. SMR — ReportBench trace-evaluate  (repo: SMR / backend)

**Goal:** the 4th locked blog example — a trace-evaluate map-reduce — exists, or
is explicitly waived.

**Two touchpoints, don't conflate them:**
- **SMR proxy infra** — the DeepSeek Responses↔chat bridge — is **already used and
  working** for the `--actor codex` proxy path. Nothing to build.
- **ReportBench example** — `examples/smr_reportbench_trace_evaluate.json` — is
  **missing**. This is the gap.

**Build (or waive):**
- **Author** `smr_reportbench_trace_evaluate.json`: a jesterky spec that maps a
  trace-evaluate actor over report traces and reduces to a verdict — same shape as
  `examples/gepa_trace_annotate.json`, pointed at ReportBench inputs. Add the
  matching schema + a proof-packet section 12. Optional: link an Effort ledger.
- **Or waive**: document in the blog why example #4 is deferred, and drop it from
  the "four locked examples." Cheapest honest path if ReportBench inputs aren't
  ready.

**Acceptance:** the spec runs (fake + real), produces a trace-evaluate manifest,
and the proof packet cites it — OR a written waiver in the blog + notes.

**Dependencies:** smallest of the four; ~a day. Independent of §1–§3.

---

## 5. Ship hygiene (gates the OSS story)

Not integration, but gates the public launch and prevents regressions:
- **Commit** the tree (large, green, tested — currently uncommitted; ~90 dirty
  files across multiple sessions — reconcile before committing).
- **CI** — `.github`: `cargo test --workspace` (NOT just `cargo check` — that
  skips test targets and let a build break ship once) + the `schema_drift` guard.
- **GitHub-public org** (Josh) — unblocks crate/Mintlify clone URLs.
- **Publish 0.1.1/0.2** if demos need the WIP crates; **mloky freeze** so the
  oracle can't drift; trusted-publishing / release assets (optional).

---

## Sequencing & acceptance matrix

| # | Surface | Repo | Gated on | Size | Proof of done |
|---|---|---|---|---|---|
| 5a | Commit + CI | jesterky | — | hours | CI green on PR |
| 1 | Optimizers hook | optimizers/ | nothing (unblocked) | days | GEPA+GELO read manifest, Craftax A/B delta |
| 4 | SMR ReportBench | SMR/backend | ReportBench inputs | ~1 day | spec runs / or waiver |
| 2 | Stack M3 | Stack | Josh A+ grade | large | 5 verbs, staging proof |
| 3 | Hosted M4 | backend + synth-ai | Josh (public, prod) | largest | synth-ai→hosted→local replay |

**Recommended order:** 5a (commit+CI) → **1 (optimizers)** first among integrations
— unblocked, highest leverage, the proof that the substrate is real → 4 (SMR, cheap)
→ 2 (Stack) → 3 (Hosted). Stack and Hosted are Josh-gated and each larger than the
whole substrate build; scope them as their own multi-PR efforts.

## Guardrails (every surface)
- **One-way dependency** — consumers link jesterky; jesterky links nothing back.
- **Read `stop_reason` / typed manifest fields** — never phrase-match a failure
  string or an LLM output (house rule: decisions from typed data).
- **Grade outcomes only** — code never judges actor work quality; a verifier does.
- **No API keys** — codex/ChatGPT-bundle auth; no OpenAI key, no OpenRouter,
  no litellm (banned), no Anthropic models in product routes.
- **Conformance-identical streams** — a hosted/Stack run must replay locally to
  the same `Addr`-ordered events, or the abstraction leaks.
- **jesterky-quality = OSS demo only** — private-data evals go in `Jstack/quality`,
  never a published jesterky crate (`cargo publish` ships all source).
```
