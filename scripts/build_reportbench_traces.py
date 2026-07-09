#!/usr/bin/env python3
"""Build v4 traces from real SMR ReportBench lane artifacts.

Scans an evals ReportBench lanes directory and, for each lane whose
artifacts/ contains an autograde_latest.json, validates the producer contract, and emits
<out-dir>/<lane>.v4.json shaped for the jesterky `trace.expand` op
(top-level *.v4.json files; summary carries reward/outcome_reward).
The `evidence` object embeds the outcome facts an evaluator grades:
autograde counts, the benchmark verdict, verifier verdicts, and which
artifact files are present. Source lanes are read-only.
"""

import argparse
import json
import sys
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path

from json_contract import JsonValue, read_json_object

DEFAULT_LANES_DIR = (
    Path(__file__).resolve().parents[2] / "evals" / "reportbench" / "lanes"
)
DEFAULT_OUT_DIR = "proof/reportbench_traces"

REQUIRED_ARTIFACTS = (
    "autograde_latest.json",
    "reportbench_output.json",
    "summary_latest.json",
    "verifier_review.json",
)


class RunState(str, Enum):
    DONE = "done"
    SUCCEEDED = "succeeded"
    COMPLETED = "completed"
    FAILED = "failed"
    ERROR = "error"
    CANCELLED = "cancelled"

    @property
    def trace_status(self) -> "TraceStatus":
        if self in {RunState.DONE, RunState.SUCCEEDED, RunState.COMPLETED}:
            return TraceStatus.COMPLETED
        return TraceStatus(self.value)


class TraceStatus(str, Enum):
    COMPLETED = "completed"
    FAILED = "failed"
    ERROR = "error"
    CANCELLED = "cancelled"


@dataclass(frozen=True)
class AutogradeGrade:
    checks_passed: int
    checks_total: int
    checks_failed: int
    fatal_errors: tuple[str, ...]


@dataclass(frozen=True)
class Autograde:
    reward: float
    state: RunState
    run_id: str
    grade: AutogradeGrade


@dataclass(frozen=True)
class VerifierCriterion:
    id: str
    score: float
    reason: str


@dataclass(frozen=True)
class VerifierEvidence:
    score: float
    summary: str
    criteria: tuple[VerifierCriterion, ...]


@dataclass(frozen=True)
class ReportBenchSummary:
    reward: float
    outcome_reward: float
    run_id: str
    state: RunState


@dataclass(frozen=True)
class ReportBenchMetadata:
    source: str
    lane: str


@dataclass(frozen=True)
class AutogradeEvidence:
    reward: float
    checks_passed: int
    checks_total: int
    checks_failed: int
    fatal_errors: tuple[str, ...]
    state: RunState


@dataclass(frozen=True)
class TraceEvidence:
    autograde: AutogradeEvidence
    benchmark_verdict: JsonValue
    verifier: VerifierEvidence
    artifacts_present: tuple[str, ...]


@dataclass(frozen=True)
class ReportBenchTrace:
    schema_version: str
    rollout_id: str
    status: TraceStatus
    summary: ReportBenchSummary
    metadata: ReportBenchMetadata
    evidence: TraceEvidence


def parse_autograde(path: Path) -> Autograde:
    raw = read_json_object(path)
    grade = raw.object("grade")
    state = raw.enum("state", RunState)
    return Autograde(
        reward=raw.number("reward"),
        state=state,
        run_id=raw.string("run_id"),
        grade=AutogradeGrade(
            checks_passed=grade.integer("checks_passed"),
            checks_total=grade.integer("checks_total"),
            checks_failed=grade.integer("checks_failed"),
            fatal_errors=grade.strings("fatal_errors"),
        ),
    )


def parse_verifier(path: Path) -> VerifierEvidence:
    raw = read_json_object(path)
    criteria = tuple(
        VerifierCriterion(
            id=criterion.string("id"),
            score=criterion.number("score"),
            reason=criterion.string("reason"),
        )
        for criterion in raw.object("run_verifier").objects("criteria")
    )
    return VerifierEvidence(
        score=raw.number("score"),
        summary=raw.string("summary"),
        criteria=criteria,
    )


def build_trace(lane: str, artifacts: Path) -> ReportBenchTrace:
    autograde = parse_autograde(artifacts / "autograde_latest.json")
    reportbench_output = read_json_object(artifacts / "reportbench_output.json")
    read_json_object(artifacts / "summary_latest.json")
    benchmark_verdict: JsonValue = reportbench_output.value("benchmark_verdict")
    verifier = parse_verifier(artifacts / "verifier_review.json")

    return ReportBenchTrace(
        schema_version="synth_rollout_trace_v4",
        rollout_id=f"reportbench-{lane}",
        status=autograde.state.trace_status,
        summary=ReportBenchSummary(
            reward=autograde.reward,
            outcome_reward=autograde.reward,
            run_id=autograde.run_id,
            state=autograde.state,
        ),
        metadata=ReportBenchMetadata(source="evals/reportbench", lane=lane),
        evidence=TraceEvidence(
            autograde=AutogradeEvidence(
                reward=autograde.reward,
                checks_passed=autograde.grade.checks_passed,
                checks_total=autograde.grade.checks_total,
                checks_failed=autograde.grade.checks_failed,
                fatal_errors=autograde.grade.fatal_errors,
                state=autograde.state,
            ),
            benchmark_verdict=benchmark_verdict,
            verifier=verifier,
            artifacts_present=REQUIRED_ARTIFACTS,
        ),
    )


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

    built = 0
    for lane_dir in sorted(p for p in lanes_dir.iterdir() if p.is_dir()):
        artifacts = lane_dir / "artifacts"
        if not (artifacts / "autograde_latest.json").is_file():
            continue
        trace = build_trace(lane_dir.name, artifacts)
        out_path = out_dir / f"{lane_dir.name}.v4.json"
        with open(out_path, "w") as fh:
            json.dump(asdict(trace), fh, indent=2)
            fh.write("\n")
        built += 1

    print(f"built {built} traces -> {out_dir}")
    return 0 if built else 1


if __name__ == "__main__":
    sys.exit(main())
