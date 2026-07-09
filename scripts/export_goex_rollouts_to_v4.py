#!/usr/bin/env python3
"""Export GoEx rollout evidence JSON into synth_rollout_trace_v4 files.

Used by the GELO ± jesterky workflows ablation and as a standalone helper when
debugging the goex → jesterky annotate path outside the Rust hook.

Example:
  python3 scripts/export_goex_rollouts_to_v4.py \\
    --evidence /path/to/artifacts/goex_rollout_evidence.json \\
    --out /tmp/jesterky_traces
"""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path
from typing import cast

from json_contract import JsonObject, JsonObjectReader, read_json_object, safe_filename_component


class EvidenceLane(str, Enum):
    SEARCH = "search"
    HELDOUT_MEASUREMENT = "heldout_measurement"


class GoExStatus(str, Enum):
    COMPLETED = "completed"
    SUCCEEDED = "succeeded"
    FAILED = "failed"


class MessageRole(str, Enum):
    SYSTEM = "system"
    USER = "user"
    ASSISTANT = "assistant"


@dataclass(frozen=True)
class GoExCheckpoint:
    checkpoint_id: str
    call_index: int
    reward: float
    objective_labels: tuple[str, ...]

    @classmethod
    def parse(cls, raw: JsonObjectReader) -> "GoExCheckpoint":
        return cls(
            checkpoint_id=raw.string("checkpoint_id"),
            call_index=raw.integer("policy_llm_call_index"),
            reward=raw.number("reward"),
            objective_labels=raw.strings("objective_labels"),
        )


@dataclass(frozen=True)
class GoExRecord:
    seed: int
    rollout_id: str
    status: GoExStatus
    reward: float
    evidence_id: str
    split: str
    candidate_id: str
    dispatch_kind: str
    theme_id: str
    trace_ref: str
    checkpoints: tuple[GoExCheckpoint, ...]

    @classmethod
    def parse(cls, raw: JsonObjectReader) -> "GoExRecord":
        checkpoints = tuple(
            GoExCheckpoint.parse(checkpoint)
            for checkpoint in raw.objects("mid_rollout_checkpoints")
        )
        if not checkpoints:
            raise ValueError(f"{raw.context}.mid_rollout_checkpoints must not be empty")
        return cls(
            seed=raw.integer("seed"),
            rollout_id=raw.string("rollout_id"),
            status=raw.enum("status", GoExStatus),
            reward=raw.number("reward"),
            evidence_id=raw.string("evidence_id"),
            split=raw.string("split"),
            candidate_id=raw.string("candidate_id"),
            dispatch_kind=raw.string("dispatch_kind"),
            theme_id=raw.string("theme_id"),
            trace_ref=raw.string("trace_ref"),
            checkpoints=checkpoints,
        )


@dataclass(frozen=True)
class TraceMessage:
    role: MessageRole
    content: str


@dataclass(frozen=True)
class TraceRequest:
    messages: tuple[TraceMessage, ...]
    provider_hint: str


@dataclass(frozen=True)
class TraceResponse:
    message: TraceMessage
    usage: TraceUsage


@dataclass(frozen=True)
class TraceUsage:
    """No token telemetry is present in committed GoEx checkpoint evidence."""


@dataclass(frozen=True)
class TraceMetrics:
    reward_total: float
    objective_labels: tuple[str, ...]


@dataclass(frozen=True)
class TraceSpanMetadata:
    checkpoint_id: str
    evidence_id: str


@dataclass(frozen=True)
class TraceSpan:
    span_id: str
    call_index: int
    run_id: str
    request: TraceRequest
    response: TraceResponse
    metrics: TraceMetrics
    metadata: TraceSpanMetadata


@dataclass(frozen=True)
class TraceEventMetadata:
    evidence_id: str


@dataclass(frozen=True)
class TraceEvent:
    type: str
    sequence_index: int
    span_id: str
    metadata: TraceEventMetadata


@dataclass(frozen=True)
class GoExTraceSummary:
    seed: int
    outcome_reward: float
    reward: float
    achievements: tuple[str, ...]
    llm_turns: int
    split: str
    candidate_id: str


