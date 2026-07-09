# Handoff — GEPA & GELO Craftax ablations at defensible n

**Owner of this doc:** Claude (jesterky release push, 07-08). **You:** the engineer
running the real ablations. **Why this exists:** the tonight smoke run (n=8) is
statistically invalid and cannot be the release headline. This handoff gives you the
proven harness, the real intervention prompts, the bar, and the honesty rules so you
can produce a defensible with/without ablation for the launch blog.

---

## 0. What is wrong with the current result (do not ship it)

`proof/gepa_craftax_ablation.md` reports GEPA base-ReAct → GEPA-prompt uplift of
**+0.85 mean reward at n=8**, 7 wins / 1 loss, sign-flip permutation **p=0.137**.
That is directional, not significant. One bad seed (501: GEPA 0.6 vs base 4.0)
carries most of the variance (sd 1.96). At n=8 you cannot separate the intervention
from noise. **The blog needs a powered result or it makes no claim.**

Keep the file and its 16 raw records as the smoke-run audit trail — do not delete —
but the headline table gets replaced by your n≥64 run.

---

## 1. The bar (acceptance)

- **n ≥ 64 per arm, paired by seed** — matches the documented champion-cycle scale.
  The precedent (heuristic-policy, not prompt) is mean delta 3.7156, 95% CI
  **[3.1859, 4.2422]**, n=64, at
  `evals/projectbench/_runs/factory/craftax_cybernetics_effort/20260705T000000Z-always-on-local/artifacts/gamebench_hillclimb_heldout/candidates/{baseline,improved_policy}/summary.json`.
  Your n=64 is the *minimum*; go to n=128 if container time allows (≈5s/episode →
  n=128×2 arms ≈ 21 min/optimizer).
- **Report proper statistics** per ablation: mean per arm, paired mean delta,
  bootstrap or normal 95% CI on the delta, exact sign-flip permutation p (two-sided),
  and paired wins/ties/losses. A claim ships only if the **CI excludes 0** (or p<0.05).
- **Fairness (AT-4):** arms differ ONLY in the intervention (the system prompt).
  Same container, same seed set, same model, same decode config, same output-format
  instructions, same max_steps/max_llm_turns.
- **No fabrication.** If an arm flakes, report actual completed counts and drop
  incomplete seeds from BOTH arms (keep pairing intact). A smaller honest n beats a
  padded one. If you cannot find a *real* GELO-proposed prompt (see §3), GELO drops
  from the headline — do not invent one.

---

## 2. The proven harness (reuse verbatim)

The smoke run proved this path runs: 16/16 episodes completed, ~5s each.

