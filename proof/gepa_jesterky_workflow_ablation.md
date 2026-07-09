# GEPA ± jesterky workflows ablation

**Status:** FAIL — bigger budget (6 gens / 8 train / 4 heldout); A heldout 1.5 > B 1.0

Budget vs prior smoke: gens 3→6, proposals/gen 2→3, train 4→8, heldout 2→4, total rollouts 40→120.
B themes still non-empty (g0–g2: 20/23/22). Prior small-budget tie (1.0=1.0) superseded by these runs.

Claim shape: GEPA with jesterky trace annotate workflows vs without.
Arms differ only in `jesterky_workflow.enabled`.

Primary metric: **best_heldout_mean** (M5a). Existing n=64 base-vs-GEPA-prompt
headline in `proof/gepa_craftax_ablation.md` is a separate claim.

## Results

| Arm | jesterky | non-seed | baseline heldout | best heldout | uplift | receipts |
|-----|----------|---------:|-----------------:|-------------:|-------:|---------:|
| A | off | 11 | 0.75 | 1.5 | 0.75 | 0 |
| B | on | 6 | 1.0 | 1.0 | 0.0 | 3 |

- A sources: `{'seed': 1, 'reflector:parent_variation': 11}`
- B sources: `{'seed': 1, 'reflector:parent_variation': 6}`
- A best: `gepa_0f34df687ccf`
- B best: `gepa_d3da93b581f8`

## Ship bar

- min non-seed candidates per arm: 1
- Arm B must have jesterky receipts with non-empty theme_count/annotated
- Arm A must have zero jesterky receipts
- Arm B best_heldout_mean must beat Arm A
- Hollow annotate materialization is FAIL (not PASS)

## Failures

- Arm B did not beat Arm A on best_holdout_mean (A=1.5; B=1.0)

## Artifacts

- Arm A result: `proof/gepa_jesterky_workflow_runs/craftax_gepa_jesterky_workflow_arm_a_big_20260709T033652Z/result_manifest.json`
- Arm B result: `proof/gepa_jesterky_workflow_runs/craftax_gepa_jesterky_workflow_arm_b_big_20260709T040537Z/result_manifest.json`

Configs:

- `proof/gepa_jesterky_workflow_arm_a.toml`
- `proof/gepa_jesterky_workflow_arm_b.toml`

Provenance: `proof/gepa_jesterky_workflow_runs/run_registry.jsonl` — append-only
ledger, one `started`+`finished` pair per run, each pointing at its
`result_manifest.json`, raw + normalized event feeds, and `cache_profile.json`.
Audit and re-scoring read the registry, not remembered paths.

## Disposition (release reframe, 2026-07-09)

The release ships this claim as **wired, no measured uplift on Craftax M5a**.
The hook is validated end-to-end: config round-trips, Arm B materializes
non-empty themes before every propose, Arm A stays clean at zero receipts,
hollow materialization fails closed. The intervention did not clear the primary
metric at either budget, so the blog's "± jesterky workflows" uplift case is
GELO (`proof/gelo_jesterky_workflow_ablation.md`, A +0.339 → B +0.977), and
GEPA's workflow arm is reported exactly as measured: substrate works, uplift
not demonstrated on this taskset.

Probes if revisited: a harder heldout slice (the taskset ceiling is soft at
~1–1.5, so the metric saturates), and whether annotate context biases proposals
toward train-fit — A found a better non-seed candidate on heldout; B improved
its seed on train (0.5→1.0) while heldout stayed flat.

