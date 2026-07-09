#!/usr/bin/env python3
"""Score a jesterky smr_reportbench_trace_evaluate run manifest.

Joins the workflow's recorded evaluator verdicts (one per trace, from the
`evaluate_traces` map) against each trace's autograde reward in the trace
dir, and emits per-trace rows plus aggregates: mean autograde reward (the
ReportBench report score), workflow pass rate, and the agreement rate
between the workflow verdict and a perfect autograde.

An unparseable recorded output is a hard error naming the trace — no
silent skips.
"""

import argparse
import json
import sys
from pathlib import Path

EVALUATOR_ACTOR = "reportbench_trace_evaluator"


def recorded_verdicts(manifest: dict):
    """Yield (map_index, outputs) for every evaluator recording."""
    for rec in manifest.get("recorded", []):
        call = rec.get("call") or {}
        if call.get("actor") != EVALUATOR_ACTOR:
            continue
        index = None
        for part in (rec.get("addr") or {}).get("node_path", []):
            if isinstance(part, dict) and "index" in part:
                index = part["index"]
        yield index, rec.get("outputs")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--trace-dir", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--md")
    args = parser.parse_args()

    with open(args.manifest) as fh:
        manifest = json.load(fh)

    trace_dir = Path(args.trace_dir)
    traces = {}
    for path in sorted(trace_dir.glob("*.v4.json")):
        trace_id = path.name[: -len(".v4.json")]
        with open(path) as fh:
            traces[trace_id] = json.load(fh)
    if not traces:
        print(f"no *.v4.json traces in {trace_dir}", file=sys.stderr)
        return 1

    # trace.expand sorts jobs by trace_id; map index i is the i-th trace_id.
    ordered_ids = sorted(traces)

    rows = []
    for index, outputs in recorded_verdicts(manifest):
        if not isinstance(outputs, dict) or outputs.get("verdict") not in ("pass", "fail"):
            trace_hint = ordered_ids[index] if index is not None and index < len(ordered_ids) else "?"
            raw = json.dumps(outputs)[:200]
            print(
                f"unparseable evaluator verdict for trace `{trace_hint}` (map index {index}): {raw}",
                file=sys.stderr,
            )
            return 1
        trace_id = outputs.get("dimension")
        if trace_id not in traces:
            print(f"verdict dimension `{trace_id}` has no trace in {trace_dir}", file=sys.stderr)
            return 1
        autograde = traces[trace_id].get("evidence", {}).get("autograde", {})
        rows.append(
            {
                "trace_id": trace_id,
                "autograde_reward": autograde.get("reward"),
                "checks_passed": autograde.get("checks_passed"),
                "checks_total": autograde.get("checks_total"),
                "workflow_verdict": outputs["verdict"],
                "severity": outputs.get("severity"),
                "rationale": outputs.get("rationale"),
            }
        )

    if not rows:
        print(f"no `{EVALUATOR_ACTOR}` recordings in {args.manifest}", file=sys.stderr)
        return 1
    rows.sort(key=lambda r: r["trace_id"])

    n = len(rows)
    rewards = [r["autograde_reward"] for r in rows if isinstance(r["autograde_reward"], (int, float))]
    passes = sum(1 for r in rows if r["workflow_verdict"] == "pass")
    agree = sum(
        1 for r in rows if (r["workflow_verdict"] == "pass") == (r["autograde_reward"] == 1.0)
    )
    report = {
        "rows": rows,
        "aggregates": {
            "n": n,
            "mean_autograde_reward": sum(rewards) / len(rewards) if rewards else None,
            "workflow_pass_rate": passes / n,
            "agreement_rate": agree / n,
        },
    }
    with open(args.out, "w") as fh:
        json.dump(report, fh, indent=2)
        fh.write("\n")

    if args.md:
        lines = [
            "# SMR ReportBench trace-evaluate score",
            "",
            "| trace | autograde | checks | workflow verdict | severity | rationale |",
            "|-------|----------:|-------:|------------------|----------|-----------|",
        ]
        for r in rows:
            lines.append(
                f"| {r['trace_id']} | {r['autograde_reward']} | "
                f"{r['checks_passed']}/{r['checks_total']} | {r['workflow_verdict']} | "
                f"{r['severity']} | {r['rationale']} |"
            )
        agg = report["aggregates"]
        lines += [
            "",
            f"n={agg['n']} · report score (mean autograde reward)="
            f"{agg['mean_autograde_reward']} · workflow pass rate={agg['workflow_pass_rate']} · "
            f"agreement={agg['agreement_rate']}",
            "",
        ]
        with open(args.md, "w") as fh:
            fh.write("\n".join(lines))

    print(json.dumps(report["aggregates"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
