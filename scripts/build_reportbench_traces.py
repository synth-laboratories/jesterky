#!/usr/bin/env python3
"""Build v4 traces from real SMR ReportBench lane artifacts.

Scans an evals ReportBench lanes directory and, for each lane whose
artifacts/ contains a parseable autograde_latest.json, emits
<out-dir>/<lane>.v4.json shaped for the jesterky `trace.expand` op
(top-level *.v4.json files; summary carries reward/outcome_reward).
The `evidence` object embeds the outcome facts an evaluator grades:
autograde counts, the benchmark verdict, verifier verdicts, and which
artifact files are present. Source lanes are read-only.
"""

import argparse
import json
import sys
from pathlib import Path

DEFAULT_LANES_DIR = Path(__file__).resolve().parents[2] / "evals" / "reportbench" / "lanes"
DEFAULT_OUT_DIR = "proof/reportbench_traces"

COMPANIONS = ["reportbench_output.json", "verifier_review.json", "summary_latest.json"]


def load_json(path: Path):
    try:
        with open(path) as fh:
            return json.load(fh)
    except (OSError, json.JSONDecodeError):
        return None


def build_trace(lane: str, artifacts: Path):
    autograde = load_json(artifacts / "autograde_latest.json")
    if not isinstance(autograde, dict) or "reward" not in autograde:
        return None
    grade = autograde.get("grade") if isinstance(autograde.get("grade"), dict) else {}
    reward = autograde.get("reward")
    state = autograde.get("state")

    reportbench_output = load_json(artifacts / "reportbench_output.json")
    verifier_review = load_json(artifacts / "verifier_review.json")

    benchmark_verdict = None
    if isinstance(reportbench_output, dict):
        benchmark_verdict = reportbench_output.get("benchmark_verdict")

    verifier = None
    if isinstance(verifier_review, dict):
        verifier = {
            "score": verifier_review.get("score"),
            "summary": verifier_review.get("summary"),
            "criteria": [
                {"id": c.get("id"), "score": c.get("score"), "reason": c.get("reason")}
                for c in (verifier_review.get("run_verifier") or {}).get("criteria", [])
                if isinstance(c, dict)
            ],
        }

    artifacts_present = sorted(
        name for name in COMPANIONS + ["autograde_latest.json"] if (artifacts / name).is_file()
    )

    return {
        "schema_version": "synth_rollout_trace_v4",
        "rollout_id": f"reportbench-{lane}",
        "status": "completed" if state in ("done", "succeeded") else str(state),
        "summary": {
            "reward": reward,
            "outcome_reward": reward,
            "run_id": autograde.get("run_id"),
            "state": state,
        },
        "metadata": {
            "source": "evals/reportbench",
            "lane": lane,
        },
        "evidence": {
            "autograde": {
                "reward": reward,
                "checks_passed": grade.get("checks_passed"),
                "checks_total": grade.get("checks_total"),
                "checks_failed": grade.get("checks_failed"),
                "fatal_errors": grade.get("fatal_errors", []),
                "state": state,
            },
            "benchmark_verdict": benchmark_verdict,
            "verifier": verifier,
            "artifacts_present": artifacts_present,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lanes-dir", type=Path, default=DEFAULT_LANES_DIR)
    parser.add_argument("--out-dir", default=DEFAULT_OUT_DIR)
    args = parser.parse_args()

    lanes_dir = args.lanes_dir
    if not lanes_dir.is_dir():
        print(f"lanes dir not found: {lanes_dir}", file=sys.stderr)
        return 1
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    built, skipped = 0, 0
    for lane_dir in sorted(p for p in lanes_dir.iterdir() if p.is_dir()):
        artifacts = lane_dir / "artifacts"
        if not (artifacts / "autograde_latest.json").is_file():
            continue
        trace = build_trace(lane_dir.name, artifacts)
        if trace is None:
            skipped += 1
            continue
        out_path = out_dir / f"{lane_dir.name}.v4.json"
        with open(out_path, "w") as fh:
            json.dump(trace, fh, indent=2)
            fh.write("\n")
        built += 1

    print(f"skipped {skipped} lanes without parseable autograde_latest.json")
    print(f"built {built} traces -> {out_dir}")
    return 0 if built else 1


if __name__ == "__main__":
    sys.exit(main())
