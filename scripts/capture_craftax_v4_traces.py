#!/usr/bin/env python3
"""Capture Craftax ReAct container rollouts as synth_rollout_trace_v4 JSON files.

Runs sync rollouts against a live GameBench Craftax container (gemini-3.1-flash-lite
by default), normalizes turn artifacts into v4 traces, and writes one file per seed
under proof/craftax_v4_traces/.

Prereqs:
  - Rust gold on CRAFTAX_GOLD_URL (default http://127.0.0.1:8098)
  - Craftax ReAct container on CONTAINER_URL (default http://127.0.0.1:18104)
  - GEMINI_API_KEY in env or --env-file (default ~/Documents/GitHub/synth-ai/.env)
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import uuid
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

DEFAULT_GOLD_URL = "http://127.0.0.1:8098"
DEFAULT_CONTAINER_URL = "http://127.0.0.1:18104"
DEFAULT_OUT_DIR = Path(__file__).resolve().parents[1] / "proof" / "craftax_v4_traces"
DEFAULT_ENV_FILE = Path.home() / "Documents" / "GitHub" / "synth-ai" / ".env"
DEFAULT_SEEDS = [1, 2, 3, 4, 5, 6, 7, 8]


def load_env_file(path: Path) -> None:
    if not path.is_file():
        return
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        if key and key not in os.environ:
            os.environ[key] = value.strip().strip('"').strip("'")


def http_json(method: str, url: str, payload: dict[str, Any] | None = None, timeout_s: float = 600.0) -> Any:
    data = None
    headers = {"Accept": "application/json"}
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = Request(url, data=data, headers=headers, method=method)
    with urlopen(req, timeout=timeout_s) as resp:
        body = resp.read().decode("utf-8")
        return json.loads(body) if body else {}


def wait_for_health(label: str, url: str, timeout_s: float = 120.0) -> None:
    deadline = time.time() + timeout_s
    last_err = ""
    while time.time() < deadline:
        try:
            payload = http_json("GET", f"{url.rstrip('/')}/health", timeout_s=10.0)
            if isinstance(payload, dict) and (
                payload.get("ok") is True or payload.get("healthy") is True or payload.get("status") == "ok"
            ):
                return
            last_err = f"unexpected health payload: {payload}"
        except (HTTPError, URLError, TimeoutError, json.JSONDecodeError) as exc:
            last_err = str(exc)
        time.sleep(2.0)
    raise RuntimeError(f"{label} not healthy at {url}: {last_err}")


def turns_from_record(record: dict[str, Any]) -> list[dict[str, Any]]:
    for key in ("artifacts", "artifact"):
        items = record.get(key)
        if not isinstance(items, list):
            continue
        for item in items:
            if isinstance(item, dict) and item.get("artifact_type") == "turns":
                turns = item.get("turns")
                if isinstance(turns, list):
                    return [t for t in turns if isinstance(t, dict)]
    return []


def build_v4_trace(record: dict[str, Any], *, seed: int, max_steps: int) -> dict[str, Any]:
    rollout_id = str(record.get("rollout_id") or uuid.uuid4())
    trace_correlation_id = str(record.get("trace_correlation_id") or rollout_id)
    turns = turns_from_record(record)
    reward_info = record.get("reward_info") if isinstance(record.get("reward_info"), dict) else {}
    details = reward_info.get("details") if isinstance(reward_info.get("details"), dict) else {}
    outcome_reward = float(reward_info.get("outcome_reward") or details.get("total_reward") or 0.0)
    achievements = details.get("achievements") or []
    if not isinstance(achievements, list):
        achievements = []
    metadata = record.get("metadata") if isinstance(record.get("metadata"), dict) else {}
    spans: list[dict[str, Any]] = []
    events: list[dict[str, Any]] = []
    llm_call_index = 0
    for turn in turns:
        if int(turn.get("batch_index") or 0) != 0:
            continue
        llm_call_index += 1
        assistant_text = str(turn.get("assistant_text") or "")
        action = str(turn.get("action") or "noop")
        span_id = f"{rollout_id}-span-{llm_call_index:04d}"
        user_text = (
            f"Craftax turn {turn.get('ply', llm_call_index)}; "
            f"reward_total={turn.get('reward_total', 0.0)}; "
            f"achievements={turn.get('achievements', [])}"
        )
        llm_request = {
            "messages": [
                {"role": "system", "content": "Craftax ReAct policy"},
                {"role": "user", "content": user_text},
            ],
            "model": turn.get("model") or metadata.get("model") or "gemini-3.1-flash-lite",
        }
        llm_response = {
            "message": {"role": "assistant", "content": assistant_text},
            "usage": turn.get("usage") if isinstance(turn.get("usage"), dict) else {},
        }
        span = {
            "span_id": span_id,
            "call_index": llm_call_index,
            "run_id": rollout_id,
            "request": {
                "messages": llm_request["messages"],
                "provider_hint": "openai_compat",
            },
            "response": {
                "message": llm_response["message"],
                "usage": llm_response.get("usage"),
            },
            "metrics": {
                "env_action": action,
                "invalid_parse": bool(turn.get("invalid_parse")),
                "repaired": bool(turn.get("repaired")),
                "reward_total": float(turn.get("reward_total") or 0.0),
            },
            "metadata": {
                "batch_size": turn.get("batch_size"),
                "request_id": turn.get("request_id"),
            },
        }
        spans.append(span)
        events.append(
            {
                "type": "lm_call",
                "sequence_index": llm_call_index,
                "span_id": span_id,
                "llm_request": llm_request,
                "llm_response": llm_response,
                "metadata": {"env_action": action},
            }
        )
    status = str(record.get("status") or "completed")
    return {
        "schema_version": "synth_rollout_trace_v4",
        "trace_schema_version": 4,
        "rollout_id": rollout_id,
        "trace_correlation_id": trace_correlation_id,
        "status": status,
        "spans": spans,
        "events": events,
        "span_count": len(spans),
        "summary": {
            "seed": seed,
            "max_steps": max_steps,
            "outcome_reward": outcome_reward,
            "reward": outcome_reward,
            "achievements": achievements,
            "llm_turns": llm_call_index,
            "invalid_parse_turns": sum(1 for t in turns if t.get("invalid_parse")),
            "policy_provider": metadata.get("provider") or "gemini",
            "policy_model": metadata.get("model") or "gemini-3.1-flash-lite",
        },
        "metadata": {
            **metadata,
            "source": "capture_craftax_v4_traces.py",
            "env": "gamebench.craftax-singleplayer.rust_gold",
        },
    }


def rollout_payload(*, seed: int, max_steps: int, max_llm_turns: int, model: str) -> dict[str, Any]:
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
                "provider": "gemini",
                "model": model,
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
            "model": model,
            "provider": "gemini",
        },
    }


def capture_one(
    *,
    container_url: str,
    seed: int,
    max_steps: int,
    max_llm_turns: int,
    model: str,
    out_dir: Path,
) -> Path:
    payload = rollout_payload(seed=seed, max_steps=max_steps, max_llm_turns=max_llm_turns, model=model)
    record = http_json("POST", f"{container_url.rstrip('/')}/rollout", payload, timeout_s=600.0)
    if not isinstance(record, dict):
        raise RuntimeError(f"rollout seed={seed} returned non-object response")
    if str(record.get("status", "")).lower() not in {"completed", "succeeded", "success", "done"}:
        # sync path should return final record; tolerate missing status when reward present.
        if record.get("reward_info") is None and record.get("artifact") is None:
            raise RuntimeError(f"rollout seed={seed} incomplete: {record.get('status')}")
    trace = build_v4_trace(record, seed=seed, max_steps=max_steps)
    out_path = out_dir / f"seed-{seed}.v4.json"
    out_path.write_text(json.dumps(trace, indent=2, sort_keys=True) + "\n")
    manifest_path = out_dir / f"seed-{seed}.rollout.json"
    manifest_path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
    reward = trace["summary"]["outcome_reward"]
    print(f"captured seed={seed} reward={reward} spans={trace['span_count']} -> {out_path}")
    return out_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gold-url", default=os.environ.get("CRAFTAX_GOLD_URL", DEFAULT_GOLD_URL))
    parser.add_argument("--container-url", default=os.environ.get("CONTAINER_URL", DEFAULT_CONTAINER_URL))
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    parser.add_argument("--env-file", type=Path, default=DEFAULT_ENV_FILE)
    parser.add_argument("--seeds", type=int, nargs="+", default=DEFAULT_SEEDS)
    parser.add_argument("--max-steps", type=int, default=20)
    parser.add_argument("--max-llm-turns", type=int, default=4)
    parser.add_argument("--model", default="gemini-3.1-flash-lite")
    parser.add_argument("--skip-health", action="store_true")
    args = parser.parse_args()

    load_env_file(args.env_file)
    args.out_dir.mkdir(parents=True, exist_ok=True)

    if not args.skip_health:
        wait_for_health("craftax_gold", args.gold_url)
        wait_for_health("craftax_container", args.container_url)

    index: list[dict[str, Any]] = []
    for seed in args.seeds:
        path = capture_one(
            container_url=args.container_url,
            seed=seed,
            max_steps=args.max_steps,
            max_llm_turns=args.max_llm_turns,
            model=args.model,
            out_dir=args.out_dir,
        )
        trace = json.loads(path.read_text())
        index.append(
            {
                "seed": seed,
                "path": str(path),
                "rollout_id": trace.get("rollout_id"),
                "reward": trace.get("summary", {}).get("outcome_reward"),
                "span_count": trace.get("span_count"),
            }
        )

    index_path = args.out_dir / "index.json"
    index_path.write_text(json.dumps({"traces": index}, indent=2, sort_keys=True) + "\n")
    print(f"wrote index -> {index_path} ({len(index)} traces)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
