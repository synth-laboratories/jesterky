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
import re
from pathlib import Path
from typing import Any


def sanitize_filename(raw: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9_-]+", "_", raw).strip("_")
    return cleaned or "trace"


def record_to_v4(record: dict[str, Any]) -> dict[str, Any]:
    seed = int(record.get("seed") or 0)
    rollout_id = str(record.get("rollout_id") or record.get("evidence_id") or f"seed-{seed}")
    status = str(record.get("status") or "completed")
    reward = float(record.get("reward") or 0.0)
    mid = record.get("mid_rollout_checkpoints")
    if not isinstance(mid, list):
        mid = []
    spans: list[dict[str, Any]] = []
    events: list[dict[str, Any]] = []
    for idx, checkpoint in enumerate(mid):
        if not isinstance(checkpoint, dict):
            continue
        call_index = int(checkpoint.get("policy_llm_call_index") or (idx + 1))
        span_id = f"{rollout_id}-span-{call_index:04d}"
        cp_reward = float(checkpoint.get("reward") or reward)
        labels = checkpoint.get("objective_labels") or []
        spans.append(
            {
                "span_id": span_id,
                "call_index": call_index,
                "run_id": rollout_id,
                "request": {
                    "messages": [
                        {"role": "system", "content": "GoEx Craftax search evidence"},
                        {
                            "role": "user",
                            "content": f"seed={seed}; checkpoint={call_index}",
                        },
                    ],
                    "provider_hint": "openai_compat",
                },
                "response": {
                    "message": {
                        "role": "assistant",
                        "content": json.dumps(checkpoint, sort_keys=True),
                    },
                    "usage": {},
                },
                "metrics": {
                    "reward_total": cp_reward,
                    "objective_labels": labels,
                },
                "metadata": {
                    "checkpoint_id": checkpoint.get("checkpoint_id"),
                    "evidence_id": record.get("evidence_id"),
                },
            }
        )
        events.append(
            {
                "type": "lm_call",
                "sequence_index": call_index,
                "span_id": span_id,
                "metadata": {"evidence_id": record.get("evidence_id")},
            }
        )
    if not spans:
        span_id = f"{rollout_id}-span-0001"
        spans.append(
            {
                "span_id": span_id,
                "call_index": 1,
                "run_id": rollout_id,
                "request": {
                    "messages": [
                        {"role": "system", "content": "GoEx Craftax search evidence"},
                        {
                            "role": "user",
                            "content": f"seed={seed}; status={status}",
                        },
                    ],
                    "provider_hint": "openai_compat",
                },
                "response": {
                    "message": {
                        "role": "assistant",
                        "content": f"reward={reward}",
                    },
                    "usage": {},
                },
                "metrics": {"reward_total": reward},
                "metadata": {"evidence_id": record.get("evidence_id")},
            }
        )
        events.append(
            {
                "type": "lm_call",
                "sequence_index": 1,
                "span_id": span_id,
                "metadata": {"evidence_id": record.get("evidence_id")},
            }
        )
    return {
        "schema_version": "synth_rollout_trace_v4",
        "trace_schema_version": 4,
        "rollout_id": rollout_id,
        "trace_correlation_id": str(record.get("evidence_id") or rollout_id),
        "status": status,
        "spans": spans,
        "events": events,
        "span_count": len(events),
        "summary": {
            "seed": seed,
            "outcome_reward": reward,
            "reward": reward,
            "achievements": [],
            "llm_turns": len(events),
            "split": record.get("split"),
            "candidate_id": record.get("candidate_id"),
        },
        "metadata": {
            "source": "export_goex_rollouts_to_v4.py",
            "evidence_id": record.get("evidence_id"),
            "candidate_id": record.get("candidate_id"),
            "dispatch_kind": record.get("dispatch_kind"),
            "theme_id": record.get("theme_id"),
            "trace_ref": record.get("trace_ref"),
        },
    }


def export_evidence(evidence_path: Path, out_dir: Path, *, lane: str) -> int:
    payload = json.loads(evidence_path.read_text())
    records = payload.get(lane) if isinstance(payload, dict) else None
    if not isinstance(records, list):
        raise SystemExit(f"evidence file missing list at key {lane!r}: {evidence_path}")
    out_dir.mkdir(parents=True, exist_ok=True)
    written = 0
    for record in records:
        if not isinstance(record, dict):
            continue
        trace = record_to_v4(record)
        evidence_id = str(record.get("evidence_id") or trace["rollout_id"])
        path = out_dir / f"{sanitize_filename(evidence_id)}.v4.json"
        path.write_text(json.dumps(trace, indent=2, sort_keys=True) + "\n")
        written += 1
    return written


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument(
        "--lane",
        default="search",
        choices=("search", "heldout_measurement"),
        help="Which evidence lane to export (default: search).",
    )
    args = parser.parse_args()
    n = export_evidence(args.evidence, args.out, lane=args.lane)
    print(json.dumps({"written": n, "out": str(args.out), "lane": args.lane}))


if __name__ == "__main__":
    main()
