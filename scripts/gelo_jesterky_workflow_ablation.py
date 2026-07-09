#!/usr/bin/env python3
"""GELO ± jesterky workflows ablation runner.

Compares two Craftax GELO/goex runs that differ only in
`go_ex.jesterky_workflow.enabled`:

  Arm A: enabled=false (no jesterky_* in core proposer workspace)
  Arm B: enabled=true  (annotate → materialize before each core propose)

Ship bar (fail closed unless --allow-pending):
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
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARM_A = ROOT / "proof" / "gelo_jesterky_workflow_arm_a.json"
DEFAULT_ARM_B = ROOT / "proof" / "gelo_jesterky_workflow_arm_b.json"
DEFAULT_OUT = ROOT / "proof" / "gelo_jesterky_workflow_ablation.md"
DEFAULT_SUMMARY = ROOT / "proof" / "gelo_jesterky_workflow_ablation" / "ablation_summary.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def candidate_source_counts(archive_path: Path | None, result: dict[str, Any]) -> Counter[str]:
    counts: Counter[str] = Counter()
    if archive_path and archive_path.is_file():
        archive = load_json(archive_path)
        candidates = archive.get("candidates")
        if isinstance(candidates, list):
            for cand in candidates:
                if not isinstance(cand, dict):
                    continue
                source = str(cand.get("source") or "unknown")
                counts[source] += 1
            return counts
    # Fallback: result_manifest only exposes aggregate count.
    n = int(result.get("candidate_count") or 0)
    if n:
        counts["unknown_aggregate"] = n
    return counts


def non_seed_count(counts: Counter[str]) -> int:
    return sum(n for source, n in counts.items() if source != "seed")


def jesterky_receipts(result: dict[str, Any], artifacts_dir: Path | None) -> list[dict[str, Any]]:
    receipts: list[dict[str, Any]] = []
    block = result.get("jesterky_workflow")
    if isinstance(block, dict):
        raw = block.get("receipts")
        if isinstance(raw, list):
            receipts.extend([r for r in raw if isinstance(r, dict)])
    if artifacts_dir:
        path = artifacts_dir / "jesterky_workflow_receipts.jsonl"
        if path.is_file():
            for line in path.read_text().splitlines():
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if isinstance(row, dict):
                    receipts.append(row)
    return receipts


def uplift_over_baseline(artifacts_dir: Path) -> dict[str, Any]:
    """Best non-seed search mean − baseline search mean from acceptance report."""
    acc_path = artifacts_dir / "goex_acceptance_report.json"
    out: dict[str, Any] = {
        "baseline_candidate_id": None,
        "baseline_search_mean": None,
        "best_nonseed_candidate_id": None,
        "best_nonseed_search_mean": None,
        "uplift_over_baseline": None,
        "champion_candidate_id": None,
        "promoted": None,
    }
    if not acc_path.is_file():
        return out
    acc = load_json(acc_path)
    out["baseline_candidate_id"] = acc.get("baseline_candidate_id")
    out["champion_candidate_id"] = acc.get("champion_candidate_id")
    out["promoted"] = acc.get("promoted")
    baseline_id = out["baseline_candidate_id"]
    baseline_search = None
    best_nonseed: tuple[str, float] | None = None
    for cand in acc.get("candidates") or []:
        if not isinstance(cand, dict):
            continue
        cid = cand.get("candidate_id")
        search = cand.get("search") if isinstance(cand.get("search"), dict) else {}
        mean = search.get("mean_reward")
        if not isinstance(mean, (int, float)):
            continue
        mean_f = float(mean)
        if cid == baseline_id or cand.get("source") == "seed":
            if cid == baseline_id:
                baseline_search = mean_f
            continue
        if best_nonseed is None or mean_f > best_nonseed[1]:
            best_nonseed = (str(cid), mean_f)
    out["baseline_search_mean"] = baseline_search
    if best_nonseed is not None:
        out["best_nonseed_candidate_id"] = best_nonseed[0]
        out["best_nonseed_search_mean"] = best_nonseed[1]
        if isinstance(baseline_search, float):
            out["uplift_over_baseline"] = best_nonseed[1] - baseline_search
    return out


def score_arm(
    *,
    label: str,
    result_path: Path,
    expect_jesterky: bool,
) -> dict[str, Any]:
    result = load_json(result_path)
    artifacts_dir = result_path.parent
    archive_path = artifacts_dir / "goex_archive.json"
    counts = candidate_source_counts(archive_path, result)
    receipts = jesterky_receipts(result, artifacts_dir)
    enabled_receipts = [r for r in receipts if r.get("enabled")]
    uplift = uplift_over_baseline(artifacts_dir)
    # Prefer acceptance-derived uplift; fall back to result_manifest fields.
    uplift_val = uplift.get("uplift_over_baseline")
    if uplift_val is None:
        uplift_val = result.get("uplift_over_baseline")
        if uplift_val is None:
            uplift_val = result.get("holdout_uplift")
    return {
        "label": label,
        "result_path": str(result_path),
        "candidate_count": int(result.get("candidate_count") or 0),
        "candidate_sources": dict(counts),
        "non_seed_candidates": non_seed_count(counts),
        "baseline_candidate_id": uplift.get("baseline_candidate_id")
        or result.get("baseline_candidate_id"),
        "baseline_search_mean": uplift.get("baseline_search_mean")
        if uplift.get("baseline_search_mean") is not None
        else result.get("baseline_search_mean"),
        "best_nonseed_candidate_id": uplift.get("best_nonseed_candidate_id"),
        "best_nonseed_search_mean": uplift.get("best_nonseed_search_mean")
        if uplift.get("best_nonseed_search_mean") is not None
        else result.get("best_nonseed_search_mean"),
        "uplift_over_baseline": uplift_val,
        "baseline_holdout_mean": result.get("baseline_holdout_mean"),
        "best_holdout_mean": result.get("best_holdout_mean"),
        "holdout_uplift": result.get("holdout_uplift"),
        "promoted": uplift.get("promoted")
        if uplift.get("promoted") is not None
        else result.get("promoted"),
        "champion_candidate_id": uplift.get("champion_candidate_id")
        or result.get("champion_candidate_id"),
        "jesterky_enabled_config": bool(
            ((result.get("jesterky_workflow") or {}) if isinstance(result.get("jesterky_workflow"), dict) else {}).get(
                "enabled"
            )
        )
        or expect_jesterky,
        "jesterky_receipt_count": len(enabled_receipts),
        "jesterky_receipts": enabled_receipts,
        "expect_jesterky": expect_jesterky,
    }


def evaluate(arm_a: dict[str, Any], arm_b: dict[str, Any], *, min_non_seed: int) -> dict[str, Any]:
    failures: list[str] = []
    if arm_a["non_seed_candidates"] < min_non_seed:
        failures.append(
            f"Arm A non-seed candidates {arm_a['non_seed_candidates']} < required {min_non_seed}"
        )
    if arm_b["non_seed_candidates"] < min_non_seed:
        failures.append(
            f"Arm B non-seed candidates {arm_b['non_seed_candidates']} < required {min_non_seed}"
        )
    if arm_b["jesterky_receipt_count"] < 1:
        failures.append("Arm B missing jesterky workflow receipts")
    if arm_a["jesterky_receipt_count"] > 0:
        failures.append("Arm A unexpectedly has jesterky receipts (force-absence violated)")

    # Hollow receipts (theme_count=0 / annotated=0 after export) are not a PASS —
    # proposers never saw usable jesterky signal. Ignore round_0 empty-evidence
    # receipts only when every post-export receipt is empty.
    hollow = [
        r
        for r in arm_b.get("jesterky_receipts") or []
        if isinstance(r, dict)
        and r.get("enabled")
        and int(r.get("theme_count") or 0) == 0
        and int(r.get("annotated") or 0) == 0
        and str(r.get("manifest_path") or "").strip()
    ]
    if hollow and len(hollow) == len(
        [r for r in (arm_b.get("jesterky_receipts") or []) if isinstance(r, dict) and r.get("enabled")]
    ):
        failures.append(
            f"Arm B jesterky receipts are hollow (theme_count=0/annotated=0 on "
            f"{len(hollow)} receipts); do not attribute uplift to jesterky content"
        )
    elif hollow:
        failures.append(
            f"Arm B has {len(hollow)} hollow jesterky receipt(s) with empty "
            f"theme/annotate signal after a successful annotate manifest"
        )

    a_uplift = arm_a.get("uplift_over_baseline")
    b_uplift = arm_b.get("uplift_over_baseline")
    beat = False
    if isinstance(b_uplift, (int, float)) and isinstance(a_uplift, (int, float)):
        beat = float(b_uplift) > float(a_uplift)
    else:
        failures.append("missing uplift_over_baseline for comparison")
    if not beat and "missing uplift_over_baseline" not in " ".join(failures):
        failures.append(
            f"Arm B did not beat Arm A on uplift-over-baseline "
            f"(A={a_uplift} best_nonseed={arm_a.get('best_nonseed_search_mean')} "
            f"baseline={arm_a.get('baseline_search_mean')}; "
            f"B={b_uplift} best_nonseed={arm_b.get('best_nonseed_search_mean')} "
            f"baseline={arm_b.get('baseline_search_mean')})"
        )
    return {
        "pass": not failures,
        "failures": failures,
        "arm_a": arm_a,
        "arm_b": arm_b,
        "min_non_seed": min_non_seed,
    }


def write_markdown(path: Path, report: dict[str, Any]) -> None:
    a = report["arm_a"]
    b = report["arm_b"]
    hollow_fail = any("hollow" in str(f).lower() for f in report.get("failures") or [])
    if report["pass"]:
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
                f"| A | off | {a['non_seed_candidates']} | "
                f"{a.get('baseline_search_mean')} | {a.get('best_nonseed_search_mean')} | "
                f"{a.get('uplift_over_baseline')} | {a['jesterky_receipt_count']} |"
            ),
            (
                f"| B | on | {b['non_seed_candidates']} | "
                f"{b.get('baseline_search_mean')} | {b.get('best_nonseed_search_mean')} | "
                f"{b.get('uplift_over_baseline')} | {b['jesterky_receipt_count']} |"
            ),
            "",
            f"- A best non-seed: `{a.get('best_nonseed_candidate_id')}`",
            f"- B best non-seed: `{b.get('best_nonseed_candidate_id')}`",
            f"- A sources: `{a['candidate_sources']}`",
            f"- B sources: `{b['candidate_sources']}`",
            "",
            "## Ship bar",
            "",
            f"- min non-seed candidates per arm: {report['min_non_seed']}",
            "- Arm B must have jesterky receipts with non-empty theme_count/annotated",
            "- Arm A must have zero jesterky receipts",
            "- Arm B uplift-over-baseline must beat Arm A",
            "- Hollow annotate materialization is FAIL (not PASS)",
            "",
        ]
    )
    if report["failures"]:
        lines.append("## Failures")
        lines.append("")
        for failure in report["failures"]:
            lines.append(f"- {failure}")
        lines.append("")
    lines.extend(
        [
            "## Artifacts",
            "",
            f"- Arm A result: `{a['result_path']}`",
            f"- Arm B result: `{b['result_path']}`",
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
        "--allow-pending",
        action="store_true",
        help="Exit 0 even if ship bar fails (writes pending proof).",
    )
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
        a_cfg = load_json(args.arm_a_config)
        b_cfg = load_json(args.arm_b_config)
        a_wf = (a_cfg.get("go_ex") or {}).get("jesterky_workflow") or {}
        b_wf = (b_cfg.get("go_ex") or {}).get("jesterky_workflow") or {}
        if bool(a_wf.get("enabled")):
            print("Arm A config must have jesterky_workflow.enabled=false", file=sys.stderr)
            return 2
        if not bool(b_wf.get("enabled")):
            print("Arm B config must have jesterky_workflow.enabled=true", file=sys.stderr)
            return 2
        print(
            json.dumps(
                {
                    "arm_a_enabled": False,
                    "arm_b_enabled": True,
                    "arm_a_config": str(args.arm_a_config),
                    "arm_b_config": str(args.arm_b_config),
                    "arm_a_proposer_rounds": (a_cfg.get("go_ex") or {}).get("proposer_rounds"),
                    "arm_b_proposer_rounds": (b_cfg.get("go_ex") or {}).get("proposer_rounds"),
                },
                indent=2,
            )
        )

    if not args.arm_a_result or not args.arm_b_result:
        if args.score_only:
            print("--score-only requires --arm-a-result and --arm-b-result", file=sys.stderr)
            return 2
        print(launch_hint(args.arm_a_config, "craftax_gelo_jesterky_workflow_arm_a"))
        print(launch_hint(args.arm_b_config, "craftax_gelo_jesterky_workflow_arm_b"))
        pending = {
            "pass": False,
            "failures": [
                "Arm A/B Craftax GELO runs not scored yet; launch configs then re-run with --score-only"
            ],
            "arm_a": {
                "label": "A",
                "result_path": "",
                "non_seed_candidates": 0,
                "candidate_sources": {},
                "baseline_search_mean": None,
                "best_nonseed_search_mean": None,
                "uplift_over_baseline": None,
                "jesterky_receipt_count": 0,
            },
            "arm_b": {
                "label": "B",
                "result_path": "",
                "non_seed_candidates": 0,
                "candidate_sources": {},
                "baseline_search_mean": None,
                "best_nonseed_search_mean": None,
                "uplift_over_baseline": None,
                "jesterky_receipt_count": 0,
            },
            "min_non_seed": args.min_non_seed,
            "status": "pending_runs",
        }
        args.out_summary.parent.mkdir(parents=True, exist_ok=True)
        args.out_summary.write_text(json.dumps(pending, indent=2) + "\n")
        write_markdown(args.out_md, pending)
        print(f"wrote pending proof {args.out_md}")
        return 0 if args.allow_pending else 1

    arm_a = score_arm(label="A", result_path=args.arm_a_result, expect_jesterky=False)
    arm_b = score_arm(label="B", result_path=args.arm_b_result, expect_jesterky=True)
    report = evaluate(arm_a, arm_b, min_non_seed=args.min_non_seed)
    args.out_summary.parent.mkdir(parents=True, exist_ok=True)
    args.out_summary.write_text(json.dumps(report, indent=2) + "\n")
    write_markdown(args.out_md, report)
    print(json.dumps({"pass": report["pass"], "failures": report["failures"]}, indent=2))
    print(f"wrote {args.out_md}")
    if report["pass"]:
        return 0
    return 0 if args.allow_pending else 1


if __name__ == "__main__":
    raise SystemExit(main())
