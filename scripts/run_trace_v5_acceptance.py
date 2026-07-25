#!/usr/bin/env python3
"""Run bounded real Codex children in Jesterky and receipt one live V5 trace."""

from __future__ import annotations

import argparse
import hashlib
from importlib.metadata import version
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from synth_containers.tracing import (
    ApplicationTraceAssembler,
    CaptureMode,
    CaptureSupervisor,
    SupervisorConfig,
    TraceIdentityV5,
    TraceProvenanceV5,
    WorkloadKind,
)
from synth_containers.tracing.adapters import import_codex_jsonl

ROOT = Path(__file__).resolve().parents[1]
CHATGPT_CODEX_UPSTREAM = "https://chatgpt.com/backend-api/codex"
EXPECTED_CONTAINERS_VERSION = "0.3.0.20260725"
EXPECTED_CONTAINERS_SOURCE_COMMIT = "7a327c471c8850a1e8ea62fcea6813539c2a652e"
EXPECTED_CONTAINERS_WHEEL_SHA256 = (
    "1eafbca64b40c84c8c9d2554e68c8115605eea971444a378ca1d738fc39f61ee"
)
ACCEPTANCE_RUNTIME_ENVIRONMENT = (
    "PATH",
    "HOME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "CARGO_HOME",
    "RUSTUP_HOME",
)
CONTAINERS_PROVENANCE_ENVIRONMENT = (
    "SYNTH_TRACE_CONTAINERS_VERSION",
    "SYNTH_TRACE_CONTAINERS_SOURCE_COMMIT",
    "SYNTH_TRACE_CONTAINERS_WHEEL_SHA256",
    "SYNTH_TRACE_CONTAINERS_WHEEL_PATH",
)


def _digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def _runtime_environment(source: Mapping[str, str]) -> dict[str, str]:
    return {
        name: source[name]
        for name in ACCEPTANCE_RUNTIME_ENVIRONMENT
        if name in source
    }


def _acceptance_child_environment(
    *,
    source: Mapping[str, str],
    capture: Mapping[str, str],
    explicit: Mapping[str, str],
) -> dict[str, str]:
    selected = _runtime_environment(source)
    selected.update(explicit)
    for name in CONTAINERS_PROVENANCE_ENVIRONMENT:
        value = str(source.get(name) or "").strip()
        if not value:
            raise RuntimeError(
                f"Jesterky acceptance child requires explicit environment {name}"
            )
        selected[name] = value
    selected.update(capture)
    return selected


def _run(command: list[str], *, cwd: Path, env: dict[str, str]) -> dict[str, Any]:
    started = time.monotonic()
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )
    return {
        "command": command,
        "returncode": completed.returncode,
        "elapsed_seconds": round(time.monotonic() - started, 6),
        "stdout": completed.stdout[-4000:],
        "stderr": completed.stderr[-4000:],
    }


def _containers_provenance() -> dict[str, str]:
    installed = version("synth-containers")
    if installed != EXPECTED_CONTAINERS_VERSION:
        raise RuntimeError(
            "Jesterky acceptance requires synth-containers "
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
        raise RuntimeError(f"Jesterky acceptance wheel does not exist: {wheel}")
    wheel_sha256 = hashlib.sha256(wheel.read_bytes()).hexdigest()
    if wheel_sha256 != EXPECTED_CONTAINERS_WHEEL_SHA256:
        raise RuntimeError(
            "Jesterky acceptance synth-containers wheel digest mismatch: "
            f"expected {EXPECTED_CONTAINERS_WHEEL_SHA256}, found {wheel_sha256}"
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
                f"Jesterky acceptance {name} mismatch: expected {expected}, "
                f"found {configured}"
            )
        os.environ[name] = expected
    os.environ["SYNTH_TRACE_CONTAINERS_WHEEL_PATH"] = str(wheel)
    return {
        "version": EXPECTED_CONTAINERS_VERSION,
        "source_commit": EXPECTED_CONTAINERS_SOURCE_COMMIT,
        "wheel_sha256": wheel_sha256,
        "wheel_path": str(wheel),
    }


def _trace_cli(*args: str) -> dict[str, Any]:
    command = [sys.executable, "-m", "synth_containers.tracing.cli", *args]
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        env=_runtime_environment(os.environ),
    )
    parsed: Any = completed.stdout.strip()
    if parsed:
        try:
            parsed = json.loads(parsed)
        except json.JSONDecodeError:
            pass
    return {
        "command": command,
        "returncode": completed.returncode,
        "stdout": parsed,
        "stderr": completed.stderr.strip(),
    }


