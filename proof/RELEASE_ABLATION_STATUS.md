# Release ablation status (Jul 8)

Tracks each release-blog use case against the **ablation bar**: a compelling case
is a two-arm experiment — named **target quantity**, **Arm A (without)**, **Arm B
(with)**, **uplift = B−A with n** — not a single run. AT-1 = plumbing (both arms
run e2e → manifests). AT-2 = a target-metric scorer reads both arms. AT-3 = uplift
committed. AT-4 = arms differ only in the intervention. Never fabricate a number;
a small honest result ships, a fake one does not.

## Substrate plumbing — AT-1 (PROVEN tonight, fake actor)

All three optimizer/SMR specs run end-to-end and produce a real manifest with a
populated process tree + passing invariants. The only prerequisite was seeding
`trace_dir` (the specs referenced `ledger.trace_dir` with no default):

```bash
TDIR=proof/craftax_v4_traces   # any v4 trace dir; ReportBench needs a real report-trace dir
jesterky run examples/gepa_trace_annotate.json --actor fake --args "{\"trace_dir\":\"$TDIR\"}" --out /tmp/gepa.json
jesterky run examples/gelo_trace_annotate.json --actor fake --args "{\"trace_dir\":\"$TDIR\"}" --out /tmp/gelo.json
jesterky run examples/smr_reportbench_trace_evaluate.json --actor fake --args "{\"trace_dir\":\"$TDIR\"}" --out /tmp/smr.json
```

| # | Use case | Spec | AT-1 plumbing | manifest |
|---|---|---|---|---|
| 2 | GEPA trace-annotate | `gepa_trace_annotate.json` | ✅ completed | 51 events, 8 recorded, trace ✓, invariants ✓ |
| 3 | GELO trace-annotate | `gelo_trace_annotate.json` | ✅ completed | 51 events, 8 recorded, trace ✓, invariants ✓ |
| 4 | SMR ReportBench | `smr_reportbench_trace_evaluate.json` | ✅ completed | 52 events, 9 recorded, trace ✓, invariants ✓ |
| 1 | Blog quality scan (demo) | `quality_scan_blogs.json` | fake ✓; **live `--actor codex` owed** | — |

Fake runs prove the **substrate shape**, not the benchmark claim. The uplift
numbers below require real model arms (or existing champion-cycle data).

## Ablation uplift — AT-2/AT-3 (in progress)

Target quantities and arms per use case:

| # | Target quantity | Arm A (without) | Arm B (with) | Uplift table | State |
|---|---|---|---|---|---|
| 2 | Craftax mean reward | base ReAct prompt | GEPA-proposed prompt | `proof/gepa_craftax_ablation.md` | done — +0.678 mean reward, 95% CI [0.205, 1.151], exact two-sided p=0.007010, n=64 |
| 3 | Craftax uplift over baseline (GELO) | GELO, `jesterky_workflow.enabled=false` | GELO, annotate→materialize before core propose | `proof/gelo_jesterky_workflow_ablation.md` | PASS — A +0.339 vs B +0.977 (best non-seed search − baseline); B receipts non-empty (r0–r3 themes 8/32/47/65). Hollow `…003418Z` INVALID audit-only. Old prompt A/B drop in `proof/gelo_craftax_ablation.md` |
| 3b | Craftax best_heldout_mean (GEPA ± workflows) | GEPA, `jesterky_workflow.enabled=false` | GEPA, annotate→materialize before propose | `proof/gepa_jesterky_workflow_ablation.md` | FAIL — bigger budget A `…033652Z` heldout 1.5 > B `…040537Z` 1.0; B themes 20/23/22; A zero receipts. Prior small-budget tie audit-only. Hook ok; M5a primary not cleared. Separate from n=64 prompt A/B |
| 4 | ReportBench score | baseline report | trace-evaluate-guided | (A/B owed) | scorer WIRED + live-proven (07-09): real lane artifacts → `scripts/build_reportbench_traces.py` → 4 traces; gpt-5.5 evaluate live, agreement 1.0 vs autograde (fails 12/17 lane, passes 3× 18/18), report score 0.926, replay ok — `proof/smr_reportbench_trace_evaluate.md`. A/B DONE (07-09): verdict-guided revision, mean 0.9265 → 1.0 (hello_world +0.2941; 3 heading checks legit, 2 regex checks satisfied by quoted-pattern disclaimer = rubric gameability finding) — `proof/reportbench_ab.md`. In-run guidance A/B = next rung |

## MCP worker exposure — DONE (verified by running, Jul 8)

Stack exposes the 5 jesterky verbs as MCP tools workers can call:
`stack_jesterky_{register,launch,inspect,replay,compare}` (stack/src/mcp/server.ts,
impl stack/src/jesterky.ts, shells out to the `jesterky` CLI; one-way dep clean).
All four workflows (gepa/gelo/stack-quality/smr) verified invocable end-to-end via
the real launch arg vector (`--actor fake`). Worker flow: `register` a spec →
`launch` by workflow_id (smr needs `args:{trace_dir}`) → inspect/replay/compare.
Two breaks were caught+fixed by RUNNING (not inspection): a stale release binary
missing `--events-out`/`--args-file` (rebuilt), and a `tsc` type error in compare.
**Operational prereq for the worker env:** `jesterky` must be on `PATH` or
`STACK_JESTERKY_COMMAND` set to the built binary — ties to publishing 0.1.1 so
workers can install it.

## Gates before "great blog"
- [x] AT-3 headline: GEPA-Craftax uplift at **n=64, paired, CI-backed**: base 2.328 → GEPA 3.006, delta +0.678, 95% CI [0.205, 1.151], exact two-sided p=0.007010. Prior n=8 smoke (+0.85, p=0.137) is INVALID as a headline and kept only as audit trail.
- [x] AT-3: GELO ± jesterky workflows Craftax A/B — uplift over baseline PASS (A +0.339 → B +0.977; B receipts non-empty themes). See `proof/gelo_jesterky_workflow_ablation.md`. Hollow `…003418Z` INVALID audit-only. Old prompt A/B remains a documented drop in `proof/gelo_craftax_ablation.md`.
- [x] AT-3: GEPA ± jesterky workflows Craftax A/B — RESOLVED AS REFRAME (07-09): scored FAIL at bigger budget (A heldout 1.5 > B 1.0 on `…033652Z`/`…040537Z`; B themes non-empty). Hook works; M5a primary not cleared. Release ships it as **wired, no measured uplift**; the workflow-uplift case is GELO. See `proof/gepa_jesterky_workflow_ablation.md` § Disposition. Do not conflate with n=64 prompt A/B.
- [x] AT-2: SMR-ReportBench scorer wired to a LIVE trace-evaluate manifest over real lane artifacts (agreement 1.0, report score 0.926, replay ok — 07-09). AT-3 A/B uplift is post-launch.
- [x] Live quality-scan manifest (example 1, `--actor codex`): 2 SOUND / 6 FRAGILE over 8 published posts, replay ok — `proof/quality_scan_blogs.live.manifest.json` (07-09).
- [x] Green tree gate (no CI, Josh 07-08): `cargo test --workspace` + `cargo build --all-targets` pass locally — run before each landing.
- [ ] Proof packet section per example cites its committed table/manifest.
- [x] MCP `workflows` tools live for gepa/gelo/stack/smr (worker-invocable) — verified running.
- [ ] jesterky published/installed so workers have the binary on PATH.
- [ ] Prose (Josh) · Mintlify L3 · GitHub-public (Josh) · flip `draft:true`→live.
