#!/usr/bin/env python3
"""Run paired Craftax prompt ablations for GEPA/GELO.

The runner posts directly to the GameBench Craftax ReAct container and varies
only ``policy.config.system_prompt`` between arms. It writes one raw rollout
record per seed/arm plus an ``ablation_summary.json`` with paired statistics.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import sys
import uuid
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from datetime import UTC, datetime
from enum import Enum
from functools import reduce
from math import gcd
from pathlib import Path

from http_contract import HttpMethod, request_json_object, wait_for_json_health
from json_contract import (
    JsonObject,
    JsonObjectReader,
    JsonValue,
    read_json_object,
)


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_GOLD_URL = "http://127.0.0.1:8098"
DEFAULT_CONTAINER_URL = "http://127.0.0.1:18104"
DEFAULT_SMOKE_SUMMARY = (
    REPO_ROOT / "proof" / "gepa_craftax_ablation" / "ablation_summary.json"
)
DEFAULT_OUT_ROOT = REPO_ROOT / "proof"
BASE_PROMPT_ORIGIN = "gamebench:craftax-singleplayer/react#DEFAULT_SYSTEM_PROMPT"
GEPA_PROMPT_ORIGIN = "gepa:craftax-agent-hillclimb/generation_000/proposal"
DEFAULT_SEED_START = 501
DEFAULT_N = 64
DEFAULT_MODEL = "gemini-3.1-flash-lite"
ENV_NAME = "gamebench.craftax-singleplayer.rust_gold"
TARGET_METRIC = "craftax_outcome_reward_mean"


class ArtifactType(str, Enum):
    TURNS = "turns"
COMPLETED_STATUS = "completed"
DEFAULT_TIE_EPSILON = 1e-9
VALID_ACTIONS = [
    "noop",
    "left",
    "right",
    "up",
    "down",
    "do",
    "sleep",
    "place_stone",
    "place_table",
    "place_furnace",
    "place_plant",
    "make_wood_pickaxe",
    "make_stone_pickaxe",
    "make_iron_pickaxe",
    "make_wood_sword",
    "make_stone_sword",
    "make_iron_sword",
]
OUTPUT_CONTRACT = (
    'Reply with JSON only, for example {"actions":["do","right","do","left","do"]}. '
    f"Use valid actions: {', '.join(VALID_ACTIONS)}."
)
@dataclass(frozen=True)
class PromptSpec:
    text: str
    provenance: JsonObject


@dataclass(frozen=True)
class ArmSpec:
    optimizer: str
    arm: str
    label: str
    prompt: str
    prompt_provenance: JsonObject


@dataclass(frozen=True)
class RolloutJob:
    arm: ArmSpec
    seed: int
    out_path: Path
    args: argparse.Namespace


def log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)


def utc_now() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def load_json(path: Path) -> JsonObject:
    return read_json_object(path).data


def write_json(path: Path, payload: JsonValue) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")


def request_json(
    method: HttpMethod,
    url: str,
    payload: JsonObject | None = None,
    timeout_s: float = 600.0,
) -> JsonObject:
    return request_json_object(method, url, payload, timeout_s).data


def wait_for_health(label: str, url: str, timeout_s: float = 120.0) -> None:
    wait_for_json_health(label, url, lambda _: True, timeout_s)
    log(f"[ablation] {label} healthy at {url}")


def parse_seeds(raw: str) -> list[int]:
    value = raw.strip()
    if not value:
        raise argparse.ArgumentTypeError("empty seed set")
    if "-" in value and "," not in value:
        start_s, end_s = value.split("-", 1)
        start = int(start_s)
        end = int(end_s)
        if end < start:
            raise argparse.ArgumentTypeError("seed range end must be >= start")
        return list(range(start, end + 1))
    seeds = [int(part.strip()) for part in value.split(",") if part.strip()]
    if not seeds:
        raise argparse.ArgumentTypeError("empty seed set")
    return seeds


def seeds_from_args(args: argparse.Namespace) -> list[int]:
    if args.seeds:
        return parse_seeds(args.seeds)
    return list(range(args.seed_start, args.seed_start + args.n))


def load_default_prompts(summary_path: Path) -> dict[str, PromptSpec]:
    prompts = read_json_object(summary_path).object("prompts")
    base = prompts.string("arm_a_base")
    gepa = prompts.string("arm_b_gepa")
    return {
        "base": PromptSpec(
            text=base,
            provenance={
                "source": str(summary_path),
                "json_path": "prompts.arm_a_base",
                "original_source": BASE_PROMPT_ORIGIN,
                "original_symbol": "DEFAULT_SYSTEM_PROMPT",
                "description": "Base ReAct prompt from smoke-run summary; output contract normalized.",
            },
        ),
        "gepa": PromptSpec(
            text=gepa,
            provenance={
                "source": str(summary_path),
                "json_path": "prompts.arm_b_gepa",
                "original_source": GEPA_PROMPT_ORIGIN,
                "original_json_path": "proposals[0].proposed_payload.payload.react_system_prompt",
                "description": "GEPA proposer prompt from smoke-run summary; output contract normalized.",
            },
        ),
    }


def normalize_output_contract(prompt: str) -> str:
    text = prompt.strip()
    if "Reply with JSON only" in text and "Use valid actions:" in text:
        return text
    return f"{text}\n\n{OUTPUT_CONTRACT}"


def prompt_from_text_file(path: Path, *, normalize: bool) -> PromptSpec:
    prompt = path.read_text().strip()
    if not prompt:
        raise RuntimeError(f"empty prompt file: {path}")
    return PromptSpec(
        text=normalize_output_contract(prompt) if normalize else prompt,
        provenance={
            "source": str(path),
            "description": "Prompt loaded from explicit file.",
            "output_contract_normalized": normalize,
        },
    )


def candidate_prompt(candidate: JsonObjectReader) -> str | None:
    direct = candidate.optional_string("react_system_prompt")
    if direct:
        return direct.strip()
    bundle = candidate.optional_object("lever_bundle")
    if bundle is None:
        return None
    values = bundle.optional_object("values")
    if values is None:
        return None
    prompt = values.optional_string("react_system_prompt")
    if prompt:
        return prompt.strip()
    return None


def discover_gelo_prompt(
    run_dirs: list[Path], *, allow_seed: bool, normalize: bool
) -> PromptSpec:
    searched: list[JsonObject] = []
    for run_dir in run_dirs:
        registry_path = run_dir / "artifacts" / "candidate_registry.json"
        if not registry_path.is_file():
            searched.append(
                {"run_dir": str(run_dir), "status": "missing_candidate_registry"}
            )
            continue
        registry = read_json_object(registry_path)
        candidates = registry.objects("candidates")
        champion_id = registry.string("champion_candidate_id")
        ordered = sorted(
            candidates,
            key=lambda item: 0
            if item.optional_string("candidate_id") == champion_id
            else 1,
        )
        for candidate in ordered:
            prompt = candidate_prompt(candidate)
            if not prompt:
                continue
            source = candidate.string("source")
            status = candidate.string("status")
            candidate_id = candidate.string("candidate_id")
            searched.append(
                {
                    "run_dir": str(run_dir),
                    "candidate_id": candidate_id,
                    "source": source,
                    "status": status,
                }
            )
            if source == "seed" and not allow_seed:
                continue
            if status not in {"accepted", "proposed"}:
                continue
            return PromptSpec(
                text=normalize_output_contract(prompt) if normalize else prompt,
                provenance={
                    "source": str(registry_path),
                    "json_path": f"candidates[{candidate_id}].react_system_prompt",
                    "candidate_id": candidate_id,
                    "candidate_source": source,
                    "candidate_status": status,
                    "champion_candidate_id": champion_id,
                    "output_contract_normalized": normalize,
                },
            )
    raise RuntimeError(
        "no genuine GELO prompt found; searched candidate registries contained only missing "
        f"or seed/non-accepted prompt candidates: {json.dumps(searched, sort_keys=True)}"
    )

def rollout_payload(job: RolloutJob) -> JsonObject:
    args = job.args
    rollout_id = (
        f"{job.arm.optimizer}-{job.arm.arm}-seed-{job.seed}-{uuid.uuid4().hex[:8]}"
    )
    return {
        "rollout_id": rollout_id,
        "trace_correlation_id": rollout_id,
        "env": {
            "env_name": "craftax-singleplayer",
            "seed": job.seed,
            "config": {
                "seed": job.seed,
                "max_steps": args.max_steps,
            },
        },
        "policy": {
            "policy_id": f"craftax_react_{job.arm.optimizer}_{job.arm.arm}_gemini_v1",
            "config": {
                "provider": "gemini",
                "model": args.model,
                "temperature": args.temperature,
                "max_tokens": args.max_tokens,
                "system_prompt": job.arm.prompt,
                "use_lm": True,
                "max_llm_turns": args.max_llm_turns,
                "min_actions_per_call": args.min_actions_per_call,
                "max_actions_per_call": args.max_actions_per_call,
            },
        },
        "metadata": {
            "ablation": f"{job.arm.optimizer}_craftax",
            "arm": job.arm.arm,
            "arm_label": job.arm.label,
            "seed": job.seed,
            "model": args.model,
            "provider": "gemini",
            "target_metric": TARGET_METRIC,
            "output_contract_normalized": True,
            "runner": "scripts/gepa_gelo_ablation.py",
        },
    }


def run_rollout(job: RolloutJob) -> tuple[RolloutJob, JsonObject, bool]:
    if job.out_path.is_file() and not job.args.overwrite:
        return job, load_json(job.out_path), True
    payload = rollout_payload(job)
    record = request_json(
        HttpMethod.POST,
        f"{job.args.container_url.rstrip('/')}/rollout",
        payload,
        timeout_s=job.args.rollout_timeout_s,
    )
    record_reader = JsonObjectReader(
        record, f"rollout seed={job.seed} arm={job.arm.arm}"
    )
    status = record_reader.string("status")
    if status != COMPLETED_STATUS:
        raise RuntimeError(
            f"rollout seed={job.seed} arm={job.arm.arm} returned status={status!r}"
        )
    write_json(job.out_path, record)
    return job, record, False


def turns_from_record(record: JsonObject) -> list[JsonObject]:
    reader = JsonObjectReader(record, "rollout")
    turn_artifacts = tuple(
        artifact
        for artifact in reader.objects("artifacts")
        if artifact.enum("artifact_type", ArtifactType) is ArtifactType.TURNS
    )
    if len(turn_artifacts) != 1:
        raise JsonContractError(
            "rollout.artifacts must contain exactly one turns artifact; "
            f"found {len(turn_artifacts)}"
        )
    return [turn.data for turn in turn_artifacts[0].objects("turns")]


def token_sum(turns: list[JsonObject], key: str) -> int:
    total = 0
    for turn in turns:
        usage = turn.get("usage") if isinstance(turn.get("usage"), dict) else {}
        value = usage.get(key)
        if isinstance(value, (int, float)):
            total += int(value)
    return total


def summarize_record(
    record: JsonObject, *, arm: str, seed: int, path: Path
) -> JsonObject:
    reward_info = (
        record.get("reward_info") if isinstance(record.get("reward_info"), dict) else {}
    )
    details = (
        reward_info.get("details")
        if isinstance(reward_info.get("details"), dict)
        else {}
    )
    if "outcome_reward" not in reward_info:
        raise RuntimeError(f"missing reward_info.outcome_reward in {path}")
    turns = turns_from_record(record)
    achievements = (
        details.get("achievements")
        if isinstance(details.get("achievements"), list)
        else []
    )
    return {
        "arm": arm,
        "seed": seed,
        "path": str(path),
        "rollout_id": str(record.get("rollout_id") or ""),
        "reward": float(reward_info["outcome_reward"]),
        "achievements": sorted(str(item) for item in achievements),
        "steps": int(details.get("steps") or details.get("step_index") or 0),
        "llm_calls": len(turns),
        "input_tokens": token_sum(turns, "input_tokens"),
        "output_tokens": token_sum(turns, "output_tokens"),
        "invalid_action_count": int(details.get("invalid_action_count") or 0),
        "status": str(record.get("status") or ""),
    }


def mean_or_none(values: list[float]) -> float | None:
    return statistics.mean(values) if values else None


def reward_block(items: list[JsonObject]) -> JsonObject:
    rewards = [float(item["reward"]) for item in items]
    return {
        "mean_reward": mean_or_none(rewards),
        "rewards": rewards,
        "reward_min": min(rewards) if rewards else None,
        "reward_max": max(rewards) if rewards else None,
        "completed_count": len(items),
    }


def normal_ci_95(values: list[float]) -> JsonObject:
    if len(values) < 2:
        return {
            "method": "normal_approx_z_1.96",
            "low": None,
            "high": None,
            "standard_error": None,
        }
    sd = statistics.stdev(values)
    se = sd / math.sqrt(len(values))
    margin = 1.96 * se
    mean = statistics.mean(values)
    return {
        "method": "normal_approx_z_1.96",
        "low": mean - margin,
        "high": mean + margin,
        "standard_error": se,
        "sample_sd": sd,
    }


def exact_sign_flip_p(
    deltas: list[float], *, scale: int, max_states: int
) -> JsonObject:
    if not deltas:
        return {
            "method": "exact_scaled_dp",
            "p_two_sided": None,
            "extreme_assignments": None,
            "total_assignments": 0,
        }
    scaled = [int(round(delta * scale)) for delta in deltas]
    divisor = reduce(gcd, (abs(value) for value in scaled if value != 0), 0) or 1
    weights = [value // divisor for value in scaled]
    max_abs = sum(abs(value) for value in weights)
    if (2 * max_abs + 1) > max_states:
        raise RuntimeError(
            "exact sign-flip state space too large after scaling "
            f"(states={2 * max_abs + 1}, max={max_states}, scale={scale}, gcd={divisor})"
        )
    counts: Counter[int] = Counter({0: 1})
    for weight in weights:
        next_counts: Counter[int] = Counter()
        for current_sum, count in counts.items():
            next_counts[current_sum + weight] += count
            next_counts[current_sum - weight] += count
        counts = next_counts
    observed = abs(sum(weights))
    extreme = sum(
        count for signed_sum, count in counts.items() if abs(signed_sum) >= observed
    )
    total = 2 ** len(weights)
    return {
        "method": "exact_scaled_dp",
        "p_two_sided": extreme / total,
        "extreme_assignments": extreme,
        "total_assignments": total,
        "scale": scale,
        "gcd": divisor,
        "observed_abs_sum": observed,
    }


def paired_summary(
    *,
    optimizer: str,
    arms: tuple[ArmSpec, ArmSpec],
    records_by_arm: dict[str, dict[int, tuple[JsonObject, Path]]],
    requested_seeds: list[int],
    args: argparse.Namespace,
) -> JsonObject:
    base_arm, candidate_arm = arms
    per_arm: dict[str, list[JsonObject]] = {base_arm.arm: [], candidate_arm.arm: []}
    for arm in arms:
        for seed in requested_seeds:
            item = records_by_arm.get(arm.arm, {}).get(seed)
            if item is None:
                continue
            record, path = item
            per_arm[arm.arm].append(
                summarize_record(record, arm=arm.arm, seed=seed, path=path)
            )

    base_seeds = {
        int(item["seed"])
        for item in per_arm[base_arm.arm]
        if item["status"] == COMPLETED_STATUS
    }
    candidate_seeds = {
        int(item["seed"])
        for item in per_arm[candidate_arm.arm]
        if item["status"] == COMPLETED_STATUS
    }
    paired_seeds = [
        seed
        for seed in requested_seeds
        if seed in base_seeds and seed in candidate_seeds
    ]

    by_arm_seed = {
        arm_name: {int(item["seed"]): item for item in items}
        for arm_name, items in per_arm.items()
    }
    paired_rows: list[JsonObject] = []
    deltas: list[float] = []
    for seed in paired_seeds:
        base_reward = float(by_arm_seed[base_arm.arm][seed]["reward"])
        candidate_reward = float(by_arm_seed[candidate_arm.arm][seed]["reward"])
        delta = candidate_reward - base_reward
        deltas.append(delta)
        paired_rows.append(
            {
                "seed": seed,
                "arm_a_reward": base_reward,
                "arm_b_reward": candidate_reward,
                "delta": delta,
                "winner": candidate_arm.arm
                if delta > args.tie_epsilon
                else base_arm.arm
                if delta < -args.tie_epsilon
                else "tie",
            }
        )

    mean_delta = statistics.mean(deltas) if deltas else None
    ci = normal_ci_95(deltas)
    permutation = exact_sign_flip_p(
        deltas, scale=args.permutation_scale, max_states=args.max_permutation_states
    )
    p_two_sided = permutation.get("p_two_sided")
    ci_low = ci.get("low")
    ci_high = ci.get("high")
    ci_excludes_zero = bool(
        ci_low is not None and ci_high is not None and (ci_low > 0 or ci_high < 0)
    )
    p_lt_0_05 = bool(p_two_sided is not None and p_two_sided < 0.05)
    significance = {
        "ci_excludes_zero": ci_excludes_zero,
        "p_lt_0_05": p_lt_0_05,
        "headline_claim_allowed": len(paired_seeds) >= args.min_paired_n
        and (ci_excludes_zero or p_lt_0_05),
    }

    return {
        "ablation": f"{optimizer}_craftax",
        "target_metric": TARGET_METRIC,
        "container": args.container_url,
        "gold_url": args.gold_url,
        "model": args.model,
        "temperature": args.temperature,
        "max_tokens": args.max_tokens,
        "env": ENV_NAME,
        "max_steps": args.max_steps,
        "max_llm_turns": args.max_llm_turns,
        "requested_seeds": requested_seeds,
        "paired_seeds": paired_seeds,
        "n": len(paired_seeds),
        "min_paired_n": args.min_paired_n,
        f"arm_a_{base_arm.arm}": reward_block(
            [by_arm_seed[base_arm.arm][seed] for seed in paired_seeds]
        ),
        f"arm_b_{candidate_arm.arm}": reward_block(
            [by_arm_seed[candidate_arm.arm][seed] for seed in paired_seeds]
        ),
        "uplift_mean": mean_delta,
        "paired_deltas": deltas,
        "paired_mean_delta": mean_delta,
        "paired_wins": sum(1 for delta in deltas if delta > args.tie_epsilon),
        "paired_ties": sum(1 for delta in deltas if abs(delta) <= args.tie_epsilon),
        "paired_losses": sum(1 for delta in deltas if delta < -args.tie_epsilon),
        "stats": {
            "delta_ci_95": ci,
            "sign_flip_permutation": permutation,
        },
        "significance": significance,
        "generated_at": utc_now(),
        "prompts": {
            f"arm_a_{base_arm.arm}": base_arm.prompt,
            f"arm_b_{candidate_arm.arm}": candidate_arm.prompt,
        },
        "prompt_provenance": {
            f"arm_a_{base_arm.arm}": base_arm.prompt_provenance,
            f"arm_b_{candidate_arm.arm}": candidate_arm.prompt_provenance,
        },
        "fairness": {
            "arms_differ_only_by": "policy.config.system_prompt",
            "same_container": True,
            "same_seed_set": True,
            "same_model": True,
            "same_decode_config": True,
            "same_output_contract": True,
            "output_contract": OUTPUT_CONTRACT,
            "tie_epsilon": args.tie_epsilon,
            "incomplete_seeds_dropped_from_both_arms": True,
        },
        "per_seed": {
            base_arm.arm: per_arm[base_arm.arm],
            candidate_arm.arm: per_arm[candidate_arm.arm],
            "paired": paired_rows,
        },
        "incomplete": {
            base_arm.arm: [seed for seed in requested_seeds if seed not in base_seeds],
            candidate_arm.arm: [
                seed for seed in requested_seeds if seed not in candidate_seeds
            ],
            "dropped_unpaired": [
                seed for seed in requested_seeds if seed not in paired_seeds
            ],
        },
    }


def fmt(value: float | None, digits: int = 3) -> str:
    if value is None:
        return "n/a"
    return f"{value:.{digits}f}"


def provenance_md(provenance: JsonObject) -> str:
    source = provenance.get("source")
    if not isinstance(source, str) or not source.strip():
        raise RuntimeError("prompt provenance missing non-empty `source`")
    parts = [f"`{source}`"]
    json_path = provenance.get("json_path")
    if json_path:
        parts.append(f"`{json_path}`")
    original = provenance.get("original_source")
    if original:
        original_path = f"`{original}`"
        original_json_path = provenance.get("original_json_path")
        original_symbol = provenance.get("original_symbol")
        if original_json_path:
            original_path = f"{original_path} `{original_json_path}`"
        if original_symbol:
            original_path = f"{original_path} `{original_symbol}`"
        parts.append(f"original {original_path}")
    return "; ".join(parts)


def render_markdown(
    summary: JsonObject, *, optimizer: str, candidate_label: str
) -> str:
    arm_a_key = "arm_a_base"
    arm_b_key = f"arm_b_{candidate_label}"
    arm_a = summary[arm_a_key]
    arm_b = summary[arm_b_key]
    stats = summary["stats"]
    ci = stats["delta_ci_95"]
    permutation = stats["sign_flip_permutation"]
    significance = summary["significance"]
    n = int(summary["n"])
    p_value = permutation.get("p_two_sided")
    verdict = (
        "CI excludes 0; headline uplift claim allowed."
        if significance["headline_claim_allowed"]
        else "CI/p gate not cleared; do not ship as a proven uplift claim."
    )
    return f"""# {optimizer.upper()}-Craftax ablation - base ReAct prompt vs {optimizer.upper()} prompt

