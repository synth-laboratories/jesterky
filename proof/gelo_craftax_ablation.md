# GELO-Craftax ablation

## Live claim (workflows A/B)

The GELO launch claim is **GELO with jesterky workflows vs without**, not
base prompt vs a GELO-proposed prompt.

| Arm | Config | Intervention |
|---|---|---|
| A | `go_ex.jesterky_workflow.enabled = false` | GELO as today; no `jesterky_*` in core proposer workspace |
| B | `enabled = true` + workflow params/models | After train/search evidence each round: export traces → `jesterky run gelo_trace_annotate` → materialize theme/annotation context into core proposer workspace before propose |

Proof packet / runner:

- configs: `proof/gelo_jesterky_workflow_arm_{a,b}.json` (uplift-style budget; differ only on `enabled`)
- runner: `scripts/gelo_jesterky_workflow_ablation.py`
- status table: `proof/gelo_jesterky_workflow_ablation.md`
- summary JSON: `proof/gelo_jesterky_workflow_ablation/ablation_summary.json`

Ship bar: both arms produce many non-seed candidates; Arm B shows jesterky
receipts with **non-empty** theme/annotate signal and beats Arm A on uplift
over baseline (best non-seed search mean − baseline search mean). Hollow
receipts (`theme_count=0`) are a hard error, not a PASS.
Do **not** cite the seed-only `craftax_gamebench_rust_gelo_20260626T1915Z` family.
Do **not** cite `craftax_gelo_jesterky_workflow_arm_b_20260709T003418Z` as PASS
(invalid hollow materialization). Current PASS Arm B is
`craftax_gelo_jesterky_workflow_arm_b_20260709T010853Z` (see
`proof/gelo_jesterky_workflow_ablation.md`).

```bash
python3 scripts/gelo_jesterky_workflow_ablation.py --validate-configs
# after both Craftax GELO runs finish:
python3 scripts/gelo_jesterky_workflow_ablation.py --score-only \
  --arm-a-result <arm_a>/artifacts/result_manifest.json \
  --arm-b-result <arm_b>/artifacts/result_manifest.json
```

## Documented drop (old prompt A/B)

The earlier handoff asked for Arm B = a real GELO-proposed Craftax prompt.
Searched the handoff-named run family:

`.../goex_runs/craftax_gamebench_rust_gelo_20260626T1915Z/`

All nine `artifacts/candidate_registry.json` files contain the same accepted
candidate, `goex_prompt_b02596a361f8`, with `source: "seed"`. Core proposer
manifests say the contract was initialized and no proposer turn executed.

That prompt A/B path is a **documented drop**. It is not the live GELO claim.
The post-hoc prompt scorer remains available for audit only:

```bash
python3 scripts/gepa_gelo_ablation.py --optimizer gelo --dry-run
```
