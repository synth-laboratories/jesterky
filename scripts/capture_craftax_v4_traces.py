#!/usr/bin/env python3
"""Capture Craftax ReAct container rollouts as synth_rollout_trace_v4 JSON files.

Runs sync rollouts against a live GameBench Craftax container (gemini-3.1-flash-lite
by default), normalizes turn artifacts into v4 traces, and writes one file per seed
under proof/craftax_v4_traces/.

Prereqs:
  - Rust gold on CRAFTAX_GOLD_URL (default http://127.0.0.1:8098)
  - Craftax ReAct container on CONTAINER_URL (default http://127.0.0.1:18104)
  - Provider credentials configured in the running ReAct container
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import uuid
from dataclasses import asdict, dataclass
from enum import Enum
from pathlib import Path

from http_contract import HttpMethod, request_json_object, wait_for_json_health
from json_contract import JsonObject, JsonObjectReader

DEFAULT_GOLD_URL = "http://127.0.0.1:8098"
DEFAULT_CONTAINER_URL = "http://127.0.0.1:18104"
DEFAULT_OUT_DIR = Path(__file__).resolve().parents[1] / "proof" / "craftax_v4_traces"
DEFAULT_SEEDS = [1, 2, 3, 4, 5, 6, 7, 8]


class HealthField(str, Enum):
    OK = "ok"
    HEALTHY = "healthy"


class RolloutStatus(str, Enum):
    COMPLETED = "completed"


class ArtifactType(str, Enum):
    TURNS = "turns"


def wait_for_health(
    label: str,
    url: str,
    contract: HealthField,
    timeout_s: float = 120.0,
) -> None:
    wait_for_json_health(
        label,
        url,
        lambda payload: payload.boolean(contract.value),
        timeout_s,
    )


@dataclass(frozen=True)
class CraftaxTurn:
    batch_index: int
    action: str
    ply: int
    reward_total: float
    achievements: tuple[str, ...]
    invalid_parse: bool
    repaired: bool
    batch_size: int


@dataclass(frozen=True)
class CraftaxPrimaryTurn(CraftaxTurn):
    assistant_text: str
    model: str
    usage: TokenUsage
    request_id: str


@dataclass(frozen=True)
class CraftaxContinuationTurn(CraftaxTurn):
    pass


@dataclass(frozen=True)
class TokenUsage:
    completion_tokens: int
    prompt_tokens: int
    total_tokens: int

    @classmethod
    def parse(cls, raw: JsonObjectReader) -> "TokenUsage":
        return cls(
            completion_tokens=raw.integer("completion_tokens"),
            prompt_tokens=raw.integer("prompt_tokens"),
            total_tokens=raw.integer("total_tokens"),
        )


class MessageRole(str, Enum):
    SYSTEM = "system"
    USER = "user"
    ASSISTANT = "assistant"


@dataclass(frozen=True)
class TraceMessage:
    role: MessageRole
    content: str


@dataclass(frozen=True)
class TraceRequest:
    messages: tuple[TraceMessage, ...]
    provider_hint: str


@dataclass(frozen=True)
class LlmRequest:
    messages: tuple[TraceMessage, ...]
    model: str


@dataclass(frozen=True)
class TraceResponse:
    message: TraceMessage
    usage: TokenUsage


def parse_craftax_turn(raw: JsonObjectReader) -> CraftaxTurn:
    batch_index = raw.integer("batch_index")
    action = raw.string("action")
    ply = raw.integer("ply")
    reward_total = raw.number("reward_total")
    achievements = raw.strings("achievements")
    invalid_parse = raw.boolean("invalid_parse")
    repaired = raw.boolean("repaired")
    batch_size = raw.integer("batch_size")
    if batch_index == 0:
        return CraftaxPrimaryTurn(
            batch_index=batch_index,
            action=action,
            ply=ply,
            reward_total=reward_total,
            achievements=achievements,
            invalid_parse=invalid_parse,
            repaired=repaired,
            batch_size=batch_size,
            assistant_text=raw.string("assistant_text", allow_empty=True),
            model=raw.string("model"),
            usage=TokenUsage.parse(raw.object("usage")),
            request_id=raw.string("request_id"),
        )
    raw.null("model")
    raw.null("request_id")
    raw.string("assistant_text", allow_empty=True)
    raw.object("usage")
    return CraftaxContinuationTurn(
        batch_index=batch_index,
        action=action,
        ply=ply,
        reward_total=reward_total,
        achievements=achievements,
        invalid_parse=invalid_parse,
        repaired=repaired,
        batch_size=batch_size,
    )


@dataclass(frozen=True)
class RolloutMetadata:
    raw: JsonObject


@dataclass(frozen=True)
class PolicyIdentity:
    provider: str
    model: str


@dataclass(frozen=True)
class CraftaxRolloutRecord:
    raw: JsonObject
    rollout_id: str
    trace_correlation_id: str
    status: RolloutStatus
    outcome_reward: float
    achievements: tuple[str, ...]
    metadata: RolloutMetadata
    turns: tuple[CraftaxTurn, ...]

    @classmethod
    def parse(cls, raw: JsonObjectReader) -> "CraftaxRolloutRecord":
        rollout_id = raw.string("rollout_id")
        trace_correlation_id = raw.string("trace_correlation_id")
        status = raw.enum("status", RolloutStatus)
        reward_info = raw.object("reward_info")
        details = reward_info.object("details")
        metadata_reader = raw.object("metadata")
        metadata = RolloutMetadata(raw=metadata_reader.data)
        turns = turns_from_record(raw)
        return cls(
            raw=raw.data,
            rollout_id=rollout_id,
            trace_correlation_id=trace_correlation_id,
            status=status,
            outcome_reward=reward_info.number("outcome_reward"),
            achievements=details.strings("achievements"),
            metadata=metadata,
            turns=turns,
        )


def turns_from_record(record: JsonObjectReader) -> tuple[CraftaxTurn, ...]:
    turns_artifacts = tuple(
        artifact
        for artifact in record.objects("artifacts")
        if artifact.enum("artifact_type", ArtifactType) is ArtifactType.TURNS
    )
    if len(turns_artifacts) != 1:
        raise JsonContractError(
            "rollout.artifacts must contain exactly one turns artifact; "
            f"found {len(turns_artifacts)}"
        )
    return tuple(
        parse_craftax_turn(turn) for turn in turns_artifacts[0].objects("turns")
    )


def build_v4_trace(
    record: CraftaxRolloutRecord,
    *,
    seed: int,
    max_steps: int,
    policy: PolicyIdentity,
) -> JsonObject:
    spans: list[JsonObject] = []
    events: list[JsonObject] = []
    llm_call_index = 0
    for turn in record.turns:
        match turn:
            case CraftaxContinuationTurn():
                continue
            case CraftaxPrimaryTurn():
                pass
        llm_call_index += 1
        span_id = f"{record.rollout_id}-span-{llm_call_index:04d}"
        user_text = (
            f"Craftax turn {turn.ply}; "
            f"reward_total={turn.reward_total}; "
            f"achievements={list(turn.achievements)}"
        )
        messages = (
            TraceMessage(MessageRole.SYSTEM, "Craftax ReAct policy"),
            TraceMessage(MessageRole.USER, user_text),
        )
        request = TraceRequest(
            messages=messages,
            provider_hint="openai_compat",
        )
        llm_request = LlmRequest(messages=messages, model=policy.model)
        response = TraceResponse(
            message=TraceMessage(MessageRole.ASSISTANT, turn.assistant_text),
            usage=turn.usage,
        )
        span = {
            "span_id": span_id,
            "call_index": llm_call_index,
            "run_id": record.rollout_id,
            "request": asdict(request),
            "response": asdict(response),
            "metrics": {
                "env_action": turn.action,
                "invalid_parse": turn.invalid_parse,
                "repaired": turn.repaired,
                "reward_total": turn.reward_total,
            },
            "metadata": {
                "batch_size": turn.batch_size,
                "request_id": turn.request_id,
            },
        }
        spans.append(span)
        events.append(
            {
                "type": "lm_call",
                "sequence_index": llm_call_index,
                "span_id": span_id,
                "llm_request": asdict(llm_request),
                "llm_response": asdict(response),
                "metadata": {"env_action": turn.action},
            }
        )
    return {
        "schema_version": "synth_rollout_trace_v4",
        "trace_schema_version": 4,
        "rollout_id": record.rollout_id,
        "trace_correlation_id": record.trace_correlation_id,
        "status": record.status,
        "spans": spans,
        "events": events,
        "span_count": len(spans),
        "summary": {
            "seed": seed,
            "max_steps": max_steps,
            "outcome_reward": record.outcome_reward,
            "reward": record.outcome_reward,
            "achievements": record.achievements,
            "llm_turns": llm_call_index,
            "invalid_parse_turns": sum(1 for t in record.turns if t.invalid_parse),
            "policy_provider": policy.provider,
            "policy_model": policy.model,
        },
        "metadata": {
            **record.metadata.raw,
            "source": "capture_craftax_v4_traces.py",
            "env": "gamebench.craftax-singleplayer.rust_gold",
        },
    }


def rollout_payload(
    *, seed: int, max_steps: int, max_llm_turns: int, policy: PolicyIdentity
) -> JsonObject:
    rollout_id = f"craftax-v4-seed-{seed}-{uuid.uuid4().hex[:8]}"
    return {
        "rollout_id": rollout_id,
        "trace_correlation_id": rollout_id,
        "env": {
            "env_name": "craftax-singleplayer",
            "seed": seed,
            "config": {"seed": seed, "max_steps": max_steps},
        },
        "policy": {
            "policy_id": "craftax_react_gemini_v1",
            "config": {
                "provider": policy.provider,
                "model": policy.model,
                "use_lm": True,
                "max_tokens": 512,
                "max_llm_turns": max_llm_turns,
                "min_actions_per_call": 5,
                "max_actions_per_call": 15,
            },
        },
        "metadata": {
            "capture_lane": "jesterky_v4_corpus",
            "seed": seed,
            "model": policy.model,
            "provider": policy.provider,
        },
    }


@dataclass(frozen=True)
class CaptureResult:
    path: Path
    rollout_id: str
    reward: float
    span_count: int


def capture_one(
    *,
    container_url: str,
    seed: int,
    max_steps: int,
    max_llm_turns: int,
    model: str,
    out_dir: Path,
) -> CaptureResult:
    policy = PolicyIdentity(provider="gemini", model=model)
    payload = rollout_payload(
        seed=seed, max_steps=max_steps, max_llm_turns=max_llm_turns, policy=policy
    )
    raw_record = request_json_object(
        HttpMethod.POST,
        f"{container_url.rstrip('/')}/rollout",
        payload,
        timeout_s=600.0,
    )
    record = CraftaxRolloutRecord.parse(raw_record)
    trace = build_v4_trace(record, seed=seed, max_steps=max_steps, policy=policy)
    out_path = out_dir / f"seed-{seed}.v4.json"
    out_path.write_text(json.dumps(trace, indent=2, sort_keys=True) + "\n")
    manifest_path = out_dir / f"seed-{seed}.rollout.json"
    manifest_path.write_text(json.dumps(record.raw, indent=2, sort_keys=True) + "\n")
    span_count = sum(1 for turn in record.turns if turn.batch_index == 0)
    print(
        f"captured seed={seed} reward={record.outcome_reward} "
        f"spans={span_count} -> {out_path}"
    )
    return CaptureResult(
        path=out_path,
        rollout_id=record.rollout_id,
        reward=record.outcome_reward,
        span_count=span_count,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--gold-url", default=os.environ.get("CRAFTAX_GOLD_URL", DEFAULT_GOLD_URL)
    )
    parser.add_argument(
        "--container-url",
        default=os.environ.get("CONTAINER_URL", DEFAULT_CONTAINER_URL),
    )
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    parser.add_argument("--seeds", type=int, nargs="+", default=DEFAULT_SEEDS)
    parser.add_argument("--max-steps", type=int, default=20)
    parser.add_argument("--max-llm-turns", type=int, default=4)
    parser.add_argument("--model", default="gemini-3.1-flash-lite")
    parser.add_argument("--skip-health", action="store_true")
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)

    if not args.skip_health:
        wait_for_health("craftax_gold", args.gold_url, HealthField.OK)
        wait_for_health("craftax_container", args.container_url, HealthField.HEALTHY)

    index: list[JsonObject] = []
    for seed in args.seeds:
        result = capture_one(
            container_url=args.container_url,
            seed=seed,
            max_steps=args.max_steps,
            max_llm_turns=args.max_llm_turns,
            model=args.model,
            out_dir=args.out_dir,
        )
        index.append(
            {
                "seed": seed,
                "path": str(result.path),
                "rollout_id": result.rollout_id,
                "reward": result.reward,
                "span_count": result.span_count,
            }
        )

    index_path = args.out_dir / "index.json"
    index_path.write_text(
        json.dumps({"traces": index}, indent=2, sort_keys=True) + "\n"
    )
    print(f"wrote index -> {index_path} ({len(index)} traces)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
