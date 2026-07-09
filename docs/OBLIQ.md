# OBLIQ-Bench on jesterky

[OBLIQ-Bench](https://arxiv.org/abs/2605.06235) (Tchuindjo, Shah, **Khattab**)
stress-tests retrieval with **oblique** queries: relevance is latent (shared
proof strategy, implicit stance, tip-of-the-tongue recollection) and current
first-stage retrievers score near zero while reasoning models can still
*verify* relevance once candidates are shown.

Data: [huggingface.co/datasets/dianetc/OBLIQ-Bench](https://huggingface.co/datasets/dianetc/OBLIQ-Bench).

## What this workflow measures

`examples/obliq_math_verify.json` runs the paper’s **verification** protocol on
the **Math Meta-Program** task (analogue queries over ~3.5k math problems):

1. Sample `max_queries` queries.
2. Build a **gold-infused candidate pool** of size `pool_size` (all golds + hard
   random distractors, shuffled).
3. Map an LLM listwise **reranker** over each pool.
4. Score mean **Recall@k** and **nDCG@k** against `qrels.tsv`.

This answers: *can DeepSeek flash recognize latent relevance when golds are
present?* — not full-corpus retrieval. Use `mode: "retrieve"` for a cheap
lexical first-stage + gold-infuse (still not dense retrieval).

## Setup data

```bash
mkdir -p data/obliq-bench/math
BASE=https://huggingface.co/datasets/dianetc/OBLIQ-Bench/resolve/main/analogues/math
curl -sL -o data/obliq-bench/math/corpus.jsonl        "$BASE/corpus/corpus.jsonl"
curl -sL -o data/obliq-bench/math/queries.jsonl       "$BASE/queries+qrels/queries.jsonl"
curl -sL -o data/obliq-bench/math/qrels.tsv           "$BASE/queries+qrels/qrels.tsv"
curl -sL -o data/obliq-bench/math/qrels_pool.tsv      "$BASE/queries+qrels/qrels_pool.tsv"
curl -sL -o data/obliq-bench/math/per_query_excluded_ids.json \
  "$BASE/queries+qrels/per_query_excluded_ids.json"
```

## Run (DeepSeek v4 flash)

```bash
export SYNTH_API_KEY=…   # SMR proxy key
# CODEX_HOME with wire_api = "responses", base_url = http://127.0.0.1:8001/api/v1

cargo run -q -p jesterky-cli -- run examples/obliq_math_verify.json \
  --actor codex \
  --model deepseek/deepseek-v4-flash-direct \
  --codex-home /tmp/jesterky_deepseek_flash_home \
  --args '{
    "data_dir": "data/obliq-bench/math",
    "max_queries": 12,
    "pool_size": 16,
    "k": 10,
    "seed": 7,
    "mode": "verify"
  }' \
  --run-id obliq-math-flash-q12 \
  --out proof/obliq_math_verify.manifest.json \
  --follow
```

### Args

| Key | Default | Meaning |
|---|---|---|
| `data_dir` | `data/obliq-bench/math` | Local task directory |
| `max_queries` | `12` | How many queries to rank |
| `pool_size` | `16` | Candidates per query |
| `k` | `10` | Cutoff for Recall@k / nDCG@k |
| `seed` | `7` | Query sample + distractor shuffle |
| `mode` | `verify` | Difficulty ladder — see below |
| `metric_scope` | auto | `pool` (verify*) or `full` (retrieve*) |
| `max_doc_chars` | `700` | Truncate doc/query text for the model |

### Difficulty ladder (`mode`)

| Mode | Pool construction | Metrics | Difficulty |
|---|---|---|---|
| `verify` | Golds + random distractors | pool-conditional | Easy (ceiling ~1.0) |
| `hard_verify` | **1 gold** + lexical near-miss distractors | pool-conditional | Harder needle |
| `retrieve` | Pure lexical top-`pool_size`, **no gold infuse** | full qrels | Hard (first-stage bottleneck) |
| `retrieve_hard` | Same as retrieve, only queries with **0** lexical@pool golds | full qrels | Hardest (all miss first-stage) |

## Typed surface

| Piece | Location |
|---|---|
| Programs | `jesterky_quality::obliq` — `obliq.expand`, `obliq.aggregate` |
| Actors | `obliq_reranker`, `obliq_metrics_recorder` |
| Schemas | `examples/obliq_rank.schema.json`, `examples/obliq_metrics.schema.json` |
| Spec | `examples/obliq_math_verify.json` |

## Other OBLIQ tasks

Same layout under `data/obliq-bench/{twitter,wildchat,writing,congress}` once
downloaded; point `data_dir` / task label accordingly. Congress is single-gold
tip-of-the-tongue; WildChat corpus is large — prefer small `max_queries`.

## Measured result (DeepSeek v4 flash)

| Setting | Value |
|---|---|
| Task | Math Meta-Program (`analogues/math`) |
| Mode | `verify` (gold-infused + hard distractors) |
| Queries | 12 (`seed=7`) |
| Pool | 16 (≤12 golds, ≥4 distractors) |
| k | 10 |
| Model | `deepseek/deepseek-v4-flash-direct` via SMR proxy |
| Wall | ~3 min |
| **mean nDCG@10** | **0.993** |
| **mean Recall@10** | **0.924** (pool-conditional golds) |

Artifact: `proof/obliq_math_verify.manifest.json`.

Interpretation matches the paper: once golds sit in a mixed pool, a reasoning
LLM ranks them near-perfectly. OBLIQ’s hardness is *surfacing* those docs from
the full corpus, not verifying them.