Target quantity: **Craftax mean reward** (`reward_info.outcome_reward`). Arms are
paired by seed and differ only in `policy.config.system_prompt`; model, decode config,
container, seed set, max steps, and output-format instructions are held constant.

## Result

| Arm | Prompt | Mean reward | n |
|-----|--------|------------:|--:|
| A (without) | base ReAct | **{fmt(arm_a["mean_reward"])}** | {n} |
| B (with) | {optimizer.upper()} prompt | **{fmt(arm_b["mean_reward"])}** | {n} |

**Uplift (B - A) = {fmt(summary["paired_mean_delta"])} mean reward**, 95% CI
[{fmt(ci.get("low"))}, {fmt(ci.get("high"))}], two-sided exact sign-flip p =
{fmt(p_value, 6)}, wins/ties/losses = {summary["paired_wins"]}/{summary["paired_ties"]}/{summary["paired_losses"]}.

Honest read: {verdict}

## Provenance

- Arm A prompt: {provenance_md(summary["prompt_provenance"][arm_a_key])}
- Arm B prompt: {provenance_md(summary["prompt_provenance"][arm_b_key])}
- Container: `{summary["container"]}`; gold lane: `{summary["gold_url"]}`
- Env: `{summary["env"]}`
- Model: `{summary["model"]}`, temperature {summary["temperature"]}, max_tokens {summary["max_tokens"]}
- Seeds requested: {summary["requested_seeds"][0]}-{summary["requested_seeds"][-1]}; paired completed n={n}
- Max steps: {summary["max_steps"]}; max LLM turns: {summary["max_llm_turns"]}

