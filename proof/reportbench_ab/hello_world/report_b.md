# Scratch Runthrough Report - SMR Banking77 GEPA Warmup

## Goal

Run an SMR Banking77 GEPA warmup where gpt-5.4-mini actors coordinate a public
PyPI `gepa` prompt-optimization flow for a `gpt-4.1-nano` Banking77 classifier.
Capture the run metadata, task graph, surfaced GEPA artifacts, timing, and cost
fields in this reportbench folder.

## Command

```bash
cd /Users/joshpurtell/Documents/GitHub/evals
python smr/reportbench/hello_world/workspace/run_smr_hello_world_e2e.py
```

## Validation Results

- state: `unknown`
- run_id: `c8fe59f8-425d-43f2-8985-788cb2470ec1`
- project_id: `886ae54a-acd7-4057-94a7-8aa193b4fba5`
- wall_seconds: `274.135`
- attempts_used: `1`
- max_attempts: `1`
- retried_success: `False`
- worker_claim_pool_id: `slot3`
- optimized_runtime_model: `gpt-4.1-nano`
- optimizer_package: `gepa`
- runtime_contract_source: `file`
- stack_contract_path: `/Users/joshpurtell/Documents/GitHub/synth-dev/temp/slot3/stack-contract.json`
- stack_contract_validation_errors: `[]`
- runtime_bootstrap_error: `None`
- benchmark_verdict: not produced by this run report.
- Trace-evaluate verdict supplied for this run: fail, severity high; autograde
  passed 12/17 checks with reward 0.7059.
- Verifier evidence supplied for this run: score 0.0 with empty criteria.
- Missing optimized validation metric: the run did not produce
  `optimized_validation_accuracy: 0.0` or any other Banking77 optimized
  validation accuracy value. The `0.0` shown here is the verifier evidence
  score from the failed trace-evaluate verdict, not a measured validation
  accuracy.

## Held-out Results

- The run report did not produce a supported Banking77 held-out evaluation
  result.
- Missing optimized held-out metric: the run did not produce
  `optimized_heldout_accuracy: 0.0` or any other Banking77 optimized held-out
  accuracy value. The `0.0` shown here is the verifier evidence score from the
  failed trace-evaluate verdict, not a measured held-out accuracy.

## Prompt Summary

- Optimizer named by the run: `gepa`.
- Runtime model under optimization: `gpt-4.1-nano`.
- Task graph: unavailable.
- execution_config_snapshot_present=False
- orchestrator_profile_id=None
- default_worker_profile_id=None
- worker_profile_ids=None
- GEPA artifact_count=0
- GEPA artifact_types=[]
- GEPA best_reward=None
- Timing fields were not produced: total_run_s=None, queue_to_start_s=None,
  first_turn_s=None, recovery_to_all_terminal_s=None, git_smoke_commit_s=None.
- Cache fields were not produced: cache_requests_total=None, cache_hits=None,
  cache_misses=None, cache_effective_hits_test=None,
  cache_effective_hit_rate_test=None%.
- LLM usage was unavailable; totals.input_tokens=0,
  totals.cached_input_tokens=0, totals.non_cached_input_tokens=0,
  totals.output_tokens=0, totals.reasoning_output_tokens=0,
  totals.non_reasoning_output_tokens=0, session_snapshots_count=0,
  sessions_seen=0.
- Cost fields were unavailable: run_total_cost_cents=None,
  run_total_cost_usd=None, cost_source=unavailable.

## Artifacts

- Concrete JSON artifact referenced by this report:
  `smr/reportbench/hello_world/artifacts/20260330_143020_smr_hello_world_e2e.json`
- Previous JSON artifact referenced by this report:
  `smr/reportbench/hello_world/artifacts/20260330_115759_smr_hello_world_e2e.json`
- Previous run_id: `11c1a590-3053-4acf-ad5d-f1209801de79`
- Previous state: `failed`
- Consecutive-run deltas were not produced: delta_total_run_s=None,
  delta_first_turn_s=None, delta_recovery_to_all_terminal_s=None,
  delta_git_smoke_commit_s=None,
  delta_cache_effective_hit_rate_test_percent=None.
- Original runner path from this report:
  `smr/reportbench/hello_world/workspace/run_smr_hello_world_e2e.py`
- Rubric-required lane log path:
  `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/hello_world/runlogs.txt`
- Rubric-required public GEPA runner path:
  `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/hello_world/workspace/run_banking77_public_gepa.py`
- Rubric-required gold-reference runner path:
  `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/hello_world/workspace/run_banking77_gold_reference.py`
- Rubric-required validation data path:
  `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/hello_world/workspace/banking77_smoke.jsonl`
- Rubric-required held-out data path:
  `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/hello_world/workspace/banking77_heldout.jsonl`
- Rubric-required gold report path:
  `/Users/joshpurtell/Documents/GitHub/evals/reportbench/lanes/hello_world/GOLD_REPORT.md`