def _codex_home(source: Path, destination: Path, proxy_base_url: str) -> Path:
    auth = source / "auth.json"
    if not auth.is_file():
        raise RuntimeError(f"Codex ChatGPT auth is missing: {auth}")
    destination.mkdir(parents=True, exist_ok=True)
    destination.chmod(0o700)
    shutil.copy2(auth, destination / "auth.json")
    (destination / "auth.json").chmod(0o600)
    config_path = destination / "config.toml"
    config_path.write_text(
        'approval_policy = "never"\n'
        f"openai_base_url = {json.dumps(proxy_base_url)}\n",
        encoding="utf-8",
    )
    config_path.chmod(0o600)
    return destination


def _auth_secret_values(auth_path: Path) -> tuple[bytes, ...]:
    payload = json.loads(auth_path.read_text(encoding="utf-8"))
    values: set[bytes] = {auth_path.read_bytes()}

    def visit(value: Any, key: str = "") -> None:
        if isinstance(value, dict):
            for child_key, child_value in value.items():
                visit(child_value, str(child_key).lower())
            return
        if isinstance(value, list):
            for child in value:
                visit(child, key)
            return
        if (
            isinstance(value, str)
            and len(value) >= 8
            and any(marker in key for marker in ("token", "secret", "api_key", "authorization"))
        ):
            values.add(value.encode("utf-8"))

    visit(payload)
    return tuple(sorted(values))


def _assert_auth_absent(
    root: Path,
    auth_path: Path,
    *,
    extra_payloads: tuple[Any, ...] = (),
) -> dict[str, Any]:
    needles = _auth_secret_values(auth_path)
    scanned = sorted(path for path in root.rglob("*") if path.is_file())
    leaked: list[str] = []
    for path in scanned:
        body = path.read_bytes()
        if any(needle and needle in body for needle in needles):
            leaked.append(str(path))
    for index, payload in enumerate(extra_payloads):
        body = json.dumps(payload, sort_keys=True, default=str).encode("utf-8")
        if any(needle and needle in body for needle in needles):
            leaked.append(f"in-memory-payload:{index}")
    if leaked:
        raise RuntimeError(
            "Codex authentication material leaked into acceptance artifacts: "
            + ", ".join(leaked)
        )
    return {
        "passed": True,
        "files_scanned": len(scanned),
        "payloads_scanned": len(extra_payloads),
        "secret_values_scanned": len(needles),
    }


