#!/usr/bin/env python3
"""ReportBench A/B: does trace-evaluate guidance improve a report's autograde?

Arm A is each graded lane's report exactly as it was autograded (reward read
from autograde_latest.json — the run is not re-executed). Arm B is a revised
report: `prepare` copies the lane's report into a workdir next to a PROMPT.md
carrying the rubric and the live trace-evaluate verdict, and prints the exact
`codex exec` command that performs the revision out-of-band — this script
never asks a model for tokens. `--grade-only` re-grades the revised report
with the real evals grader (`validate_report.grade_rubric`) and emits the A/B
table. Nothing under the evals tree is modified.
"""

import argparse
import importlib.util
import json
import shlex
import shutil
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DEFAULT_EVALS_ROOT = REPO.parent / "evals" / "reportbench"
EVALS_LANES = DEFAULT_EVALS_ROOT / "lanes"
GRADER = DEFAULT_EVALS_ROOT / "validate_report.py"
DEFAULT_SCORE = REPO / "proof" / "smr_reportbench_score.json"
DEFAULT_WORKDIR = REPO / "proof" / "reportbench_ab"


def die(message: str) -> None:
    raise SystemExit(f"error: {message}")


def load_json(path: Path):
    try:
        with open(path) as fh:
            return json.load(fh)
    except FileNotFoundError:
        die(f"missing required file: {path}")
    except json.JSONDecodeError as exc:
        die(f"could not parse {path}: {exc}")