@dataclass(frozen=True)
class GoExTraceMetadata:
    source: str
    evidence_id: str
    candidate_id: str
    dispatch_kind: str
    theme_id: str
    trace_ref: str


@dataclass(frozen=True)
class TraceEnvelope:
    schema_version: str
    trace_schema_version: int
    rollout_id: str
    trace_correlation_id: str
    status: GoExStatus
    spans: tuple[TraceSpan, ...]
    events: tuple[TraceEvent, ...]
    span_count: int
    summary: GoExTraceSummary
    metadata: GoExTraceMetadata


def record_to_v4(record: GoExRecord) -> JsonObject:
    spans: list[TraceSpan] = []
    events: list[TraceEvent] = []
    for checkpoint in record.checkpoints:
        call_index = checkpoint.call_index
        span_id = f"{record.rollout_id}-span-{call_index:04d}"
        spans.append(
            TraceSpan(
                span_id=span_id,
                call_index=call_index,
                run_id=record.rollout_id,
                request=TraceRequest(
                    messages=(
                        TraceMessage(
                            role=MessageRole.SYSTEM,
                            content="GoEx Craftax search evidence",
                        ),
                        TraceMessage(
                            role=MessageRole.USER,
                            content=f"seed={record.seed}; checkpoint={call_index}",
                        ),
                    ),
                    provider_hint="openai_compat",
                ),
                response=TraceResponse(
                    message=TraceMessage(
                        role=MessageRole.ASSISTANT,
                        content=(
                            f"checkpoint={checkpoint.checkpoint_id}; "
                            f"reward={checkpoint.reward}; "
                            f"objectives={list(checkpoint.objective_labels)}"
                        ),
                    ),
                    usage=TraceUsage(),
                ),
                metrics=TraceMetrics(
                    reward_total=checkpoint.reward,
                    objective_labels=checkpoint.objective_labels,
                ),
                metadata=TraceSpanMetadata(
                    checkpoint_id=checkpoint.checkpoint_id,
                    evidence_id=record.evidence_id,
                ),
            )
        )
        events.append(
            TraceEvent(
                type="lm_call",
                sequence_index=call_index,
                span_id=span_id,
                metadata=TraceEventMetadata(evidence_id=record.evidence_id),
            )
        )
    envelope = TraceEnvelope(
        schema_version="synth_rollout_trace_v4",
        trace_schema_version=4,
        rollout_id=record.rollout_id,
        trace_correlation_id=record.evidence_id,
        status=record.status,
        spans=tuple(spans),
        events=tuple(events),
        span_count=len(events),
        summary=GoExTraceSummary(
            seed=record.seed,
            outcome_reward=record.reward,
            reward=record.reward,
            achievements=(),
            llm_turns=len(events),
            split=record.split,
            candidate_id=record.candidate_id,
        ),
        metadata=GoExTraceMetadata(
            source="export_goex_rollouts_to_v4.py",
            evidence_id=record.evidence_id,
            candidate_id=record.candidate_id,
            dispatch_kind=record.dispatch_kind,
            theme_id=record.theme_id,
            trace_ref=record.trace_ref,
        ),
    )
    return cast(JsonObject, asdict(envelope))


def export_evidence(evidence_path: Path, out_dir: Path, *, lane: EvidenceLane) -> int:
    records = read_json_object(evidence_path).objects(lane.value)
    out_dir.mkdir(parents=True, exist_ok=True)
    written = 0
    for raw_record in records:
        record = GoExRecord.parse(raw_record)
        trace = record_to_v4(record)
        filename = safe_filename_component(record.evidence_id, "evidence_id")
        path = out_dir / f"{filename}.v4.json"
        path.write_text(json.dumps(trace, indent=2, sort_keys=True) + "\n")
        written += 1
    return written


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument(
        "--lane",
        type=EvidenceLane,
        choices=tuple(EvidenceLane),
        default=EvidenceLane.SEARCH,
        help="Which evidence lane to export (default: search).",
    )
    args = parser.parse_args()
    n = export_evidence(args.evidence, args.out, lane=args.lane)
    print(json.dumps({"written": n, "out": str(args.out), "lane": args.lane.value}))


if __name__ == "__main__":
    main()
