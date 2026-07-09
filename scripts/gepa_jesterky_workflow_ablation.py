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
from dataclasses import dataclass
from pathlib import Path

from json_contract import (
    JsonContractError,
    JsonObject,
    JsonObjectReader,
    JsonValue,
    read_json_array_objects,
    read_json_lines,
    read_json_object,
)

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARM_A = ROOT / "proof" / "gepa_jesterky_workflow_arm_a.toml"
DEFAULT_ARM_B = ROOT / "proof" / "gepa_jesterky_workflow_arm_b.toml"
DEFAULT_OUT = ROOT / "proof" / "gepa_jesterky_workflow_ablation.md"
DEFAULT_SUMMARY = ROOT / "proof" / "gepa_jesterky_workflow_ablation" / "ablation_summary.json"


@dataclass(frozen=True)
class Candidate:
    candidate_id: str
    source: str
    heldout_reward: float | None

    @classmethod
    def parse(cls, raw: JsonObjectReader) -> "Candidate":
        return cls(
            candidate_id=raw.string("candidate_id"),
            source=raw.string("source"),
            heldout_reward=raw.nullable_number("heldout_reward"),
        )


@dataclass(frozen=True)
class WorkflowReceipt:
    generation: int
    enabled: bool
    theme_count: int
    annotated: int
    manifest_path: str

    @classmethod
    def parse(cls, raw: JsonObjectReader) -> "WorkflowReceipt":
        return cls(
            generation=raw.integer("generation"),
            enabled=raw.boolean("enabled"),
            theme_count=raw.integer("theme_count"),
            annotated=raw.integer("annotated"),
            manifest_path=raw.string("manifest_path"),
        )

    def as_json(self) -> JsonObject:
        return {
            "generation": self.generation,
            "enabled": self.enabled,
            "theme_count": self.theme_count,
            "annotated": self.annotated,
            "manifest_path": self.manifest_path,
        }


@dataclass(frozen=True)
class HeldoutMeans:
    baseline: float
    best: float
    best_candidate_id: str

    @property
    def uplift(self) -> float:
        return self.best - self.baseline


def candidate_registry(run_dir: Path) -> tuple[Candidate, ...]:
    path = run_dir / "candidate_registry.json"
    candidates = tuple(Candidate.parse(raw) for raw in read_json_array_objects(path))
    if not candidates:
        raise ValueError(f"{path} must contain at least one candidate")
    return candidates


def candidate_source_counts(candidates: tuple[Candidate, ...]) -> Counter[str]:
    return Counter(candidate.source for candidate in candidates)


def non_seed_count(counts: Counter[str]) -> int:
    return sum(n for source, n in counts.items() if source != "seed")


def jesterky_receipts(
    run_dir: Path, *, required: bool
) -> tuple[WorkflowReceipt, ...]:
    path = run_dir / "jesterky_workflow_receipts.jsonl"
    if not path.exists():
        if required:
            raise FileNotFoundError(f"enabled arm missing required receipt artifact: {path}")
        return ()
    receipts = tuple(WorkflowReceipt.parse(raw) for raw in read_json_lines(path))
    deduped: list[WorkflowReceipt] = []
    seen: set[tuple[int, str]] = set()
    for receipt in receipts:
        key = (receipt.generation, receipt.manifest_path)
        if key in seen:
            continue
        seen.add(key)
        deduped.append(receipt)
    return tuple(deduped)


def heldout_means(
    result: JsonObjectReader, candidates: tuple[Candidate, ...]
) -> HeldoutMeans:
    best_candidate_id = result.object("best_candidate").string("candidate_id")
    seeds = [candidate for candidate in candidates if candidate.source == "seed"]
    if len(seeds) != 1 or seeds[0].heldout_reward is None:
        raise JsonContractError(
            "candidate registry must contain one scored seed candidate"
        )
    best = next(
        (candidate for candidate in candidates if candidate.candidate_id == best_candidate_id),
        None,
    )
    if best is None or best.heldout_reward is None:
        raise JsonContractError(
            f"best candidate {best_candidate_id!r} is absent or unscored"
        )
    return HeldoutMeans(seeds[0].heldout_reward, best.heldout_reward, best_candidate_id)


def score_arm(*, label: str, result_path: Path, expect_jesterky: bool) -> JsonObject:
    result = read_json_object(result_path)
    run_dir = result_path.parent
    candidates = candidate_registry(run_dir)
    counts = candidate_source_counts(candidates)
    receipts = jesterky_receipts(run_dir, required=expect_jesterky)
    enabled_receipts = tuple(receipt for receipt in receipts if receipt.enabled)
    heldout = heldout_means(result, candidates)
    configured_enabled = result.object("jesterky_workflow").boolean("enabled")
    if configured_enabled is not expect_jesterky:
        raise ValueError(
            f"{result_path} jesterky_workflow.enabled={configured_enabled} "
            f"does not match expected arm value {expect_jesterky}"
        )
    return {
        "label": label,
        "result_path": str(result_path),
        "candidate_count": len(candidates),
        "candidate_sources": dict(counts),
        "non_seed_candidates": non_seed_count(counts),
        "baseline_holdout_mean": heldout.baseline,
        "best_holdout_mean": heldout.best,
        "holdout_uplift": heldout.uplift,
        "best_candidate_id": heldout.best_candidate_id,
        "jesterky_enabled_config": configured_enabled,
        "jesterky_receipt_count": len(enabled_receipts),
        "jesterky_receipts": [receipt.as_json() for receipt in enabled_receipts],
        "expect_jesterky": expect_jesterky,
    }


def evaluate(arm_a: JsonObject, arm_b: JsonObject, *, min_non_seed: int) -> JsonObject:
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


def write_markdown(path: Path, report: JsonObject) -> None:
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
