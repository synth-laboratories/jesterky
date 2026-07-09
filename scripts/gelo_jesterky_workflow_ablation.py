#!/usr/bin/env python3
"""GELO ± jesterky workflows ablation runner.

Compares two Craftax GELO/goex runs that differ only in
`go_ex.jesterky_workflow.enabled`:

  Arm A: enabled=false (no jesterky_* in core proposer workspace)
  Arm B: enabled=true  (annotate → materialize before each core propose)

Ship bar (fail closed):
  - both arms produce many non-seed candidates
  - Arm B has jesterky workflow receipts
  - Arm B uplift-over-baseline (best non-seed search mean − baseline
    search mean) beats Arm A

Modes:
  --score-only   score two existing result_manifest.json paths
  default        print launch commands / optionally exec if --launch

This does NOT reuse gepa_gelo_ablation.py (that is post-hoc prompt A/B).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from dataclasses import asdict, dataclass
from pathlib import Path

from json_contract import (
    JsonObject,
    JsonObjectReader,
    read_json_lines,
    read_json_object,
)

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARM_A = ROOT / "proof" / "gelo_jesterky_workflow_arm_a.json"
DEFAULT_ARM_B = ROOT / "proof" / "gelo_jesterky_workflow_arm_b.json"
DEFAULT_OUT = ROOT / "proof" / "gelo_jesterky_workflow_ablation.md"
DEFAULT_SUMMARY = (
    ROOT / "proof" / "gelo_jesterky_workflow_ablation" / "ablation_summary.json"
)


@dataclass(frozen=True)
class WorkflowReceipt:
    enabled: bool
    theme_count: int
    annotated: int
    manifest_path: str


@dataclass(frozen=True)
class AcceptanceCandidate:
    candidate_id: str
    source: str
    search_mean: float


@dataclass(frozen=True)
class AcceptanceResult:
    baseline_candidate_id: str
    baseline_search_mean: float
    best_nonseed_candidate_id: str
    best_nonseed_search_mean: float
    uplift_over_baseline: float
    champion_candidate_id: str
    promoted: bool


@dataclass(frozen=True)
class ArmResult:
    label: str
    result_path: str
    candidate_count: int
    candidate_sources: dict[str, int]
    non_seed_candidates: int
    baseline_candidate_id: str
    baseline_search_mean: float
    best_nonseed_candidate_id: str
    best_nonseed_search_mean: float
    uplift_over_baseline: float
    baseline_holdout_mean: float
    best_holdout_mean: float
    holdout_uplift: float
    promoted: bool
    champion_candidate_id: str
    jesterky_enabled_config: bool
    jesterky_receipts: tuple[WorkflowReceipt, ...]
    expect_jesterky: bool

    @property
    def jesterky_receipt_count(self) -> int:
        return len(self.jesterky_receipts)


@dataclass(frozen=True)
class AblationReport:
    passed: bool
    failures: tuple[str, ...]
    arm_a: ArmResult
    arm_b: ArmResult
    min_non_seed: int


def candidate_source_counts(archive_path: Path) -> Counter[str]:
    counts: Counter[str] = Counter()
    for candidate in read_json_object(archive_path).objects("candidates"):
        counts[candidate.string("source")] += 1
    return counts


def non_seed_count(counts: Counter[str]) -> int:
    return sum(n for source, n in counts.items() if source != "seed")


def workflow_receipts(path: Path) -> tuple[WorkflowReceipt, ...]:
    return tuple(
        WorkflowReceipt(
            enabled=row.boolean("enabled"),
            theme_count=row.integer("theme_count"),
            annotated=row.integer("annotated"),
            manifest_path=row.string("manifest_path"),
        )
        for row in read_json_lines(path)
    )


def acceptance_result(path: Path) -> AcceptanceResult:
    raw = read_json_object(path)
    baseline_id = raw.string("baseline_candidate_id")
    candidates: list[AcceptanceCandidate] = []
    for candidate in raw.objects("candidates"):
        search = candidate.object("search")
        candidates.append(
            AcceptanceCandidate(
                candidate_id=candidate.string("candidate_id"),
                source=candidate.string("source"),
                search_mean=search.number("mean_reward"),
            )
        )
    baseline = next(
        (
            candidate
            for candidate in candidates
            if candidate.candidate_id == baseline_id
        ),
        None,
    )
    if baseline is None:
        raise JsonContractError(
            f"{path} baseline candidate {baseline_id!r} is absent from candidates"
        )
    nonseed = [candidate for candidate in candidates if candidate.source != "seed"]
    if not nonseed:
        raise JsonContractError(f"{path} has no non-seed candidate")
    best = max(nonseed, key=lambda candidate: candidate.search_mean)
    return AcceptanceResult(
        baseline_candidate_id=baseline_id,
        baseline_search_mean=baseline.search_mean,
        best_nonseed_candidate_id=best.candidate_id,
        best_nonseed_search_mean=best.search_mean,
        uplift_over_baseline=best.search_mean - baseline.search_mean,
        champion_candidate_id=raw.string("champion_candidate_id"),
        promoted=raw.boolean("promoted"),
    )


def score_arm(
    *,
    label: str,
    result_path: Path,
    expect_jesterky: bool,
) -> ArmResult:
    result = read_json_object(result_path)
    artifacts_dir = result_path.parent
    archive_path = artifacts_dir / "goex_archive.json"
    counts = candidate_source_counts(archive_path)
    receipts = workflow_receipts(artifacts_dir / "jesterky_workflow_receipts.jsonl")
    acceptance = acceptance_result(artifacts_dir / "goex_acceptance_report.json")
    workflow = result.object("jesterky_workflow")
    candidate_count = result.integer("candidate_count")
    archive_count = sum(counts.values())
    if candidate_count != archive_count:
        raise ValueError(
            f"{result_path} candidate_count={candidate_count} disagrees with "
            f"goex_archive.json count={archive_count}"
        )
    enabled = workflow.boolean("enabled")
    if enabled != expect_jesterky:
        raise ValueError(
            f"{result_path} jesterky_workflow.enabled={enabled} does not match "
            f"expected arm setting {expect_jesterky}"
        )
    if expect_jesterky and any(not receipt.enabled for receipt in receipts):
        raise ValueError(f"{result_path} contains disabled receipts in the enabled arm")
    return ArmResult(
        label=label,
        result_path=str(result_path),
        candidate_count=candidate_count,
        candidate_sources=dict(counts),
        non_seed_candidates=non_seed_count(counts),
        baseline_candidate_id=acceptance.baseline_candidate_id,
        baseline_search_mean=acceptance.baseline_search_mean,
        best_nonseed_candidate_id=acceptance.best_nonseed_candidate_id,
        best_nonseed_search_mean=acceptance.best_nonseed_search_mean,
        uplift_over_baseline=acceptance.uplift_over_baseline,
        baseline_holdout_mean=result.number("baseline_holdout_mean"),
        best_holdout_mean=result.number("best_holdout_mean"),
        holdout_uplift=result.number("holdout_uplift"),
        promoted=acceptance.promoted,
        champion_candidate_id=acceptance.champion_candidate_id,
        jesterky_enabled_config=enabled,
        jesterky_receipts=receipts,
        expect_jesterky=expect_jesterky,
    )


def evaluate(
    arm_a: ArmResult, arm_b: ArmResult, *, min_non_seed: int
) -> AblationReport:
    failures: list[str] = []
    if arm_a.non_seed_candidates < min_non_seed:
        failures.append(
            f"Arm A non-seed candidates {arm_a.non_seed_candidates} < required {min_non_seed}"
        )
    if arm_b.non_seed_candidates < min_non_seed:
        failures.append(
            f"Arm B non-seed candidates {arm_b.non_seed_candidates} < required {min_non_seed}"
        )
    if arm_b.jesterky_receipt_count < 1:
        failures.append("Arm B missing jesterky workflow receipts")
    if arm_a.jesterky_receipt_count > 0:
        failures.append(
            "Arm A unexpectedly has jesterky receipts (force-absence violated)"
        )

    # Hollow receipts (theme_count=0 / annotated=0 after export) are not a PASS —
    # proposers never saw usable jesterky signal. Ignore round_0 empty-evidence
    # receipts only when every post-export receipt is empty.
    hollow = [
        receipt
        for receipt in arm_b.jesterky_receipts
        if receipt.enabled
        and receipt.theme_count == 0
        and receipt.annotated == 0
        and receipt.manifest_path
    ]
    if hollow and len(hollow) == arm_b.jesterky_receipt_count:
        failures.append(
            f"Arm B jesterky receipts are hollow (theme_count=0/annotated=0 on "
            f"{len(hollow)} receipts); do not attribute uplift to jesterky content"
        )
    elif hollow:
        failures.append(
            f"Arm B has {len(hollow)} hollow jesterky receipt(s) with empty "
            f"theme/annotate signal after a successful annotate manifest"
        )

    if arm_b.uplift_over_baseline <= arm_a.uplift_over_baseline:
        failures.append(
            f"Arm B did not beat Arm A on uplift-over-baseline "
            f"(A={arm_a.uplift_over_baseline} best_nonseed={arm_a.best_nonseed_search_mean} "
            f"baseline={arm_a.baseline_search_mean}; "
            f"B={arm_b.uplift_over_baseline} best_nonseed={arm_b.best_nonseed_search_mean} "
            f"baseline={arm_b.baseline_search_mean})"
        )
    return AblationReport(
        passed=not failures,
        failures=tuple(failures),
        arm_a=arm_a,
        arm_b=arm_b,
        min_non_seed=min_non_seed,
    )


def write_markdown(path: Path, report: AblationReport) -> None:
    a = report.arm_a
    b = report.arm_b
    hollow_fail = any("hollow" in failure.lower() for failure in report.failures)
    if report.passed:
        status = "PASS"
    elif hollow_fail:
        status = "INVALID — do not cite as PASS (hollow jesterky materialization)"
    else:
        status = "FAIL / PENDING"
    lines = [
        "# GELO ± jesterky workflows ablation",
        "",
        f"**Status:** {status}",
        "",
        "Claim shape: GELO with jesterky trace annotate/flag/process workflows vs",
        "GELO without them. Arms differ only in `go_ex.jesterky_workflow.enabled`.",
        "",
        "Primary metric: **uplift over baseline** = best non-seed search mean −",
        "baseline search mean (from `goex_acceptance_report.json`).",
        "",
    ]
    if hollow_fail:
        lines.extend(
            [
                "## Invalidation",
                "",
                "Arm B receipts exist but `theme_count=0` / `annotated=0`. Annotate may",
                "have produced themes while goex failed to materialize them — uplift",
                "numbers below are **not** attributable to jesterky content.",
                "",
            ]
        )
    lines.extend(
        [
            "## Results",
            "",
            "| Arm | jesterky | non-seed | baseline search | best non-seed search | uplift over baseline | receipts |",
            "|-----|----------|---------:|----------------:|---------------------:|---------------------:|---------:|",
            (
                f"| A | off | {a.non_seed_candidates} | "
                f"{a.baseline_search_mean} | {a.best_nonseed_search_mean} | "
                f"{a.uplift_over_baseline} | {a.jesterky_receipt_count} |"
            ),
            (
                f"| B | on | {b.non_seed_candidates} | "
                f"{b.baseline_search_mean} | {b.best_nonseed_search_mean} | "
                f"{b.uplift_over_baseline} | {b.jesterky_receipt_count} |"
            ),
            "",
            f"- A best non-seed: `{a.best_nonseed_candidate_id}`",
            f"- B best non-seed: `{b.best_nonseed_candidate_id}`",
            f"- A sources: `{a.candidate_sources}`",
            f"- B sources: `{b.candidate_sources}`",
            "",
            "## Ship bar",
            "",
            f"- min non-seed candidates per arm: {report.min_non_seed}",
            "- Arm B must have jesterky receipts with non-empty theme_count/annotated",
            "- Arm A must have zero jesterky receipts",
            "- Arm B uplift-over-baseline must beat Arm A",
            "- Hollow annotate materialization is FAIL (not PASS)",
            "",
        ]
    )
    if report.failures:
        lines.append("## Failures")
        lines.append("")
        for failure in report.failures:
            lines.append(f"- {failure}")
        lines.append("")
    lines.extend(
        [
            "## Artifacts",
            "",
            f"- Arm A result: `{a.result_path}`",
            f"- Arm B result: `{b.result_path}`",
            "",
            "Configs:",
            "",
            f"- `{DEFAULT_ARM_A}`",
            f"- `{DEFAULT_ARM_B}`",
            "",
        ]
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n")


def launch_hint(config: Path, run_id: str) -> str:
    return (
        "# Launch locally once Craftax container + gold are up, e.g.:\n"
        f"#   <goex-runner> --config {config} --run-id {run_id}\n"
        "# Or materialize via synth-optimizers and submit/host as appropriate.\n"
    )


@dataclass(frozen=True)
class GoExConfigSummary:
    enabled: bool
    proposer_rounds: int


def goex_config_summary(raw: JsonObjectReader, path: Path) -> GoExConfigSummary:
    go_ex = raw.object("go_ex")
    workflow = go_ex.object("jesterky_workflow")
    return GoExConfigSummary(
        enabled=workflow.boolean("enabled"),
        proposer_rounds=go_ex.integer("proposer_rounds"),
    )


def assert_configs_differ_only_by_workflow(
    arm_a: JsonObjectReader,
    arm_b: JsonObjectReader,
    arm_a_path: Path,
    arm_b_path: Path,
) -> tuple[GoExConfigSummary, GoExConfigSummary]:
    summary_a = goex_config_summary(arm_a, arm_a_path)
    summary_b = goex_config_summary(arm_b, arm_b_path)
    if summary_a.enabled:
        raise ValueError("Arm A config must have jesterky_workflow.enabled=false")
    if not summary_b.enabled:
        raise ValueError("Arm B config must have jesterky_workflow.enabled=true")
    go_ex_a: JsonObject = {**arm_a.object("go_ex").data}
    go_ex_b: JsonObject = {**arm_b.object("go_ex").data}
    go_ex_a["jesterky_workflow"] = {
        **arm_a.object("go_ex").object("jesterky_workflow").data,
        "enabled": False,
    }
    go_ex_b["jesterky_workflow"] = {
        **arm_b.object("go_ex").object("jesterky_workflow").data,
        "enabled": False,
    }
    comparable_a: JsonObject = {
        **arm_a.data,
        "go_ex": go_ex_a,
        "run": {**arm_a.object("run").data, "run_id": "<arm-run-id>"},
    }
    comparable_b: JsonObject = {
        **arm_b.data,
        "go_ex": go_ex_b,
        "run": {**arm_b.object("run").data, "run_id": "<arm-run-id>"},
    }
    if comparable_a != comparable_b:
        raise ValueError(
            "Arm A/B configs differ outside `go_ex.jesterky_workflow.enabled` "
            "and the required distinct `run.run_id`"
        )
    return summary_a, summary_b


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arm-a-result", type=Path, help="Arm A result_manifest.json")
    parser.add_argument("--arm-b-result", type=Path, help="Arm B result_manifest.json")
    parser.add_argument("--arm-a-config", type=Path, default=DEFAULT_ARM_A)
    parser.add_argument("--arm-b-config", type=Path, default=DEFAULT_ARM_B)
    parser.add_argument("--min-non-seed", type=int, default=3)
    parser.add_argument("--out-md", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--out-summary", type=Path, default=DEFAULT_SUMMARY)
    parser.add_argument(
        "--score-only",
        action="store_true",
        help="Require --arm-a-result and --arm-b-result; do not print launch hints.",
    )
    parser.add_argument(
        "--validate-configs",
        action="store_true",
        help="Validate Arm A/B configs differ only on jesterky_workflow.enabled.",
    )
    args = parser.parse_args()

    if args.validate_configs or not args.score_only:
        a_cfg = read_json_object(args.arm_a_config)
        b_cfg = read_json_object(args.arm_b_config)
        a_summary, b_summary = assert_configs_differ_only_by_workflow(
            a_cfg,
            b_cfg,
            args.arm_a_config,
            args.arm_b_config,
        )
        print(
            json.dumps(
                {
                    "arm_a_enabled": False,
                    "arm_b_enabled": True,
                    "arm_a_config": str(args.arm_a_config),
                    "arm_b_config": str(args.arm_b_config),
                    "arm_a_proposer_rounds": a_summary.proposer_rounds,
                    "arm_b_proposer_rounds": b_summary.proposer_rounds,
                },
                indent=2,
            )
        )

    if not args.arm_a_result or not args.arm_b_result:
        if args.score_only:
            print(
                "--score-only requires --arm-a-result and --arm-b-result",
                file=sys.stderr,
            )
            return 2
        print(launch_hint(args.arm_a_config, "craftax_gelo_jesterky_workflow_arm_a"))
        print(launch_hint(args.arm_b_config, "craftax_gelo_jesterky_workflow_arm_b"))
        print(
            "Arm A/B runs are required before proof artifacts can be written; "
            "re-run with both --arm-a-result and --arm-b-result.",
            file=sys.stderr,
        )
        return 1

    arm_a = score_arm(label="A", result_path=args.arm_a_result, expect_jesterky=False)
    arm_b = score_arm(label="B", result_path=args.arm_b_result, expect_jesterky=True)
    report = evaluate(arm_a, arm_b, min_non_seed=args.min_non_seed)
    args.out_summary.parent.mkdir(parents=True, exist_ok=True)
    report_payload = asdict(report)
    report_payload["pass"] = report_payload.pop("passed")
    args.out_summary.write_text(json.dumps(report_payload, indent=2) + "\n")
    write_markdown(args.out_md, report)
    print(json.dumps({"pass": report.passed, "failures": report.failures}, indent=2))
    print(f"wrote {args.out_md}")
    return 0 if report.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
