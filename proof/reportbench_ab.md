# ReportBench A/B — baseline report vs trace-evaluate-guided revision

**Question:** does the `smr_reportbench_trace_evaluate` workflow's verdict, fed
back as guidance, improve a report's autograde score?

**Design (2026-07-09):** Arm A = each graded lane's report exactly as autograded
(reward read from `autograde_latest.json`; the SMR run is not re-executed).
Arm B = the same report revised by gpt-5.5 (`codex exec`, workspace scoped to
the lane workdir) given the rubric plus that lane's live trace-evaluate verdict
(verdict/severity/rationale from `proof/smr_reportbench_score.json`), under an
explicit no-fabrication instruction. Arm B is re-graded with the real evals
grader (`validate_report.grade_rubric`) via `scripts/reportbench_ab.py`.
Faithfulness gate: re-grading the UNREVISED copy reproduces arm A's reward
exactly (hello_world 0.7059, 12/17) before any revision ran.

## Result

| lane | arm A (as graded) | arm B (revised) | delta |
|------|------------------:|----------------:|------:|
| hello_world | 0.7059 (12/17) | 1.0 (17/17) | +0.2941 |
| readme_smoke | 1.0 (18/18) | 1.0 (19/19) | +0.0000 |
| readme_smoke_codex | 1.0 (18/18) | 1.0 (19/19) | +0.0000 |
| readme_smoke_deepseek | 1.0 (18/18) | 1.0 (19/19) | +0.0000 |

n=4 · mean A=0.9265 · mean B=1.0 · mean delta=+0.0735
(`proof/reportbench_ab/ab_table.json`; per-lane workdirs with PROMPT.md,
report_b.md, rubric_b.json, revision_summary.txt committed alongside.)

## Honest read

1. **The informative cell is hello_world** (+0.2941); the three readme lanes
   were already at ceiling and stayed there.
2. **Three of the five recovered checks are legitimate:** the revision added
   the required `Validation Results` / `Held-out Results` / `Prompt Summary`
   headings and reorganized existing content under them without inventing
   results.
3. **Two recovered checks expose rubric gameability, and we report that as the
   finding:** the `validation_metric_present` / `heldout_metric_present`
   regexes now match only because the revision quotes the expected pattern
   inside an explicit disclaimer — "the run did not produce
   `optimized_validation_accuracy: 0.0` … not a measured validation accuracy."
   The prose is honest; the check is satisfied by mention, not by measurement.
   A regex rubric cannot distinguish a reported metric from a quoted one.
4. **Check counts differ across arms for the readme lanes (18 vs 19):** arm A
   counts come from the recorded autograde (graded under the retired
   `evals/smr/reportbench` rubric copy); arm B is graded under the live lane
   `RUBRIC.json`, which has one more check. Rewards are comparable (both are
   pass fractions); counts are not identical by construction.
5. **Scope:** n=4 lanes, one revision per lane, one model. This measures
   "verdict-guided revision recovers rubric compliance," not end-to-end
   report-generation uplift — that A/B (guidance injected during the SMR run
   itself) remains the next rung.
