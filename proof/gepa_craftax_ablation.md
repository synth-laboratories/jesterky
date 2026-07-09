# GEPA-Craftax ablation - base ReAct prompt vs GEPA prompt

Target quantity: **Craftax mean reward** (`reward_info.outcome_reward`). Arms are
paired by seed and differ only in `policy.config.system_prompt`; model, decode config,
container, seed set, max steps, and output-format instructions are held constant.

## Result

| Arm | Prompt | Mean reward | n |
|-----|--------|------------:|--:|
| A (without) | base ReAct | **2.328** | 64 |
| B (with) | GEPA prompt | **3.006** | 64 |

**Uplift (B - A) = 0.678 mean reward**, 95% CI
[0.205, 1.151], two-sided exact sign-flip p =
0.007010, wins/ties/losses = 40/4/20.

Honest read: CI excludes 0; headline uplift claim allowed.

## Provenance

- Arm A prompt: `proof/gepa_craftax_ablation/ablation_summary.json`; `prompts.arm_a_base`; original source was the Craftax ReAct `DEFAULT_SYSTEM_PROMPT`.
- Arm B prompt: `proof/gepa_craftax_ablation/ablation_summary.json`; `prompts.arm_b_gepa`; original source was the selected GEPA proposal manifest field `proposals[0].proposed_payload.payload.react_system_prompt`.
- Container: `http://127.0.0.1:18104`; gold lane: `http://127.0.0.1:8098`
- Env: `gamebench.craftax-singleplayer.rust_gold`
- Model: `gemini-3.1-flash-lite`, temperature 0.0, max_tokens 512
- Seeds requested: 501-564; paired completed n=64
- Max steps: 64; max LLM turns: 12

## Fairness

The output-format instruction is identical across arms:

`Reply with JSON only, for example {"actions":["do","right","do","left","do"]}. Use valid actions: noop, left, right, up, down, do, sleep, place_stone, place_table, place_furnace, place_plant, make_wood_pickaxe, make_stone_pickaxe, make_iron_pickaxe, make_wood_sword, make_stone_sword, make_iron_sword.`

Only the strategic system-prompt content varies. Incomplete seeds are dropped from
both arms to preserve pairing.

## Artifacts

- `ablation_summary.json` - means, paired deltas, CI, permutation p, prompts, provenance
- `{base,gepa}-seed-*.rollout.json` - raw container rollout records
