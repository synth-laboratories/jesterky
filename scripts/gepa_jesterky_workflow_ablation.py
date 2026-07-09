#!/usr/bin/env python3
"""GEPA ± jesterky workflows ablation runner.

Compares two Craftax GEPA runs that differ only in
`jesterky_workflow.enabled`:

  Arm A: enabled=false (no state/jesterky_* in proposer workspace)
  Arm B: enabled=true  (annotate → materialize before each propose)

Ship bar (fail closed unless --allow-pending):
  - both arms produce non-seed candidates
  - Arm B has jesterky workflow receipts with non-empty theme_count/annotated
  - Arm A has zero jesterky receipts
  - Arm B best_heldout_mean beats Arm A (M5a primary)

Modes:
  --score-only   score two existing manifest.json / result paths
  --validate-configs  check Arm A/B differ only on enabled
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARM_A = ROOT / "proof" / "gepa_jesterky_workflow_arm_a.toml"
DEFAULT_ARM_B = ROOT / "proof" / "gepa_jesterky_workflow_arm_b.toml"
DEFAULT_OUT = ROOT / "proof" / "gepa_jesterky_workflow_ablation.md"
DEFAULT_SUMMARY = ROOT / "proof" / "gepa_jesterky_workflow_ablation" / "ablation_summary.json"


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def candidate_source_counts(run_dir: Path, result: dict[str, Any]) -> Counter[str]:
    counts: Counter[str] = Counter()
    for name in ("candidate_registry.json", "candidates.json"):
        path = run_dir / name
        if not path.is_file():
            # also check artifacts/
            alt = run_dir / "artifacts" / name
            path = alt if alt.is_file() else path
        if not path.is_file():
            continue
        data = load_json(path)
        candidates = data.get("candidates") if isinstance(data, dict) else data
        if isinstance(data, list):
            candidates = data
        if not isinstance(candidates, list):
            continue
        for cand in candidates:
            if not isinstance(cand, dict):
                continue
            source = str(cand.get("source") or (cand.get("metadata") or {}).get("source") or "unknown")
            counts[source] += 1
        if counts:
            return counts
    # Fallback: best_candidate alone
    best = result.get("best_candidate")
    if isinstance(best, dict):
        source = str(best.get("source") or "unknown")
        counts[source] += 1
    return counts


def non_seed_count(counts: Counter[str]) -> int:
    return sum(n for source, n in counts.items() if source != "seed")


def jesterky_receipts(result: dict[str, Any], run_dir: Path) -> list[dict[str, Any]]:
    receipts: list[dict[str, Any]] = []
    block = result.get("jesterky_workflow")
    if isinstance(block, dict):
        raw = block.get("receipts")
        if isinstance(raw, list):
            receipts.extend([r for r in raw if isinstance(r, dict)])
    path = run_dir / "jesterky_workflow_receipts.jsonl"
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
    # Prefer jsonl rows; drop duplicates by (generation, manifest_path).
    deduped: list[dict[str, Any]] = []
    seen: set[tuple[Any, Any]] = set()
    for row in receipts:
        key = (row.get("generation"), row.get("manifest_path") or row.get("trace_dir"))
        if key in seen:
            continue
        seen.add(key)
        deduped.append(row)
    return deduped


def heldout_means(result: dict[str, Any], run_dir: Path) -> dict[str, Any]:
    out: dict[str, Any] = {
        "baseline_holdout_mean": result.get("baseline_holdout_mean"),
        "best_holdout_mean": result.get("best_holdout_mean")
        if result.get("best_holdout_mean") is not None
        else result.get("best_heldout_mean"),
        "holdout_uplift": result.get("holdout_uplift"),
        "seed_holdout_mean": None,
        "best_candidate_id": None,
    }
    best = result.get("best_candidate")
    if isinstance(best, dict):
        out["best_candidate_id"] = best.get("candidate_id")
        if out["best_holdout_mean"] is None and isinstance(
            best.get("heldout_reward"), (int, float)
        ):
            out["best_holdout_mean"] = float(best["heldout_reward"])
        frames = best.get("sensor_frames") or []
        if out["best_holdout_mean"] is None and isinstance(frames, list):
            heldout_frames = [
                f
                for f in frames
                if isinstance(f, dict) and f.get("evaluation_stage") == "heldout"
            ]
            if heldout_frames:
                rewards = [
                    float(f["reward"])
                    for f in heldout_frames
                    if isinstance(f.get("reward"), (int, float))
                ]
                if rewards:
                    out["best_holdout_mean"] = sum(rewards) / len(rewards)
        scores = best.get("scores") or best.get("heldout_scores") or {}
        if isinstance(scores, dict) and out["best_holdout_mean"] is None:
            for key in ("mean_reward", "reward", "heldout_mean", "score"):
                if isinstance(scores.get(key), (int, float)):
                    out["best_holdout_mean"] = float(scores[key])
                    break
    # score_chart / frontier may carry seed vs best
    for name in ("score_chart.json", "gepa_summary.json", "frontier.json"):
        path = run_dir / name
        if not path.is_file():
            continue
        data = load_json(path)
        if not isinstance(data, dict):
            continue
        if out["best_holdout_mean"] is None and isinstance(data.get("best_heldout_mean"), (int, float)):
            out["best_holdout_mean"] = float(data["best_heldout_mean"])
        if out["baseline_holdout_mean"] is None and isinstance(
            data.get("seed_heldout_mean"), (int, float)
        ):
            out["baseline_holdout_mean"] = float(data["seed_heldout_mean"])
            out["seed_holdout_mean"] = out["baseline_holdout_mean"]
    # Seed baseline from candidate registry when present
    if out["baseline_holdout_mean"] is None:
        for name in ("candidate_registry.json", "candidates.json"):
            path = run_dir / name
            if not path.is_file():
                continue
            data = load_json(path)
            if isinstance(data, list):
                candidates = data
            elif isinstance(data, dict):
                candidates = data.get("candidates")
            else:
                continue
            if not isinstance(candidates, list):
                continue
            for cand in candidates:
                if not isinstance(cand, dict):
                    continue
                if str(cand.get("source") or "") != "seed":
                    continue
                if isinstance(cand.get("heldout_reward"), (int, float)):
                    out["baseline_holdout_mean"] = float(cand["heldout_reward"])
                    out["seed_holdout_mean"] = out["baseline_holdout_mean"]
                    break
            if out["baseline_holdout_mean"] is not None:
                break
    if (
        out["holdout_uplift"] is None
        and isinstance(out["best_holdout_mean"], (int, float))
        and isinstance(out["baseline_holdout_mean"], (int, float))
    ):
        out["holdout_uplift"] = float(out["best_holdout_mean"]) - float(out["baseline_holdout_mean"])
    return out


def score_arm(*, label: str, result_path: Path, expect_jesterky: bool) -> dict[str, Any]:
    result = load_json(result_path)
    run_dir = result_path.parent
    counts = candidate_source_counts(run_dir, result)
    receipts = jesterky_receipts(result, run_dir)
    enabled_receipts = [r for r in receipts if r.get("enabled")]
    heldout = heldout_means(result, run_dir)
    return {
        "label": label,
        "result_path": str(result_path),
        "candidate_count": int(result.get("candidate_count") or sum(counts.values()) or 0),
        "candidate_sources": dict(counts),
        "non_seed_candidates": non_seed_count(counts),
        "baseline_holdout_mean": heldout.get("baseline_holdout_mean"),
        "best_holdout_mean": heldout.get("best_holdout_mean"),
        "holdout_uplift": heldout.get("holdout_uplift"),
        "best_candidate_id": heldout.get("best_candidate_id"),
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

    hollow = [
        r
        for r in arm_b.get("jesterky_receipts") or []
        if isinstance(r, dict)
        and r.get("enabled")
        and int(r.get("theme_count") or 0) == 0
        and int(r.get("annotated") or 0) == 0
        and str(r.get("manifest_path") or "").strip()
    ]
    enabled = [
        r
        for r in (arm_b.get("jesterky_receipts") or [])
        if isinstance(r, dict) and r.get("enabled")
    ]
    if hollow and len(hollow) == len(enabled):
        failures.append(
            f"Arm B jesterky receipts are hollow (theme_count=0/annotated=0 on "
            f"{len(hollow)} receipts); do not attribute uplift to jesterky content"
        )
    elif hollow:
        failures.append(
            f"Arm B has {len(hollow)} hollow jesterky receipt(s) with empty theme/annotate signal"
        )

    a_best = arm_a.get("best_holdout_mean")
    b_best = arm_b.get("best_holdout_mean")
    beat = False
    if isinstance(b_best, (int, float)) and isinstance(a_best, (int, float)):
        beat = float(b_best) > float(a_best)
    else:
        failures.append("missing best_holdout_mean for comparison")
    if not beat and "missing best_holdout_mean" not in " ".join(failures):
        failures.append(
            f"Arm B did not beat Arm A on best_holdout_mean (A={a_best}; B={b_best})"
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
        "# GEPA ± jesterky workflows ablation",
        "",
        f"**Status:** {status}",
        "",
        "Claim shape: GEPA with jesterky trace annotate workflows vs without.",
        "Arms differ only in `jesterky_workflow.enabled`.",
        "",
        "Primary metric: **best_heldout_mean** (M5a). Existing n=64 base-vs-GEPA-prompt",
        "headline in `proof/gepa_craftax_ablation.md` is a separate claim.",
        "",
        "## Results",
        "",
        "| Arm | jesterky | non-seed | baseline heldout | best heldout | uplift | receipts |",
        "|-----|----------|---------:|-----------------:|-------------:|-------:|---------:|",
        (
            f"| A | off | {a['non_seed_candidates']} | "
            f"{a.get('baseline_holdout_mean')} | {a.get('best_holdout_mean')} | "
            f"{a.get('holdout_uplift')} | {a['jesterky_receipt_count']} |"
        ),
        (
            f"| B | on | {b['non_seed_candidates']} | "
            f"{b.get('baseline_holdout_mean')} | {b.get('best_holdout_mean')} | "
            f"{b.get('holdout_uplift')} | {b['jesterky_receipt_count']} |"
        ),
        "",
        f"- A sources: `{a['candidate_sources']}`",
        f"- B sources: `{b['candidate_sources']}`",
        f"- A best: `{a.get('best_candidate_id')}`",
        f"- B best: `{b.get('best_candidate_id')}`",
        "",
        "## Ship bar",
        "",
        f"- min non-seed candidates per arm: {report['min_non_seed']}",
        "- Arm B must have jesterky receipts with non-empty theme_count/annotated",
        "- Arm A must have zero jesterky receipts",
        "- Arm B best_heldout_mean must beat Arm A",
        "- Hollow annotate materialization is FAIL (not PASS)",
        "",
    ]
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
            f"- Arm A result: `{a.get('result_path')}`",
            f"- Arm B result: `{b.get('result_path')}`",
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


def parse_enabled(toml_text: str) -> bool | None:
    in_section = False
    for line in toml_text.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_section = stripped == "[jesterky_workflow]"
            continue
        if in_section and stripped.startswith("enabled"):
            _, _, rhs = stripped.partition("=")
            return rhs.strip().lower() in {"true", "1"}
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arm-a-result", type=Path)
    parser.add_argument("--arm-b-result", type=Path)
    parser.add_argument("--arm-a-config", type=Path, default=DEFAULT_ARM_A)
    parser.add_argument("--arm-b-config", type=Path, default=DEFAULT_ARM_B)
    parser.add_argument("--min-non-seed", type=int, default=1)
    parser.add_argument("--out-md", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--out-summary", type=Path, default=DEFAULT_SUMMARY)
    parser.add_argument("--allow-pending", action="store_true")
    parser.add_argument("--score-only", action="store_true")
    parser.add_argument("--validate-configs", action="store_true")
    args = parser.parse_args()

    if args.validate_configs or not args.score_only:
        a_en = parse_enabled(args.arm_a_config.read_text())
        b_en = parse_enabled(args.arm_b_config.read_text())
        if a_en is not False:
            print("Arm A config must have jesterky_workflow.enabled=false", file=sys.stderr)
            return 2
        if b_en is not True:
            print("Arm B config must have jesterky_workflow.enabled=true", file=sys.stderr)
            return 2
        print(
            json.dumps(
                {
                    "arm_a_enabled": False,
                    "arm_b_enabled": True,
                    "arm_a_config": str(args.arm_a_config),
                    "arm_b_config": str(args.arm_b_config),
                },
                indent=2,
            )
        )

    if not args.arm_a_result or not args.arm_b_result:
        if args.score_only:
            print("--score-only requires --arm-a-result and --arm-b-result", file=sys.stderr)
            return 2
        pending = {
            "pass": False,
            "failures": [
                "Arm A/B Craftax GEPA runs not scored yet; launch configs then re-run with --score-only"
            ],
            "arm_a": {
                "label": "A",
                "result_path": "",
                "non_seed_candidates": 0,
                "candidate_sources": {},
                "baseline_holdout_mean": None,
                "best_holdout_mean": None,
                "holdout_uplift": None,
                "jesterky_receipt_count": 0,
            },
            "arm_b": {
                "label": "B",
                "result_path": "",
                "non_seed_candidates": 0,
                "candidate_sources": {},
                "baseline_holdout_mean": None,
                "best_holdout_mean": None,
                "holdout_uplift": None,
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
