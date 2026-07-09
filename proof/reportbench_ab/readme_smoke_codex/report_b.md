# README Smoke Eval Summary

## Goal

Run a minimal SMR README smoke lane and confirm that a worker actor, not backend
bootstrap, replaced the project README and pushed a tiny proof bundle.

## Command

```bash
cd /Users/joshpurtell/Documents/GitHub/evals
python smr/reportbench/readme_smoke/workspace/run_smr_hello_world_e2e.py
```

## Worker Proof

- state: `done`
- verification_state: `passed`
- run_id: `003dfe99-59e3-4270-93ce-ed5fb9d29321`
- project_id: `a6dabb1d-1915-4aa2-a433-d1bb9d9c8d1b`
- wall_seconds: `94.62`
- attempts_used: `1`
- max_attempts: `2`
- retried_success: `False`
- worker_claim_pool_id: `slot1`
- post_bootstrap_commit_present=`True`
- readme_marker_present=`True`
- bootstrap_phrase_absent=`True`
- worker_proof_valid=`None`
- latest_run_state=`None`
- staged_files=`['README.md']`
- unexpected_repo_files=`[]`
- proof_ok=`True`

## Repo Bundle

- head_commit_sha: `0cce27f56af4eaba292ee7ec17f107e5f3d3cd34`
- default_branch: `main`
- commit_count: `3`
- latest_commit_message: `Add worker-authored README smoke bundle`
- file_errors: `{}`

## Sublinear Proof

- run_issue_id: `sublinear-issue-e67b7e8b-2a5a-470e-946b-59894b787a3b`
- matched_task_identifier: `SUB-211`
- matched_task_state_name: `Done`
- matched_task_state_type: `completed`
- sublinear_ok: `True`
- sublinear_projection_status: `ok`
- sublinear_projection_basis: `sublinear_ok=True with matched_task_state_type=completed`
- sublinear_failure_reasons: `[]`

## Actor Proof

- actor_count: `2`
- orchestrator_count: `1`
- worker_count: `1`
- orchestrator_actor_keys: `['be2bc4a0-3c46-45a0-b12d-21aaafc2551d']`
- worker_actor_keys: `['e28072d6-1820-4173-94dd-c6277e175440']`
- actor_topology_ok: `True`
- actor_failure_reasons: `[]`

## Runtime Bootstrap

- runtime_contract_source: `synth_dev_temp`
- stack_contract_path: `/Users/joshpurtell/Documents/GitHub/synth-dev/temp/slot1/stack-contract.json`
- stack_contract_validation_errors: `[]`
- runtime_bootstrap_error: `None`

## Task Graph

- write_readme_and_bundle: state=queued agent=None model=None worker_profile_id=None

- execution_config_snapshot_present=False
- orchestrator_profile_id=None
- default_worker_profile_id=None
- worker_profile_ids=None
- task_plan_ok=True
- repo_task_count=1
- duplicate_task_affinity_keys=[]
- task_plan_failure_reasons=[]

## Artifacts

- artifact_count=0
- artifact_types=[]
- best_reward=None
- report_workproduct_ok: `True`
- report_workproduct_basis: `Trace-evaluate verdict for this run records required artifacts present; local output artifact is listed below.`

- unavailable

Required WorkProduct path references:

- `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/readme_smoke_codex/RUBRIC.json`
- `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/readme_smoke_codex/GOLD_REPORT.md`
- `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/readme_smoke_codex/README.md`
- `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/readme_smoke_codex/eval_runthrough.md`
- `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/readme_smoke_codex/artifacts/run_result.json`

## Timing breakdown

- total_run_s=None
- queue_to_start_s=None
- first_turn_s=None
- recovery_to_all_terminal_s=None
- git_smoke_commit_s=None

## Cache Summary

- cache_requests_total=None
- cache_hits=None
- cache_misses=None
- cache_effective_hits_test=None
- cache_effective_hit_rate_test=None%

## LLM Usage

- gpt-5.4-mini: input=26969 cached_input=17920 output=1127 reasoning_output=242 snapshots=1

- totals.input_tokens=26969
- totals.cached_input_tokens=17920
- totals.non_cached_input_tokens=9049
- totals.output_tokens=1127
- totals.reasoning_output_tokens=242
- totals.non_reasoning_output_tokens=885
- session_snapshots_count=1
- sessions_seen=1

## Cost

- run_total_cost_cents=4
- run_total_cost_usd=0.04
- cost_source=smr_run_spend


## Consecutive-run comparison

- previous_artifact: `reportbench/lanes/readme_smoke/artifacts/20260420_024920_smr_readme_smoke_e2e.json`
- previous_run_id: `5a99eba5-f851-4d45-b8fc-0363fbc17735`
- previous_state: `failed`
- delta_total_run_s: None
- delta_first_turn_s: None
- delta_recovery_to_all_terminal_s: None
- delta_git_smoke_commit_s: None
- delta_cache_effective_hit_rate_test_percent: None


## Local Outputs

- Artifact: `reportbench/lanes/readme_smoke/artifacts/20260420_025123_smr_readme_smoke_e2e.json`
- Log: `reportbench/lanes/readme_smoke/runlogs.txt`
- Runner: `smr/reportbench/readme_smoke/workspace/run_smr_hello_world_e2e.py`
