#!/usr/bin/env python3
"""Register one Jesterky child through Containers' canonical trace contract."""

from __future__ import annotations

import argparse
import hashlib
from importlib.metadata import version
import json
import os
from pathlib import Path

from synth_containers.tracing import (
    ActorKind,
    ActorV5,
    SessionV5,
    TraceContextV1,
    record_id,
    utc_now,
)
from synth_containers.tracing.adapters import import_codex_jsonl
from synth_containers.tracing.capture.emitter import TraceEmitter
from synth_containers.tracing.models.actors import SessionCoverageV5

EXPECTED_CONTAINERS_VERSION = "0.4.0.20260725"
EXPECTED_CONTAINERS_SOURCE_COMMIT = "0456dc5ea4900e54714acc26867345d84a07b9ff"
EXPECTED_CONTAINERS_WHEEL_SHA256 = (
    "9d40f21be7ea21a72aec8e316b4de4524e20cee54cc641bfdb2439413daa22b0"
)


def _verify_containers_runtime() -> None:
    installed = version("synth-containers")
    if installed != EXPECTED_CONTAINERS_VERSION:
        raise RuntimeError(
            "Jesterky Trace V5 requires synth-containers "
            f"{EXPECTED_CONTAINERS_VERSION}; found {installed}"
        )
    wheel_value = str(
        os.environ.get("SYNTH_TRACE_CONTAINERS_WHEEL_PATH")
        or os.environ.get("SYNTH_CONTAINERS_WHEEL")
        or ""
    ).strip()
    if not wheel_value:
        raise RuntimeError(
            "SYNTH_TRACE_CONTAINERS_WHEEL_PATH or SYNTH_CONTAINERS_WHEEL is required"
        )
    wheel = Path(wheel_value).expanduser().resolve()
    if not wheel.is_file():
        raise RuntimeError(f"Jesterky Trace V5 wheel does not exist: {wheel}")
    actual_sha256 = hashlib.sha256(wheel.read_bytes()).hexdigest()
    if actual_sha256 != EXPECTED_CONTAINERS_WHEEL_SHA256:
        raise RuntimeError(
            "Jesterky Trace V5 synth-containers wheel digest mismatch: "
            f"expected {EXPECTED_CONTAINERS_WHEEL_SHA256}, found {actual_sha256}"
        )
    expected_environment = {
        "SYNTH_TRACE_CONTAINERS_VERSION": EXPECTED_CONTAINERS_VERSION,
        "SYNTH_TRACE_CONTAINERS_SOURCE_COMMIT": EXPECTED_CONTAINERS_SOURCE_COMMIT,
        "SYNTH_TRACE_CONTAINERS_WHEEL_SHA256": EXPECTED_CONTAINERS_WHEEL_SHA256,
    }
    for name, expected in expected_environment.items():
        configured = str(os.environ.get(name) or "").strip()
        if configured and configured != expected:
            raise RuntimeError(
                f"Jesterky Trace V5 {name} mismatch: expected {expected}, "
                f"found {configured}"
            )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--actor")
    parser.add_argument("--workflow-address")
    parser.add_argument("--attempt", type=int)
    parser.add_argument("--finish-status", choices=("completed", "failed", "interrupted"))
    parser.add_argument("--native-jsonl", type=Path)
    args = parser.parse_args()

    _verify_containers_runtime()
    parent = TraceContextV1.from_environment()
    if parent is None or not parent.collector_url:
        raise RuntimeError("complete parent Synth trace context is required")
    if args.native_jsonl is not None:
        imported = import_codex_jsonl(
            args.native_jsonl.resolve(),
            target_id=parent.actor_session_id,
        )
        with TraceEmitter.from_environment() as emitter:
            envelope_ids = [
                emitter.event(
                    event_type=str(event["event_type"]),
                    payload={
                        "native": event.get("body"),
                        "codex_id": event.get("codex_id"),
                        "authority": "codex_stdout_jsonl",
                    },
                )
                for event in imported.events
            ]
        print(
            json.dumps(
                {
                    "capture_id": parent.capture_id,
                    "session_id": parent.actor_session_id,
                    "event_count": len(envelope_ids),
                    "usage_snapshot_count": len(imported.usage_snapshots),
                },
                sort_keys=True,
            )
        )
        return 0
    if args.finish_status:
        with TraceEmitter.from_environment() as emitter:
            envelope_id = emitter.finish(status=args.finish_status)
        print(
            json.dumps(
                {
                    "capture_id": parent.capture_id,
                    "session_id": parent.actor_session_id,
                    "status": args.finish_status,
                    "envelope_id": envelope_id,
                },
                sort_keys=True,
            )
        )
        return 0
    if args.actor is None or args.workflow_address is None or args.attempt is None:
        parser.error("--actor, --workflow-address, and --attempt are required to register")
    key = {
        "actor": args.actor,
        "workflow_address": args.workflow_address,
        "attempt": args.attempt,
    }
    delegation_id = record_id(
        "deleg",
        kind="delegation",
        scope=(parent.trace_id, parent.actor_id),
        key=key,
    )
    actor_id = record_id(
        "actor",
        kind="actor",
        scope=(parent.trace_id,),
        key=key,
    )
    session_id = record_id(
        "sess",
        kind="session",
        scope=(parent.trace_id, actor_id),
        key=key,
    )
    capture_id = record_id(
        "cap",
        kind="delegated_capture",
        scope=(parent.trace_id, parent.capture_id),
        key=key,
    )
    child = TraceContextV1(
        trace_id=parent.trace_id,
        capture_id=capture_id,
        actor_id=actor_id,
        actor_session_id=session_id,
        parent_actor_id=parent.actor_id,
        parent_actor_session_id=parent.actor_session_id,
        parent_span_id=parent.parent_span_id,
        delegation_id=delegation_id,
        workflow_address=args.workflow_address,
        binding_path=parent.binding_path,
        collector_url=parent.collector_url,
        output_dir=parent.output_dir,
    )
    actor = ActorV5(
        actor_id=actor_id,
        kind=ActorKind.AGENT,
        display_name=args.actor,
        role=args.actor,
        parent_actor_id=parent.actor_id,
        harness="jesterky",
        workflow_id=os.environ.get("SYNTH_JESTERKY_RUN_ID"),
        metadata={
            "native_workflow_address": args.workflow_address,
            "attempt": args.attempt,
        },
    )
    session = SessionV5(
        session_id=session_id,
        actor_id=actor_id,
        started_at=utc_now(),
        attempt_id=str(args.attempt),
        workflow_id=os.environ.get("SYNTH_JESTERKY_RUN_ID"),
        capture_id=capture_id,
        parent_session_id=parent.actor_session_id,
        harness="jesterky",
        coverage=SessionCoverageV5(
            reasons=("derived_from_collector_records_at_finalization",),
        ),
        metadata={"native_workflow_address": args.workflow_address},
    )
    with TraceEmitter.from_environment() as emitter:
        registered = emitter.register_context(
            child,
            actor=actor.to_dict(),
            session=session.to_dict(),
        )
        child_collector_token = emitter.registered_context_token(child)
    if registered != capture_id:
        raise RuntimeError("collector registered a different child capture id")
    parent_collector_token = str(
        os.environ.get("SYNTH_TRACE_COLLECTOR_TOKEN") or ""
    ).strip()
    if not parent_collector_token:
        raise RuntimeError("parent collector capability is required")
    if child_collector_token == parent_collector_token:
        raise RuntimeError("collector reused the parent capability for a child context")
    registry_dir = str(os.environ.get("SYNTH_JESTERKY_TRACE_REGISTRY_DIR") or "").strip()
    if registry_dir:
        receipt_path = Path(registry_dir) / f"{capture_id}.json"
        receipt_path.parent.mkdir(parents=True, exist_ok=True)
        receipt_path.parent.chmod(0o700)
        receipt_path.write_text(
            json.dumps(
                {
                    "context": child.to_dict(),
                    "actor": actor.to_dict(),
                    "session": session.to_dict(),
                    "collector_token_scope": "child",
                    "collector_token_distinct_from_parent": True,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        receipt_path.chmod(0o600)
    child_environment = child.to_environment()
    child_environment["SYNTH_TRACE_COLLECTOR_TOKEN"] = child_collector_token
    print(json.dumps(child_environment, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