- **Container:** `gamebench-craftax-rust-react` at `http://127.0.0.1:18104`,
  gold lane rust at `http://127.0.0.1:8098`. Confirm both are up before starting
  (`curl -s :18104/health`, `curl -s :8098/health` or the container's health route);
  restart per the gamebench craftax runbook if down. Do NOT change ports mid-run.
- **Env:** `gamebench.craftax-singleplayer.rust_gold`, reward =
  `reward_info.outcome_reward` (float; the smoke run's per-seed `reward` field).
- **Model:** `gemini-3.1-flash-lite`, temperature 0, max_tokens 512. (Codex/ChatGPT
  auth rules do not apply here — this is the gamebench container's own model route,
  not a Synth product route.)
- **Rollout config:** `max_steps: 64`, `max_llm_turns: 12`.
- **Invocation:** POST to the container's `/rollout` per (seed, arm). The swap point
  is `policy.config.system_prompt` — that one field is the entire intervention. The
  response is a rollout record whose top-level keys are
  `reward_info, artifact, artifacts, checkpoint, metadata, rollout_id, status, …`;
  read `reward_info.outcome_reward`, assert `status == "completed"`, and persist the
  full record.
- **Seeds:** the smoke run used 501–508. Extend contiguously: 501–564 (n=64) or
  501–628 (n=128). Use the SAME seeds for both arms and both optimizers so every
  comparison is paired.
- **Runner:** the smoke agent did NOT commit its runner script. Reference clients for
  the same container live at
  `gamebench/tasks/craftax-singleplayer/scripts/run_policy_eval_and_chart.py` and
  `run_policy_sweep.py` — model your POST loop on those (they already speak this
  container's rollout contract). Commit your runner to
  `jesterky/scripts/gepa_gelo_ablation.py` this time so the run is reproducible.

---

## 3. The real intervention prompts (this is the load-bearing part)

**Arm A (both optimizers share it) — base ReAct.** The container's verbatim
`DEFAULT_SYSTEM_PROMPT`:
`gamebench/tasks/craftax-singleplayer/containers/react/agent_policy.py` (the smoke
run cited lines 44–49). Also stored verbatim as `prompts.arm_a_base` in
`proof/gepa_craftax_ablation/ablation_summary.json`.

**Arm B — GEPA.** The real GEPA proposer output, field `react_system_prompt`,
`proposal_type=parent_variation`:
`efforts/craftax-agent-hillclimb-20260705t162228z/findings/proof/20260705T162555Z/artifacts/gepa_runs/gepa_dc6d949fac464ca7b07f2291791081bb/proposer_workspaces/generation_000/proposal/manifest.json`.
Stored verbatim as `prompts.arm_b_gepa` in the smoke summary — reuse that exact text.
(There is a second GEPA run at `…/20260705T163005Z/…/stackeval_craftax-local-gepa_…`
— do NOT mix; pin the same one the smoke run used unless you re-derive both cleanly.)

**Arm B — GELO.** A real GELO-optimized Craftax prompt must come from an actual GELO
run. Candidates (Go-Ex/GELO craftax runs on the same rust gold env):
`efforts/*/lane-*/workspace/gamebench/tasks/craftax-singleplayer/configs/reports/goex_runs/craftax_gamebench_rust_gelo_20260626T1915Z/`
(present under `craftax-gemini-policy-independent-20260708`,
`craftax-code-policy-independent-20260708`, `harvey-gemini-policy-independent-20260708`).
Find the optimized `system_prompt` / policy-text artifact in one of those run dirs and
use it verbatim. **If none contains a genuine GELO-proposed prompt for this env, GELO
drops from the headline — do not synthesize a "GELO-style" prompt.** Report which run
dir + file you pulled it from in the table's provenance section.

**Fairness normalization (mandatory, disclose it).** The GEPA (and likely GELO)
proposer targeted a different container output contract (`<tool_call>` /
`crafter_interact`). Hold the **output-format instruction identical across all arms**
(the `:18104` container's `{"actions":[...]}` JSON contract) so the only variable is
the optimizer's *strategic* control policy. The smoke run did exactly this; keep that
normalization and state it in the table.

---

## 4. Output artifacts (what you commit)

Under `jesterky/proof/`:

- `gepa_craftax_ablation.md` — **replace** the result table with your n≥64 numbers:
  per-arm mean, paired delta + 95% CI, permutation p, wins/ties/losses, n. Keep the
  provenance + fairness sections; update n and the "honest read" to match significance.
- `gelo_craftax_ablation.md` — same structure for GELO (or a one-line note that GELO
  was dropped for lack of a real proposed prompt, with what you searched).
- `gepa_craftax_ablation/` and `gelo_craftax_ablation/` — the raw rollout record per
  (seed, arm), plus an `ablation_summary.json` (means, per-seed rewards, paired
  deltas, CI, p, both full prompts). Mirror the smoke run's summary schema.
- `jesterky/scripts/gepa_gelo_ablation.py` — the runner, so the run reproduces.

Then update `proof/RELEASE_ABLATION_STATUS.md`: flip the AT-3 GEPA/GELO lines with the
real n and significance verdict.

---

## 5. Honesty & scope rules (non-negotiable)

- Never fabricate or round-launder a number. A paired n=48 that survives (some seeds
  flaked) beats a claimed n=64 that didn't run.
- The intervention under test is the **prompt**, nothing else. If you touch decode
  params, max_steps, or the output contract, the ablation is void.
- The champion Δ3.72@n=64 is a **heuristic-policy** result — cite it for scale only,
  never present it as the prompt ablation.
- Code never grades the agent's play; the target is the container's
  `outcome_reward`, a pure environment outcome (house rule: outcomes only).
- Do NOT `git commit` unless Josh asks — write files into `proof/`, report the paths.
- This is EVAL work; the runtime (jesterky) is not in the loop for the raw rollouts.
  These ablations validate the optimizer→prompt claim the blog makes; they run against
  the gamebench container directly, exactly as the smoke run did.

---

## 6. Definition of done

1. GEPA ablation at n≥64, paired, with CI + permutation p → committed table + raw
   records + runner script.
2. GELO ablation at the same n with a **real** GELO prompt (or a documented drop).
3. `RELEASE_ABLATION_STATUS.md` AT-3 lines reflect the real result and significance.
4. One sentence you can defend: "base ReAct → {optimizer} prompt lifts Craftax mean
   reward by X (95% CI [lo, hi], n=…, p=…), arms differ only in the system prompt."
   If the CI includes 0, the honest sentence says so and the blog reframes to
   "runnable + directional," not "proven."