## Fairness

The output-format instruction is identical across arms:

`{summary["fairness"]["output_contract"]}`

Only the strategic system-prompt content varies. Incomplete seeds are dropped from
both arms to preserve pairing.

## Artifacts

- `ablation_summary.json` - means, paired deltas, CI, permutation p, prompts, provenance
- `{{base,{candidate_label}}}-seed-*.rollout.json` - raw container rollout records
"""


def write_markdown(
    summary: JsonObject, *, optimizer: str, candidate_label: str, out_dir: Path
) -> None:
    path = out_dir.parent / f"{optimizer}_craftax_ablation.md"
    path.write_text(
        render_markdown(summary, optimizer=optimizer, candidate_label=candidate_label)
    )


def build_arm_specs(args: argparse.Namespace) -> dict[str, tuple[ArmSpec, ArmSpec]]:
    prompts = load_default_prompts(args.prompt_summary)
    base_prompt = prompts["base"]
    gepa_prompt = prompts["gepa"]
    specs: dict[str, tuple[ArmSpec, ArmSpec]] = {}
    if args.optimizer in {"gepa", "both"}:
        specs["gepa"] = (
            ArmSpec(
                "gepa", "base", "base ReAct", base_prompt.text, base_prompt.provenance
            ),
            ArmSpec(
                "gepa", "gepa", "GEPA prompt", gepa_prompt.text, gepa_prompt.provenance
            ),
        )
    if args.optimizer in {"gelo", "both"}:
        if args.gelo_prompt_file:
            gelo_prompt = prompt_from_text_file(
                args.gelo_prompt_file,
                normalize=not args.no_normalize_gelo_output_contract,
            )
        else:
            run_dirs = args.gelo_run_dir
            gelo_prompt = discover_gelo_prompt(
                run_dirs,
                allow_seed=args.allow_seed_gelo_prompt,
                normalize=not args.no_normalize_gelo_output_contract,
            )
        specs["gelo"] = (
            ArmSpec(
                "gelo", "base", "base ReAct", base_prompt.text, base_prompt.provenance
            ),
            ArmSpec(
                "gelo", "gelo", "GELO prompt", gelo_prompt.text, gelo_prompt.provenance
            ),
        )
    return specs


def run_optimizer(
    args: argparse.Namespace,
    optimizer: str,
    arms: tuple[ArmSpec, ArmSpec],
    seeds: list[int],
) -> JsonObject:
    out_dir = args.out_root / f"{optimizer}_craftax_ablation"
    out_dir.mkdir(parents=True, exist_ok=True)
    jobs: list[RolloutJob] = []
    for arm in arms:
        for seed in seeds:
            jobs.append(
                RolloutJob(
                    arm=arm,
                    seed=seed,
                    out_path=out_dir / f"{arm.arm}-seed-{seed}.rollout.json",
                    args=args,
                )
            )

    records_by_arm: dict[str, dict[int, tuple[JsonObject, Path]]] = {
        arm.arm: {} for arm in arms
    }
    started = time.time()
    completed = 0
    if args.workers == 1:
        for job in jobs:
            _, record, reused = run_rollout(job)
            completed += 1
            records_by_arm[job.arm.arm][job.seed] = (record, job.out_path)
            reward = float(record["reward_info"]["outcome_reward"])
            source = "reused" if reused else "ran"
            log(
                f"[{optimizer}] {source} {job.arm.arm} seed={job.seed} reward={reward:.3f} ({completed}/{len(jobs)})"
            )
    else:
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            futures = {pool.submit(run_rollout, job): job for job in jobs}
            for future in as_completed(futures):
                job, record, reused = future.result()
                completed += 1
                records_by_arm[job.arm.arm][job.seed] = (record, job.out_path)
                reward = float(record["reward_info"]["outcome_reward"])
                source = "reused" if reused else "ran"
                log(
                    f"[{optimizer}] {source} {job.arm.arm} seed={job.seed} reward={reward:.3f} ({completed}/{len(jobs)})"
                )

    summary = paired_summary(
        optimizer=optimizer,
        arms=arms,
        records_by_arm=records_by_arm,
        requested_seeds=seeds,
        args=args,
    )
    summary["elapsed_s"] = round(time.time() - started, 3)
    write_json(out_dir / "ablation_summary.json", summary)
    if args.write_markdown:
        write_markdown(
            summary, optimizer=optimizer, candidate_label=arms[1].arm, out_dir=out_dir
        )
    return summary


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--optimizer", choices=["gepa", "gelo", "both"], default="gepa")
    parser.add_argument(
        "--container-url",
        default=os.environ.get("CONTAINER_URL", DEFAULT_CONTAINER_URL),
    )
    parser.add_argument(
        "--gold-url", default=os.environ.get("CRAFTAX_GOLD_URL", DEFAULT_GOLD_URL)
    )
    parser.add_argument("--out-root", type=Path, default=DEFAULT_OUT_ROOT)
    parser.add_argument("--prompt-summary", type=Path, default=DEFAULT_SMOKE_SUMMARY)
    parser.add_argument("--gelo-prompt-file", type=Path)
    parser.add_argument("--gelo-run-dir", type=Path, nargs="*")
    parser.add_argument("--allow-seed-gelo-prompt", action="store_true")
    parser.add_argument("--no-normalize-gelo-output-contract", action="store_true")
    parser.add_argument("--seeds", help="Seed range like 501-564, or comma list.")
    parser.add_argument("--seed-start", type=int, default=DEFAULT_SEED_START)
    parser.add_argument("--n", type=int, default=DEFAULT_N)
    parser.add_argument("--min-paired-n", type=int, default=64)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--temperature", type=float, default=0.0)
    parser.add_argument("--max-tokens", type=int, default=512)
    parser.add_argument("--max-steps", type=int, default=64)
    parser.add_argument("--max-llm-turns", type=int, default=12)
    parser.add_argument("--min-actions-per-call", type=int, default=5)
    parser.add_argument("--max-actions-per-call", type=int, default=15)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--rollout-timeout-s", type=float, default=900.0)
    parser.add_argument("--health-timeout-s", type=float, default=120.0)
    parser.add_argument("--permutation-scale", type=int, default=1_000_000)
    parser.add_argument("--max-permutation-states", type=int, default=1_000_000)
    parser.add_argument("--tie-epsilon", type=float, default=DEFAULT_TIE_EPSILON)
    parser.add_argument("--overwrite", action="store_true")
    parser.add_argument("--skip-health", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-markdown", dest="write_markdown", action="store_false")
    parser.set_defaults(write_markdown=True)
    args = parser.parse_args(argv)
    if args.optimizer in {"gelo", "both"} and not (
        args.gelo_prompt_file or args.gelo_run_dir
    ):
        parser.error("GELO scans require --gelo-prompt-file or --gelo-run-dir")
    if args.n <= 0:
        parser.error("--n must be positive")
    if args.workers <= 0:
        parser.error("--workers must be positive")
    if args.min_paired_n < 0:
        parser.error("--min-paired-n must be >= 0")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    seeds = seeds_from_args(args)
    specs = build_arm_specs(args)
    if args.dry_run:
        log(
            f"[ablation] dry run optimizer={args.optimizer} seeds={seeds[0]}-{seeds[-1]} n={len(seeds)}"
        )
        for optimizer, arms in specs.items():
            log(f"[ablation] {optimizer}: {arms[0].arm} vs {arms[1].arm}")
            log(
                f"[ablation] {optimizer} prompt provenance: {arms[1].prompt_provenance}"
            )
        return 0
    if not args.skip_health:
        wait_for_health("craftax_gold", args.gold_url, timeout_s=args.health_timeout_s)
        wait_for_health(
            "craftax_container", args.container_url, timeout_s=args.health_timeout_s
        )

    exit_code = 0
    for optimizer, arms in specs.items():
        summary = run_optimizer(args, optimizer, arms, seeds)
        n = int(summary["n"])
        ci = summary["stats"]["delta_ci_95"]
        p_value = summary["stats"]["sign_flip_permutation"].get("p_two_sided")
        log(
            f"[{optimizer}] paired n={n} delta={fmt(summary['paired_mean_delta'])} "
            f"ci=[{fmt(ci.get('low'))}, {fmt(ci.get('high'))}] p={fmt(p_value, 6)}"
        )
        if n < args.min_paired_n:
            log(
                f"[{optimizer}] paired n below requested minimum {args.min_paired_n}; table is audit-only"
            )
            exit_code = max(exit_code, 2)
    return exit_code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