def load_grade_rubric():
    if not GRADER.is_file():
        die(f"evals grader not found: {GRADER}")
    spec = importlib.util.spec_from_file_location("validate_report", GRADER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if not hasattr(module, "grade_rubric"):
        die(f"{GRADER} has no grade_rubric()")
    return module.grade_rubric


def lane_grade(lanes_dir: Path, lane: str) -> dict:
    """Grade record with rubric/report re-anchored to the live lane dir.

    autograde_latest.json records absolute paths from the retired
    evals/smr/reportbench sibling tree; the lane dir holds the live copies.
    """
    autograde = load_json(lanes_dir / lane / "artifacts" / "autograde_latest.json")
    grade = autograde.get("grade")
    if not isinstance(grade, dict):
        die(f"lane {lane}: autograde_latest.json has no grade object")
    if grade.get("reward") is None:
        die(f"lane {lane}: grade.reward missing")
    rubric_path = lanes_dir / lane / "RUBRIC.json"
    if not rubric_path.is_file():
        die(f"lane {lane}: rubric missing: {rubric_path}")
    rubric = load_json(rubric_path)
    report_rel = str(rubric.get("report_file") or "").strip()
    if not report_rel:
        die(f"lane {lane}: rubric has no report_file")
    grade["rubric_path"] = str(rubric_path)
    grade["report_path"] = str((rubric_path.parent / report_rel).resolve())
    return grade


def rubric_b(grade: dict, workdir_lane: Path) -> Path:
    """Copy the lane rubric into the workdir, pointed at report_b.md.

    grade_rubric resolves every path relative to the rubric's parent dir;
    required file paths are rewritten absolute so they still hit the lane.
    """
    rubric_src = Path(grade["rubric_path"])
    rubric = load_json(rubric_src)
    rubric["report_file"] = "report_b.md"
    for key in ("required_file_paths", "required_files"):
        if isinstance(rubric.get(key), list):
            rubric[key] = [str((rubric_src.parent / str(rel)).resolve()) for rel in rubric[key]]
    out = workdir_lane / "rubric_b.json"
    with open(out, "w") as fh:
        json.dump(rubric, fh, indent=2)
        fh.write("\n")
    return out


def prompt_md(grade: dict, verdict_row: dict) -> str:
    rubric_text = Path(grade["rubric_path"]).read_text()
    return (
        "# Revise report_b.md to satisfy the rubric — without fabricating results\n\n"
        "Edit ONLY `report_b.md` in this directory. Revise it so it satisfies the\n"
        "rubric checks below (headings, required regex patterns). You may reorganize,\n"
        "retitle, and fill in sections from information already present in the report;\n"
        "you MUST NOT invent measurements, rewards, commands, or outcomes that the\n"
        "report does not already support. If a required section has no supporting\n"
        "content, state plainly that the run did not produce it.\n\n"
        "## Trace-evaluate verdict for this run (the guidance under test)\n\n"
        f"- verdict: {verdict_row['workflow_verdict']}\n"
        f"- severity: {verdict_row['severity']}\n"
        f"- rationale: {verdict_row['rationale']}\n"
        f"- autograde at grading time: {verdict_row['checks_passed']}/{verdict_row['checks_total']}\n\n"
        "## Rubric (JSON)\n\n```json\n"
        f"{rubric_text}\n```\n"
    )


def prepare(lanes_dir: Path, score_path: Path, workdir: Path, only_lane: str | None) -> None:
    score = load_json(score_path)
    rows = score.get("rows")
    if not rows:
        die(f"{score_path} has no rows")
    for row in rows:
        lane = row["trace_id"]
        if only_lane and lane != only_lane:
            continue
        grade = lane_grade(lanes_dir, lane)
        lane_dir = workdir / lane
        lane_dir.mkdir(parents=True, exist_ok=True)
        report_src = Path(grade["report_path"])
        if not report_src.is_file():
            die(f"lane {lane}: graded report missing: {report_src}")
        shutil.copyfile(report_src, lane_dir / "report_b.md")
        rubric_b(grade, lane_dir)
        (lane_dir / "PROMPT.md").write_text(prompt_md(grade, row))
        cmd = (
            f"codex exec -m gpt-5.5 -c model_reasoning_effort=xhigh "
            f"--sandbox workspace-write --skip-git-repo-check -C {shlex.quote(str(lane_dir))} "
            f'-o {shlex.quote(str(lane_dir / "revision_summary.txt"))} '
            f'"$(cat {shlex.quote(str(lane_dir / "PROMPT.md"))})"'
        )
        print(f"== {lane}: arm A reward {grade['reward']} — revise with:\n{cmd}\n")


def grade_only(lanes_dir: Path, score_path: Path, workdir: Path, only_lane: str | None) -> None:
    grade_rubric = load_grade_rubric()
    score = load_json(score_path)
    rows = [r for r in score.get("rows", []) if not only_lane or r["trace_id"] == only_lane]
    if not rows:
        die(f"no matching rows in {score_path}")
    table = []
    for row in rows:
        lane = row["trace_id"]
        grade_a = lane_grade(lanes_dir, lane)
        rubric_path = workdir / lane / "rubric_b.json"
        if not rubric_path.is_file():
            die(f"lane {lane}: not prepared (missing {rubric_path}) — run prepare first")
        grade_b = grade_rubric(rubric_path)
        if grade_b.get("fatal_errors"):
            die(f"lane {lane}: arm B grade fatal: {grade_b['fatal_errors']}")
        table.append(
            {
                "lane": lane,
                "arm_a_reward": grade_a["reward"],
                "arm_a_checks": f"{grade_a.get('checks_passed')}/{grade_a.get('checks_total')}",
                "arm_b_reward": grade_b["reward"],
                "arm_b_checks": f"{grade_b['checks_passed']}/{grade_b['checks_total']}",
                "delta": round(grade_b["reward"] - grade_a["reward"], 4),
            }
        )
    mean_a = sum(r["arm_a_reward"] for r in table) / len(table)
    mean_b = sum(r["arm_b_reward"] for r in table) / len(table)
    report = {
        "rows": table,
        "aggregates": {
            "n": len(table),
            "mean_arm_a": round(mean_a, 4),
            "mean_arm_b": round(mean_b, 4),
            "mean_delta": round(mean_b - mean_a, 4),
        },
    }
    workdir.mkdir(parents=True, exist_ok=True)
    with open(workdir / "ab_table.json", "w") as fh:
        json.dump(report, fh, indent=2)
        fh.write("\n")
    lines = [
        "# ReportBench A/B — baseline report vs trace-evaluate-guided revision",
        "",
        "| lane | arm A (as graded) | arm B (revised) | delta |",
        "|------|------------------:|----------------:|------:|",
    ]
    for r in table:
        lines.append(
            f"| {r['lane']} | {r['arm_a_reward']} ({r['arm_a_checks']}) | "
            f"{r['arm_b_reward']} ({r['arm_b_checks']}) | {r['delta']:+.4f} |"
        )
    agg = report["aggregates"]
    lines += [
        "",
        f"n={agg['n']} · mean A={agg['mean_arm_a']} · mean B={agg['mean_arm_b']} · "
        f"mean delta={agg['mean_delta']:+.4f}",
        "",
    ]
    (workdir / "ab_table.md").write_text("\n".join(lines))
    print(json.dumps(report["aggregates"]))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lanes-dir", type=Path, default=EVALS_LANES)
    parser.add_argument("--score", type=Path, default=DEFAULT_SCORE)
    parser.add_argument("--workdir", type=Path, default=DEFAULT_WORKDIR)
    parser.add_argument("--lane", help="restrict to one lane")
    parser.add_argument("--grade-only", action="store_true")
    args = parser.parse_args()

    if args.grade_only:
        grade_only(args.lanes_dir, args.score, args.workdir, args.lane)
    else:
        prepare(args.lanes_dir, args.score, args.workdir, args.lane)
    return 0


if __name__ == "__main__":
    sys.exit(main())
