# GELO ± jesterky workflows ablation

**Status:** PASS

Claim shape: GELO with jesterky trace annotate/flag/process workflows vs
GELO without them. Arms differ only in `go_ex.jesterky_workflow.enabled`.

Primary metric: **uplift over baseline** = best non-seed search mean −
baseline search mean (from `goex_acceptance_report.json`).

Arm B receipts (unique rounds): r0 8/4, r1 32/18, r2 47/29, r3 65/40
(theme_count/annotated). Prior hollow run
`craftax_gelo_jesterky_workflow_arm_b_20260709T003418Z` remains INVALID audit-only.

## Results

| Arm | jesterky | non-seed | baseline search | best non-seed search | uplift over baseline | receipts |
|-----|----------|---------:|----------------:|---------------------:|---------------------:|---------:|
| A | off | 12 | -0.058823529411764705 | 0.2800000000000001 | 0.3388235294117648 | 0 |
| B | on | 12 | -0.043478260869565216 | 0.9333333333333332 | 0.9768115942028984 | 8 |

- A best non-seed: `goex_prompt_cd51dfa5f72f`
- B best non-seed: `goex_prompt_5055ae1f6f18`
- A sources: `{'seed': 1, 'core_proposer:core_round_000': 3, 'core_proposer:core_round_001': 3, 'core_proposer:core_round_002': 3, 'core_proposer:core_round_003': 3}`
- B sources: `{'seed': 1, 'core_proposer:core_round_000': 3, 'core_proposer:core_round_001': 3, 'core_proposer:core_round_002': 3, 'core_proposer:core_round_003': 3}`

## Ship bar

- min non-seed candidates per arm: 3
- Arm B must have jesterky receipts with non-empty theme_count/annotated
- Arm A must have zero jesterky receipts
- Arm B uplift-over-baseline must beat Arm A
- Hollow annotate materialization is FAIL (not PASS)

## Artifacts

- Arm A result: external GameBench run `craftax_gelo_jesterky_workflow_arm_a_20260709T000814Z`.
- Arm B result: external GameBench run `craftax_gelo_jesterky_workflow_arm_b_20260709T010853Z`.

Configs:

- `proof/gelo_jesterky_workflow_arm_a.json`
- `proof/gelo_jesterky_workflow_arm_b.json`
