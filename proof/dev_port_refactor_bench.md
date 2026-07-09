# Dev-port refactor bench — held-out faithful-port generalization

**Claim:** a jesterky workflow can run a model *inside* a real environment — a
seeded workspace-write sandbox holding a working Python game engine, a
train-subset oracle, and a verifier — and grade the captured artifact on
held-out scenarios the model never saw. Held-out pass rate measures
faithful-port generalization, not memorization.

Bench home: `gamebench/tasks/dev-port-singleplayer` (spec
`dev_port_to_rust.sandboxed.json`, runner `run_sandboxed.sh`). The porter runs
the env: it reads `gold_python/`, writes a Rust crate exposing a `scenario`
binary (scenario JSON on stdin → `{"events":[...]}` on stdout), builds it, runs
`check.py` against the train oracle, and iterates. The crate is captured from
the workspace by manifest globs and scored on ALL scenarios against the full
NEV oracle (`score_port.py`).

## Protocol

- 4 train scenarios seeded (TRAIN=2 for earthborne, which has only 4 total);
  the rest held out. Score = pass rate over ALL scenarios against the full
  oracle; the model saw oracle event logs for only the train subset, so on a
  hard engine a model that memorizes the train diff tops out at train/total.
- gpt-5.5 runs native codex (ChatGPT-bundle auth). Gemini and DeepSeek run the
  same agentic loop through `jesterky-proxy` (Responses⇄chat tool-call
  translation + thought-signature round-trip).
- 10-minute wall-clock cap per run (`TIMEOUT=600` in `run_sandboxed.sh`): a
  watchdog kills the run and the partial workspace crate is scored
  (`score_port.py --candidate`) — never a fabricated 0.
- Artifacts per cell: `port.sandbox.<model>.<engine>.json` (run manifest) and
  `score.sandbox.<model>.<engine>.json` (per-scenario pass/fail), committed in
  the bench dir. Runs re-executed 2026-07-09 with per-engine artifact names.

## Results (all-scenario pass rate, single run per cell, 2026-07-09)

| engine | gold LOC | scenarios | gpt-5.5 | gemini-3.1-pro-preview | deepseek-v4 (capped) |
|--------|---------:|----------:|--------:|-----------------------:|---------------------:|
| tictactoe | 1634 | 20 | **1.0** (20/20) | 0.8 (16/20) | 0.8 (16/20) |
| sokoban | 1206 | 15 | **1.0** (15/15) | 0.333 (5/15) | 0.333 (5/15) |
| minihack | 1036 | 27 | 0.148 (4/27) | 0.148 (4/27) | 0.0 (build fail at cap) |
| crafter | 3723 | 34 | 0.118 (4/34) | 0.0 (0/34) | 0.0 (build fail at cap) |
| earthborne (train=2) | 1916 | 4 | **1.0** (4/4) | 0.0 (runtime panic) | **1.0** (4/4) |

## Findings

1. **The cliff is mechanic complexity, not LOC.** Minihack has the fewest lines
   on the ladder and defeats every model — its nethack-derived mechanics span
   27 diverse scenarios. Crafter (3.7k LOC) does the same. Earthborne, the
   largest mid-tier engine, is fully ported by two of three models: bigger but
   regular.
2. **On engines it cannot fully port, gpt-5.5 lands exactly on the train
   subset.** Minihack 4/27 and crafter 4/34 are precisely the 4 seeded train
   scenarios — a runnable crate that reproduces only what it can diff, and
   zero held-out generalization. Where the engine is within reach (tictactoe,
   sokoban, earthborne) it ports faithfully at 1.0.
3. **Gemini's failure mode is the under-port.** Its lean crates build and then
   panic or mismatch at runtime (earthborne: builds ok, `scenario` bin exits
   101 on every scenario; crafter 0/34 on a successful build).
4. **DeepSeek is cap-bound.** It finishes small regular engines faithfully
   (earthborne 1.0) and is killed mid-port on the hard ones; its scores are
   statements about throughput under the 10-minute cap as much as ability.
5. **Single-run cells swing.** Across two sessions one day apart: gpt-5.5
   sokoban 0.267 → 1.0, gemini sokoban 1.0 → 0.333, gemini earthborne
   1.0 → 0.0, deepseek tictactoe 1.0 → 0.8. The cliff (findings 1–2) is stable
   across both sessions; mid-ladder cells are indicative, not point estimates.
   Multi-seed runs are the next rung before any per-model ranking claim.

## Env inventory for future rungs

tictactoe 1634 LOC · sokoban 1206 · minihack 1036 · frogs 966 (4 scen) ·
earthborne 1916 (4) · crafter 3723 (34) · craftax 5620 (36) · rogue 13076 (4).
The natural next initiative is an optimizer over this bench — the porter
prompt/scaffold climbing the ladder against a real held-out metric.
