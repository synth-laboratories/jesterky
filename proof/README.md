# jesterky proof packet

Reproducible evidence backing the launch-blog claims. Every claim in the blog
maps to one command here. Run from repo root.

## 1 — Installs from a clean checkout (M1)

```bash
cargo install --path crates/jesterky-cli --root /tmp/jt && /tmp/jt/bin/jesterky --help
```

Produces a `jesterky` binary (~3 MB) with `run`, `replay`, `validate`,
`visualize`, `schema` subcommands.

## 2 — Deterministic fake E2E: run → replay (core + record/replay)

```bash
jesterky run examples/quality_min.json --actor fake --run-id demo-fake-001 --out proof/quality_min.manifest.json
# -> status=completed events=5 recorded=1

jesterky replay proof/quality_min.manifest.json --spec examples/quality_min.json
# -> replay ok: events=5 recorded=1
```

No network. The manifest in this directory (`quality_min.manifest.json`) is the
committed artifact; replay re-drives the orchestration against the recorded
actor outputs and matches on the fidelity fields (Addr / kind / payload; wall_ms
is metadata, excluded — ADR #5).

## 3 — Contract is the source of truth (M0)

```bash
cargo run -q -p jesterky-contract --example emit_schema workflow > jesterky.schema.json
cargo test -p jesterky-contract          # schema_drift guard: artifact matches emitter
./python/gen.sh                          # regenerates Python types from the same schema
```

## 4 — Publish-ready

```bash
cargo publish --dry-run -p jesterky-contract      # green
cd python && uv build                             # sdist + wheel
```

## 5 — mloky parity gate (M2 proof spine)

```bash
cargo test -p jesterky-quality --test conformance
# -> mloky_reference_run_is_faithful ... ok
# -> jesterky_scan_matches_mloky_contract ... ok
```

jesterky descends from the mloky reference runtime. They do NOT share an event
vocabulary — mloky emits domain lifecycle events, jesterky emits the pinned
`Addr`-keyed contract stream — so byte equality is the wrong assertion. The gate
instead projects both runtimes onto a canonical `RunOutcome` and asserts the two
properties any substrate must guarantee on a map→reduce run: **conservation**
(`jobs_started == jobs_completed == jobs_in_report`, no silent drops) and
**termination** (terminal `completed`, all jobs ok). The mloky projection is read
from a real recorded run (`crates/jesterky-quality/fixtures/mloky_scan_reference.jsonl`,
8 jobs); the jesterky projection from a fresh deterministic scan. Passing means
jesterky reproduces the reference contract.

## 6 — Live model E2E (M2, requires codex + proxy)

See `HANDOFF_jesterky_round6_live_scan.md`. Runs the real quality scan through
`codex exec` (DeepSeek proxy), then replays the live manifest.

## 7 — Blog quality scan (hero workflow)

Release tier: **pending live proof**. The workflow spec is present and validates,
but the live manifest is not committed yet.

```bash
jesterky validate examples/quality_scan_blogs.json

jesterky run examples/quality_scan_blogs.json \
  --actor codex \
  --codex-home /tmp/jesterky_codex_home \
  --cd /path/to/frontend \
  --run-id quality-scan-blogs-live-001 \
  --out proof/quality_scan_blogs.live.manifest.json

jesterky replay proof/quality_scan_blogs.live.manifest.json \
  --spec examples/quality_scan_blogs.json
```

Expected proof artifact before launch:
`proof/quality_scan_blogs.live.manifest.json`.

## 8 — GEPA trace annotate

Release tier: **measured, local replay**. The committed manifest covers a Craftax
trace corpus and records the GEPA-oriented theme registry.

```bash
jesterky validate examples/gepa_trace_annotate.json

jesterky replay proof/craftax_trace_annotate/gepa_trace_annotate.manifest.json \
  --spec examples/gepa_trace_annotate.json
```

Committed proof artifact:
`proof/craftax_trace_annotate/gepa_trace_annotate.manifest.json`.

## 9 — GELO trace annotate

Release tier: **measured, local replay**. The committed manifest covers the same
Craftax trace corpus, with GELO-oriented theme detection and saturation output.

```bash
jesterky validate examples/gelo_trace_annotate.json

jesterky replay proof/craftax_trace_annotate/gelo_trace_annotate.manifest.json \
  --spec examples/gelo_trace_annotate.json
```

Committed proof artifact:
`proof/craftax_trace_annotate/gelo_trace_annotate.manifest.json`.

## 10 — DungeonGrid 4p policy (LLM-only, mloky parity path)

Release tier: **measured, LLM policy**. Round-robin 4-hero turn schedule on an
in-process grid. **Every hero turn is an LLM call** (`--actor codex` required;
`--actor fake` is rejected). Honest framing: runtime + replay + viz, not quest
solved.

```bash
jesterky validate examples/dungeongrid_4p.json

# DeepSeek flash via SMR responses proxy (base_url must be …/api/v1, not …/v1)
# CODEX_HOME example: /tmp/jesterky_deepseek_flash_home with SYNTH_API_KEY set
jesterky run examples/dungeongrid_4p.json \
  --actor codex \
  --model deepseek/deepseek-v4-flash-direct \
  --codex-home /tmp/jesterky_deepseek_flash_home \
  --args '{"quest_id":"lantern_crypt","seed":7,"max_turns":8,"hero_ids":["hero_1","hero_2","hero_3","hero_4"]}' \
  --run-id dungeongrid-4p-llm-flash-cap8 \
  --out proof/dungeongrid_4p.manifest.json \
  --follow

jesterky visualize proof/dungeongrid_4p.manifest.json --spec examples/dungeongrid_4p.json
jesterky replay proof/dungeongrid_4p.manifest.json --spec examples/dungeongrid_4p.json
```

Committed proof artifact: `proof/dungeongrid_4p.manifest.json` (from an LLM run).

The dungeongrid runplan declares formal **resource budgets** (`actor_calls`,
`tokens`, `wall_seconds`). The live panel shows progress + ETA; the manifest
carries a `budgets` snapshot (`budget_engine.v1`).

## 11 — OBLIQ-Bench math verification (DeepSeek flash)

Release tier: **measured**. Gold-infused listwise rerank on the Math Meta-Program
task of [OBLIQ-Bench](https://arxiv.org/abs/2605.06235) (Tchuindjo, Shah, Khattab).
See `docs/OBLIQ.md` for data download + knobs.

```bash
# data already under data/obliq-bench/math (HF dianetc/OBLIQ-Bench)
export SYNTH_API_KEY=…
jesterky run examples/obliq_math_verify.json \
  --actor codex \
  --model deepseek/deepseek-v4-flash-direct \
  --codex-home /tmp/jesterky_deepseek_flash_home \
  --args '{"data_dir":"data/obliq-bench/math","max_queries":12,"pool_size":16,"k":10,"seed":7,"mode":"verify"}' \
  --run-id obliq-math-flash-q12 \
  --out proof/obliq_math_verify.manifest.json \
  --follow
```

Proof: `proof/obliq_math_verify.manifest.json` (DeepSeek v4 flash, 12 queries,
pool 16 with hard distractors). Representative scores: **mean nDCG@10 ≈ 0.99**,
**mean Recall@10 ≈ 0.92** (pool-conditional golds) — verification is easy once
candidates are shown; full-corpus first-stage retrieval is the hard problem.

## 12 — SMR ReportBench trace evaluate

Release tier: **spec wired, proof pending**. The workflow maps a ReportBench trace
evaluator over a directory of v4 trace JSON files and reduces the outcomes to a
single verdict. The launch proof still needs a real ReportBench `trace_dir` and
committed manifest; fake actor runs only prove the substrate shape, not the
benchmark claim.

```bash
jesterky validate examples/smr_reportbench_trace_evaluate.json

jesterky run examples/smr_reportbench_trace_evaluate.json \
  --actor codex \
  --model deepseek/deepseek-v4-flash-direct \
  --codex-home /tmp/jesterky_deepseek_flash_home \
  --args '{"trace_dir":"proof/reportbench_traces"}' \
  --run-id smr-reportbench-trace-evaluate-001 \
  --out proof/smr_reportbench_trace_evaluate.manifest.json

jesterky replay proof/smr_reportbench_trace_evaluate.manifest.json \
  --spec examples/smr_reportbench_trace_evaluate.json
```

Expected proof artifact before launch:
`proof/smr_reportbench_trace_evaluate.manifest.json`.