def _ingest_native(
    supervisor: CaptureSupervisor,
    *,
    manifest_path: Path,
    registry_dir: Path,
    codex_jsonl_dir: Path,
) -> dict[str, Any]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assembler = ApplicationTraceAssembler(supervisor.collector)
    workflow_events: list[str] = []
    for event in manifest.get("events") or []:
        if not isinstance(event, dict):
            continue
        workflow_events.append(
            assembler.jesterky_transition(
                event,
                actor_id=supervisor.binding.workload.root_actor_id,
                session_id=supervisor.binding.workload.actor_session_id,
            )
        )

    children: list[dict[str, Any]] = []
    for registration_path in sorted(registry_dir.glob("*.json")):
        registration = json.loads(registration_path.read_text(encoding="utf-8"))
        context = registration["context"]
        actor = registration["actor"]
        session = registration["session"]
        capture_id = str(context["capture_id"])
        raw_jsonl = codex_jsonl_dir / f"{capture_id}.jsonl"
        imported = import_codex_jsonl(raw_jsonl, target_id=str(session["session_id"]))
        event_ids: list[str] = []
        for event in imported.events:
            event_ids.append(
                supervisor.collector.event(
                    event_type=str(event["event_type"]),
                    payload={
                        "native": event.get("body"),
                        "codex_id": event.get("codex_id"),
                        "authority": "codex_stdout_jsonl",
                    },
                    actor_id=str(actor["actor_id"]),
                    session_id=str(session["session_id"]),
                )
            )
        for alias in imported.aliases:
            supervisor.declare_alias(alias)
        children.append(
            {
                "capture_id": capture_id,
                "actor_id": actor["actor_id"],
                "session_id": session["session_id"],
                "parent_actor_id": actor.get("parent_actor_id"),
                "parent_actor_session_id": context.get("parent_actor_session_id"),
                "delegation_id": context.get("delegation_id"),
                "workflow_address": context.get("workflow_address"),
                "role": actor.get("role"),
                "collector_token_scope": registration.get("collector_token_scope"),
                "collector_token_distinct_from_parent": registration.get(
                    "collector_token_distinct_from_parent"
                ),
                "native_jsonl": str(raw_jsonl),
                "native_jsonl_digest": _digest(raw_jsonl),
                "native_line_count": imported.line_count,
                "native_event_count": len(event_ids),
                "usage_snapshot_count": len(imported.usage_snapshots),
                "aliases": [alias.to_dict() for alias in imported.aliases],
            }
        )
    return {
        "workflow_event_ids": workflow_events,
        "children": children,
        "manifest": manifest,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--model", default="gpt-5.4-mini")
    parser.add_argument("--effort", default="low")
    parser.add_argument("--codex-home", type=Path)
    args = parser.parse_args()

    started = time.monotonic()
    containers_provenance = _containers_provenance()
    args.out.mkdir(parents=True, exist_ok=True)
    args.out.chmod(0o700)
    manifest_path = args.out / "jesterky_manifest.json"
    events_path = args.out / "jesterky_events.jsonl"
    bundle = args.out / "trace_bundle"
    registry_dir = args.out / "child_contexts"
    codex_jsonl_dir = args.out / "codex_native"
    source_codex_home = (
        args.codex_home
        or Path(os.environ.get("CODEX_HOME") or (Path.home() / ".codex"))
    ).expanduser()
    run_id = "trace-v5-stage1-jesterky"
    supervisor = CaptureSupervisor(
        SupervisorConfig(
            bundle_root=bundle,
            trace_key={"run_id": run_id, "workflow": "trace_v5_acceptance"},
            upstream_base_url=CHATGPT_CODEX_UPSTREAM,
            provenance=TraceProvenanceV5(
                producer="jesterky.trace_v5_acceptance",
                producer_version="1",
                source_format="synth.capture.raw.v1",
                model=args.model,
                provider="openai",
                harness="jesterky",
                extra={"synth_containers": containers_provenance},
            ),
            identity=TraceIdentityV5(
                run_id=run_id,
                benchmark="jesterky.trace_v5_acceptance",
            ),
            workload_kind=WorkloadKind.JESTERKY,
            root_actor_name="jesterky-orchestrator",
            mode=CaptureMode.REQUIRED,
            run_id=run_id,
            workflow_id=run_id,
        )
    )
    supervisor.start_capture()
    temporary_codex_home: Path | None = None
    try:
        with tempfile.TemporaryDirectory(prefix="jesterky-trace-v5-codex-home-") as raw_home:
            temporary_codex_home = Path(raw_home)
            codex_home = _codex_home(
                source_codex_home,
                temporary_codex_home,
                supervisor.openai_base_url,
            )
            env = _acceptance_child_environment(
                source=os.environ,
                capture=supervisor.environment(),
                explicit={
                    "SYNTH_TRACE_CHILD_REGISTRAR": str(
                        ROOT / "scripts" / "register_trace_child.py"
                    ),
                    "SYNTH_TRACE_CHILD_REGISTRAR_PYTHON": sys.executable,
                    "SYNTH_JESTERKY_TRACE_REGISTRY_DIR": str(registry_dir),
                    "SYNTH_JESTERKY_CODEX_JSONL_DIR": str(codex_jsonl_dir),
                    "SYNTH_JESTERKY_RUN_ID": run_id,
                },
            )
            command = [
                args.cargo,
                "run",
                "-p",
                "jesterky-cli",
                "--",
                "run",
                "examples/trace_v5_acceptance.json",
                "--actor",
                "codex",
                "--model",
                args.model,
                "--effort",
                args.effort,
                "--codex-home",
                str(codex_home),
                "--args",
                json.dumps(
                    {
                        "jobs": [
                            {"dimension": "correctness", "target": "trace-v5-bounded"},
                            {"dimension": "robustness", "target": "trace-v5-bounded"},
                        ]
                    },
                    separators=(",", ":"),
                ),
                "--run-id",
                run_id,
                "--out",
                str(manifest_path),
                "--events-out",
                str(events_path),
                "--no-follow",
            ]
            execution = _run(command, cwd=ROOT, env=env)
        native = _ingest_native(
            supervisor,
            manifest_path=manifest_path,
            registry_dir=registry_dir,
            codex_jsonl_dir=codex_jsonl_dir,
        )
    except BaseException:
        supervisor.finalize(status="interrupted")
        raise
    sealed = supervisor.finalize(
        status="completed" if execution["returncode"] == 0 else "failed"
    )

    grounded_event = (
        native["workflow_event_ids"][0] if native["workflow_event_ids"] else None
    )
    workflow_completed = (
        execution["returncode"] == 0
        and str(native["manifest"].get("status") or "").lower() == "completed"
    )
    workflow_reward = 1.0 if workflow_completed else 0.0
    evidence_files = []
    for sequence, payload in enumerate(
        (
            {
                "schema_version": "jesterky.native-evaluation.v1",
                "attempt": "initial",
                "annotations": [
                    {
                        "label": "bounded_map_reduce_observed",
                        "grounding": "grounded",
                        "source_event_ids": [grounded_event] if grounded_event else [],
                    },
                    {
                        "label": "operator_summary",
                        "grounding": "summary_only",
                        "source_event_ids": [],
                    },
                ],
                "reward": {
                    "name": "workflow_lifecycle_completed",
                    "intent": "Binary lifecycle completion diagnostic; not task quality.",
                    "value": workflow_reward,
                    "lower_bound": 0.0,
                    "upper_bound": 1.0,
                    "trainable": False,
                },
            },
            {
                "schema_version": "jesterky.native-evaluation.v1",
                "attempt": "posthoc_review",
                "relation": "review_of:initial",
                "annotations": [
                    {
                        "label": "append_only_posthoc_review",
                        "grounding": "summary_only",
                        "source_event_ids": [],
                    }
                ],
            },
        ),
        start=1,
    ):
        path = args.out / f"native_evaluation_{sequence}.json"
        path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        evidence_files.append(
            {
                "path": str(path),
                "digest": _digest(path),
                "operation": _trace_cli(
                    "attach",
                    str(bundle),
                    "--native-eval",
                    str(path),
                ),
            }
        )
    validation = _trace_cli("validate", str(bundle))
    atif = _trace_cli("project", str(bundle), "--format", "atif")
    pointer = json.loads((bundle / "manifest.json").read_text(encoding="utf-8"))
    bundle_manifest = json.loads(
        (bundle / str(pointer["relative_path"])).read_text(encoding="utf-8")
    )
    child_actors = [
        actor
        for actor in sealed.document.actors
        if actor.parent_actor_id == supervisor.binding.workload.root_actor_id
    ]
    registered_session_ids = {
        str(child["session_id"]) for child in native["children"]
    }
    child_sessions = [
        session
        for session in sealed.document.sessions
        if session.session_id in registered_session_ids
    ]
    child_actor_ids = {actor.actor_id for actor in child_actors}
    provider_spans = [
        span
        for span in sealed.document.spans
        if str(span.span_kind) == "model_call"
    ]
    native_aliases = [
        alias.to_dict()
        for alias in sealed.document.aliases
        if str(alias.namespace).startswith("codex.")
    ]
    auth_residue_absent = bool(
        temporary_codex_home is not None and not temporary_codex_home.exists()
    )
    auth_secret_scan = _assert_auth_absent(
        args.out,
        source_codex_home / "auth.json",
        extra_payloads=(execution,),
    )
    initial_evidence = json.loads(Path(evidence_files[0]["path"]).read_text(encoding="utf-8"))
    review_evidence = json.loads(Path(evidence_files[1]["path"]).read_text(encoding="utf-8"))
    criteria = {
        "workflow_returncode_zero": execution["returncode"] == 0,
        "workflow_reward_matches_actual_completion": (
            float(initial_evidence["reward"]["value"])
            == (1.0 if workflow_completed else 0.0)
            and initial_evidence["reward"].get("trainable") is False
            and "reward" not in review_evidence
            and "reward_aggregation" not in review_evidence
        ),
        "codex_auth_temp_home_removed": auth_residue_absent,
        "codex_auth_secret_scan_clean": bool(auth_secret_scan["passed"]),
        "real_codex_children_registered": len(child_actors) >= 2,
        "distinct_child_captures": len(
            {child["capture_id"] for child in native["children"]}
        )
        >= 2,
        "distinct_child_collector_capabilities": all(
            child.get("collector_token_scope") == "child"
            and child.get("collector_token_distinct_from_parent") is True
            for child in native["children"]
        ),
        "canonical_child_context_complete": all(
            child.get("parent_actor_id")
            and child.get("parent_actor_session_id")
            and child.get("delegation_id")
            and child.get("workflow_address")
            and str(child.get("capture_id") or "").startswith("cap_")
            and str(child.get("actor_id") or "").startswith("actor_")
            and str(child.get("session_id") or "").startswith("sess_")
            for child in native["children"]
        ),
        "child_sessions_terminal": (
            len(child_sessions) == len(native["children"])
            and all(
                str(session.status) in {"completed", "failed", "interrupted"}
                and session.ended_at is not None
                for session in child_sessions
            )
        ),
        "codex_native_events_present": all(
            child["native_event_count"] > 0 for child in native["children"]
        ),
        "codex_native_aliases_present": bool(native_aliases),
        "provider_spans_present": len(provider_spans) >= 2,
        "provider_spans_bound_to_children": (
            len(provider_spans) >= 2
            and all(
                span.actor_id in child_actor_ids
                and span.session_id in registered_session_ids
                for span in provider_spans
            )
        ),
        "observed_provider_tokens_present": (
            int(sealed.document.usage.requests or 0) >= 2
            and int(sealed.document.usage.total_tokens or 0) > 0
        ),
        "grounded_and_summary_annotations_attached": all(
            item["operation"]["returncode"] == 0 for item in evidence_files
        ),
        "append_only_evidence_versions": len(bundle_manifest.get("evidence") or []) >= 2,
        "validation_succeeded": validation["returncode"] == 0,
        "atif_projection_present": atif["returncode"] == 0,
    }
    receipt = {
        "schema_version": "jesterky.trace-v5-live-acceptance.v1",
        "containers": containers_provenance,
        "execution": execution,
        "source_refs": [
            {
                "path": str(ROOT / "examples" / "trace_v5_acceptance.json"),
                "digest": _digest(ROOT / "examples" / "trace_v5_acceptance.json"),
            }
        ],
        "elapsed_seconds": round(time.monotonic() - started, 6),
        "native_manifest": str(manifest_path),
        "native_events": str(events_path),
        "native_import": native["children"],
        "auth_secret_scan": auth_secret_scan,
        "bundle": str(bundle),
        "bundle_manifest_digest": pointer.get("manifest_digest"),
        "trace_id": sealed.document.trace_id,
        "trace_digest": sealed.document.content_digest,
        "child_actor_ids": [actor.actor_id for actor in child_actors],
        "native_aliases": native_aliases,
        "evidence": evidence_files,
        "evidence_digests": [
            item.get("bundle_digest") for item in bundle_manifest.get("evidence") or []
        ],
        "token_usage": sealed.document.usage.to_dict(),
        "validation": validation,
        "atif_projection": atif,
        "criteria": criteria,
        "passed": all(criteria.values()),
    }
    auth_secret_scan = _assert_auth_absent(
        args.out,
        source_codex_home / "auth.json",
        extra_payloads=(receipt,),
    )
    receipt["auth_secret_scan"] = auth_secret_scan
    receipt["criteria"]["codex_auth_secret_scan_clean"] = bool(
        auth_secret_scan["passed"]
    )
    receipt["passed"] = all(receipt["criteria"].values())
    receipt_path = args.out / "jesterky_trace_v5_acceptance_receipt.json"
    receipt_path.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))
    return 0 if receipt["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
