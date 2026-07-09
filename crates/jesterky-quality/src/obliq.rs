//! **OBLIQ-Bench** — oblique (latent-pattern) retrieval eval (Tchuindjo, Shah, Khattab).
//!
//! Data: [Hugging Face `dianetc/OBLIQ-Bench`](https://huggingface.co/datasets/dianetc/OBLIQ-Bench)
//! / arXiv:2605.06235. Local layout expected under `data/obliq-bench/<task>/`:
//!
//! ```text
//! corpus.jsonl   # {_id, text}
//! queries.jsonl  # {_id, text}
//! qrels.tsv      # query-id \t corpus-id \t score
//! ```
//!
//! Default task is **math** (Math Meta-Program analogues — smallest corpus).
//!
//! Topology (`examples/obliq_math_verify.json`):
//! 1. `obliq.expand` — build gold-infused candidate pools per query
//! 2. map `obliq_reranker` — listwise LLM ranking of the pool
//! 3. `obliq.aggregate` — Recall@k / NDCG@k vs qrels
//! 4. `obliq_metrics_recorder` — headline for the panel
//!
//! This is the paper's **verification** protocol (can the model recognize latent
//! relevance when golds are in a hard pool?), not full-corpus first-stage search.

use jesterky_core::ledger::{Ledger, LedgerError};
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const OBLIQ_RERANKER: &str = "obliq_reranker";
pub const OBLIQ_RECORDER: &str = "obliq_metrics_recorder";

const DEFAULT_DATA_DIR: &str = "data/obliq-bench/math";
const DEFAULT_MAX_QUERIES: usize = 12;
const DEFAULT_POOL_SIZE: usize = 16;
const DEFAULT_K: usize = 10;
const DEFAULT_MAX_DOC_CHARS: usize = 700;
const DEFAULT_SEED: u64 = 7;

pub fn register(programs: &mut ProgramRegistry) {
    programs.register("obliq.expand", Arc::new(expand));
    programs.register("obliq.aggregate", Arc::new(aggregate));
}

pub fn roles() -> [(&'static str, &'static str); 2] {
    [
        (OBLIQ_RERANKER, OBLIQ_RERANKER_PROMPT),
        (OBLIQ_RECORDER, OBLIQ_RECORDER_PROMPT),
    ]
}

pub fn host_config() -> jesterky_contract::HostConfig {
    use jesterky_contract::{HostConfig, HostRole, HostVizConfig};
    let mut role_map = BTreeMap::new();
    for (name, prompt) in roles() {
        role_map.insert(
            name.to_string(),
            HostRole {
                prompt: Some(prompt.to_string()),
                prompt_file: None,
            },
        );
    }
    let mut output_schemas = BTreeMap::new();
    output_schemas.insert(
        OBLIQ_RERANKER.to_string(),
        "obliq_rank.schema.json".to_string(),
    );
    output_schemas.insert(
        OBLIQ_RECORDER.to_string(),
        "obliq_metrics.schema.json".to_string(),
    );
    HostConfig {
        roles: role_map,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            item_labels_op: Some("obliq.expand".to_string()),
            item_jobs_field: None,
            item_label_field: Some("query_id".to_string()),
            map_node: Some("rank_queries".to_string()),
            matrix_report_field: Some("matrix_report".to_string()),
        }),
    }
}

const OBLIQ_RERANKER_PROMPT: &str = "\
You are an IR listwise reranker for OBLIQ-Bench (latent / oblique retrieval). \
You receive one `job` with: `query_id`, `query` (natural language), and \
`candidates` (array of `{id, text}` documents, already shuffled). \
\
Relevance is often *latent*: docs may share an abstract proof strategy, \
implicit stance, or structural pattern with the query without lexical overlap. \
Rank by true latent relevance, not surface keyword match. \
\
Return ONE JSON object with exactly: \
`query_id` (echo), \
`ranked_ids` (array of candidate `id` strings, most relevant first — include \
EVERY candidate id exactly once), \
`rationale` (one short sentence on the ranking rule you used). \
No tools. No markdown. Stop after the JSON.";

const OBLIQ_RECORDER_PROMPT: &str = "\
You receive `summary` with aggregate OBLIQ metrics (`recall_at_k`, `ndcg_at_k`, \
`n_queries`, `k`, `mode`, `matrix_report`). Echo those fields unchanged. Add \
`verdict` (\"pass\" if mean_ndcg_at_k >= 0.5 else \"fail\") and a one-sentence \
`headline` summarizing model strength on oblique verification. No tools.";

/// Query ids for live viz preseed.
pub fn query_labels(
    data_dir: &str,
    max_queries: usize,
    seed: u64,
) -> Result<Vec<String>, CoreError> {
    let dir = Path::new(data_dir);
    let queries = load_queries(dir)?;
    let selected = select_query_ids(&queries, max_queries, seed);
    Ok(selected)
}

fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let data_dir = str_arg(inputs, ledger, "data_dir", DEFAULT_DATA_DIR);
    let max_queries = usize_arg(inputs, ledger, "max_queries", DEFAULT_MAX_QUERIES);
    let pool_size = usize_arg(inputs, ledger, "pool_size", DEFAULT_POOL_SIZE).max(2);
    let k = usize_arg(inputs, ledger, "k", DEFAULT_K).max(1);
    let max_doc_chars = usize_arg(inputs, ledger, "max_doc_chars", DEFAULT_MAX_DOC_CHARS).max(80);
    let seed = u64_arg(inputs, ledger, "seed", DEFAULT_SEED);
    // Modes (increasing difficulty for the *pipeline*):
    //   verify        — gold-infused pool + random distractors (easy; ceiling ~1.0)
    //   hard_verify   — 1 gold + lexical near-miss distractors (harder needle)
    //   retrieve      — pure lexical first-stage, no gold infuse (paper bottleneck)
    //   retrieve_hard — same as retrieve, but only queries with 0 golds in lexical@pool
    let mode = str_arg(inputs, ledger, "mode", "verify");
    let task = str_arg(inputs, ledger, "task", "math");
    // full = score vs all qrels (correct for pure retrieve); pool = in-pool golds only
    let metric_scope = str_arg(inputs, ledger, "metric_scope", "");

    let dir = PathBuf::from(&data_dir);
    if !dir.is_dir() {
        return Err(prog_err(format!(
            "obliq data_dir not found: {} (download OBLIQ-Bench math under data/obliq-bench/math — see docs/OBLIQ.md)",
            dir.display()
        )));
    }

    let corpus = load_corpus(&dir)?;
    let queries = load_queries(&dir)?;
    let qrels = load_qrels(&dir)?;
    let excluded = load_excluded(&dir).unwrap_or_default();

    let query_text: HashMap<String, String> = queries
        .iter()
        .map(|q| (q.id.clone(), q.text.clone()))
        .collect();
    let mut all_ids: Vec<String> = corpus.keys().cloned().collect();
    all_ids.sort();

    // Optionally restrict to hard queries (zero lexical first-stage recall).
    let candidate_qids: Vec<String> = {
        let mut ids: Vec<String> = queries.iter().map(|q| q.id.clone()).collect();
        let mut rng = XorShift64::new(seed.max(1));
        shuffle(&mut ids, &mut rng);
        if matches!(mode.as_str(), "retrieve_hard") {
            ids.retain(|qid| {
                let qtext = query_text.get(qid).map(String::as_str).unwrap_or("");
                let golds = qrels.get(qid).cloned().unwrap_or_default();
                if golds.is_empty() {
                    return false;
                }
                let block: BTreeSet<String> = excluded
                    .get(qid)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .chain(golds.iter().cloned())
                    .collect();
                let pool = build_retrieve_pool(
                    qtext, &corpus, &all_ids, &block, &golds, pool_size, seed,
                    false, // no gold infuse
                );
                let pool_set: BTreeSet<_> = pool.into_iter().collect();
                !golds.iter().any(|g| pool_set.contains(g))
            });
        }
        ids.truncate(max_queries.max(1));
        ids.sort();
        ids
    };

    let scope_default = match mode.as_str() {
        "retrieve" | "retrieve_hard" => "full",
        _ => "pool",
    };
    let metric_scope = if metric_scope.is_empty() {
        scope_default.to_string()
    } else {
        metric_scope
    };

    let mut jobs = Vec::new();
    for (i, qid) in candidate_qids.iter().enumerate() {
        let qtext = query_text.get(qid).cloned().unwrap_or_default();
        let golds: Vec<String> = qrels.get(qid).cloned().unwrap_or_default();
        if golds.is_empty() {
            continue;
        }
        let block: BTreeSet<String> = excluded
            .get(qid)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain(golds.iter().cloned())
            .collect();

        let candidates = match mode.as_str() {
            "retrieve" | "retrieve_hard" => build_retrieve_pool(
                &qtext,
                &corpus,
                &all_ids,
                &block,
                &golds,
                pool_size,
                seed.wrapping_add(i as u64),
                false, // pure first-stage — no gold infuse
            ),
            "hard_verify" => build_hard_verify_pool(
                &qtext,
                &corpus,
                &all_ids,
                &golds,
                &block,
                pool_size,
                seed.wrapping_add(i as u64),
            ),
            _ => build_verify_pool(
                &corpus,
                &all_ids,
                &golds,
                &block,
                pool_size,
                seed.wrapping_add(i as u64),
            ),
        };

        let cand_payload: Vec<Value> = candidates
            .iter()
            .filter_map(|id| {
                let text = corpus.get(id)?;
                Some(json!({
                    "id": id,
                    "text": truncate(text, max_doc_chars),
                }))
            })
            .collect();

        let cand_set: BTreeSet<String> = candidates.iter().cloned().collect();
        let pool_golds: Vec<String> = golds
            .iter()
            .filter(|g| cand_set.contains(g.as_str()))
            .cloned()
            .collect();
        // Metrics golds: full qrels for pure retrieve; in-pool only for verify*.
        let metric_golds = if metric_scope == "full" {
            golds.clone()
        } else {
            pool_golds.clone()
        };

        jobs.push(json!({
            "query_id": qid,
            "slug": qid,
            "task": task,
            "mode": mode,
            "metric_scope": metric_scope,
            "query": truncate(&qtext, max_doc_chars.saturating_mul(2).max(1200)),
            "candidates": cand_payload,
            "gold_ids": metric_golds,
            "pool_gold_count": pool_golds.len(),
            "all_gold_count": golds.len(),
            "first_stage_hit": !pool_golds.is_empty(),
            "k": k,
            "pool_size": candidates.len(),
        }));
    }

    if jobs.is_empty() {
        return Err(prog_err(format!(
            "obliq.expand produced 0 jobs from {} (mode={mode}; try mode=verify or raise max_queries)",
            dir.display()
        )));
    }

    let first_stage_hits = jobs
        .iter()
        .filter(|j| j.get("first_stage_hit").and_then(Value::as_bool) == Some(true))
        .count();

    Ok(json!({
        "jobs": jobs,
        "meta": {
            "task": task,
            "mode": mode,
            "metric_scope": metric_scope,
            "data_dir": data_dir,
            "n_queries": jobs.len(),
            "pool_size": pool_size,
            "k": k,
            "seed": seed,
            "corpus_size": corpus.len(),
            "first_stage_hits": first_stage_hits,
        }
    }))
}

fn aggregate(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let scans = inputs
        .get("scans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Pool-conditional golds: prefer job.gold_ids (in-pool only). Fallback:
    // recompute intersection of qrels ∩ ranked∪candidates so metrics stay fair
    // when the actor does not echo gold_ids.
    let data_dir = str_arg(inputs, ledger, "data_dir", DEFAULT_DATA_DIR);
    let qrels = load_qrels(Path::new(&data_dir)).unwrap_or_default();
    let k_default = usize_arg(inputs, ledger, "k", DEFAULT_K).max(1);
    let mode = str_arg(inputs, ledger, "mode", "verify");
    let task = str_arg(inputs, ledger, "task", "math");

    let mut rows = Vec::new();
    let mut sum_recall = 0.0;
    let mut sum_ndcg = 0.0;
    let mut n = 0usize;
    let mut k_used = k_default;

    for scan in &scans {
        let qid = scan
            .get("query_id")
            .or_else(|| scan.get("job").and_then(|j| j.get("query_id")))
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();

        let ranked: Vec<String> = scan
            .get("ranked_ids")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // FakeActor echoes `{job: item}` — pull ranked_ids from nested job if any.
        let ranked = if ranked.is_empty() {
            scan.get("job")
                .and_then(|j| j.get("candidates"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            ranked
        };

        let k = scan
            .get("k")
            .or_else(|| scan.get("job").and_then(|j| j.get("k")))
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(k_default)
            .max(1);
        k_used = k;

        let ranked = complete_ranking(scan, ranked);

        // Candidate universe for this item (pool).
        let mut pool_ids: BTreeSet<String> = ranked.iter().cloned().collect();
        if let Some(cands) = scan
            .get("candidates")
            .or_else(|| scan.get("job").and_then(|j| j.get("candidates")))
            .and_then(Value::as_array)
        {
            for c in cands {
                if let Some(id) = c.get("id").and_then(Value::as_str) {
                    pool_ids.insert(id.to_string());
                }
            }
        }

        let golds: Vec<String> = scan
            .get("gold_ids")
            .or_else(|| scan.get("job").and_then(|j| j.get("gold_ids")))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .filter(|g: &Vec<String>| !g.is_empty())
            .or_else(|| {
                qrels.get(&qid).map(|all| {
                    all.iter()
                        .filter(|g| pool_ids.contains(g.as_str()))
                        .cloned()
                        .collect()
                })
            })
            .unwrap_or_default();

        let gold_set: BTreeSet<String> = golds.iter().cloned().collect();
        let recall = recall_at_k(&ranked, &gold_set, k);
        let ndcg = ndcg_at_k(&ranked, &gold_set, k);
        sum_recall += recall;
        sum_ndcg += ndcg;
        n += 1;

        rows.push(json!({
            "item": qid,
            "query_id": qid,
            "n_gold": golds.len(),
            "n_ranked": ranked.len(),
            "recall_at_k": round4(recall),
            "ndcg_at_k": round4(ndcg),
            "top_ids": ranked.iter().take(k.min(5)).cloned().collect::<Vec<_>>(),
            "score": round4(ndcg * 10.0),
            "severity": if ndcg >= 0.5 { "none" } else if ndcg >= 0.2 { "medium" } else { "high" },
            "blocker": ndcg < 0.1,
            "finding": format!("R@{k}={:.2} nDCG@{k}={:.2} golds={}", recall, ndcg, golds.len()),
            "fix": if ndcg >= 0.5 { "strong latent rank" } else { "weak latent rank" },
        }));
    }

    rows.sort_by(|a, b| {
        a.get("query_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("query_id").and_then(Value::as_str).unwrap_or(""))
    });

    let mean_recall = if n > 0 { sum_recall / n as f64 } else { 0.0 };
    let mean_ndcg = if n > 0 { sum_ndcg / n as f64 } else { 0.0 };

    Ok(json!({
        "summary": {
            "task": task,
            "mode": mode,
            "n_queries": n,
            "k": k_used,
            "mean_recall_at_k": round4(mean_recall),
            "mean_ndcg_at_k": round4(mean_ndcg),
            "recall_at_k": round4(mean_recall),
            "ndcg_at_k": round4(mean_ndcg),
            "matrix_report": rows,
            "verdict": if mean_ndcg >= 0.5 { "pass" } else { "fail" },
        }
    }))
}

fn complete_ranking(scan: &Value, mut ranked: Vec<String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = ranked.iter().cloned().collect();
    if let Some(cands) = scan
        .get("candidates")
        .or_else(|| scan.get("job").and_then(|j| j.get("candidates")))
        .and_then(Value::as_array)
    {
        for c in cands {
            if let Some(id) = c.get("id").and_then(Value::as_str) {
                if seen.insert(id.to_string()) {
                    ranked.push(id.to_string());
                }
            }
        }
    }
    ranked
}

// ── pool builders ──────────────────────────────────────────────────────────

fn build_verify_pool(
    corpus: &HashMap<String, String>,
    all_ids: &[String],
    golds: &[String],
    block: &BTreeSet<String>,
    pool_size: usize,
    seed: u64,
) -> Vec<String> {
    // Fixed-size gold-infused pool with **hard distractors**.
    // Cap golds so a high-recall query cannot fill the entire pool with only
    // positives (that makes nDCG@k trivially 1.0).
    let mut rng = XorShift64::new(seed.max(1));
    let mut gold_pool: Vec<String> = golds
        .iter()
        .filter(|id| corpus.contains_key(id.as_str()))
        .cloned()
        .collect();
    shuffle(&mut gold_pool, &mut rng);
    // Keep at least ~1/4 of slots for distractors when pool_size >= 4.
    let min_distractors = if pool_size >= 4 {
        (pool_size / 4).max(2)
    } else {
        1
    };
    let gold_slots = pool_size.saturating_sub(min_distractors).max(1);
    gold_pool.truncate(gold_slots.min(gold_pool.len()).max(1));

    let mut distractors: Vec<String> = all_ids
        .iter()
        .filter(|id| !block.contains(id.as_str()) && corpus.contains_key(id.as_str()))
        .cloned()
        .collect();
    shuffle(&mut distractors, &mut rng);
    let need = pool_size.saturating_sub(gold_pool.len());
    let mut pool = gold_pool;
    pool.extend(distractors.into_iter().take(need));
    shuffle(&mut pool, &mut rng);
    pool
}

/// Lexical first-stage pool. When `gold_infuse` is false (default for
/// `retrieve` / `retrieve_hard`), this is the paper's hard bottleneck: golds
/// rarely appear in the top-k, so the reranker cannot recover them.
fn build_retrieve_pool(
    query: &str,
    corpus: &HashMap<String, String>,
    all_ids: &[String],
    block: &BTreeSet<String>,
    golds: &[String],
    pool_size: usize,
    seed: u64,
    gold_infuse: bool,
) -> Vec<String> {
    let q_tokens = tokenize(query);
    // Score the whole corpus (including golds) — first-stage must actually
    // retrieve positives; blocking golds would make pure-retrieve always fail.
    let mut scored: Vec<(f64, String)> = all_ids
        .iter()
        .filter(|id| {
            // Still respect per-query excluded ids (same-article leakage), but
            // do not block golds themselves.
            let excl_only = block.contains(id.as_str()) && !golds.iter().any(|g| g == *id);
            !excl_only
        })
        .filter_map(|id| {
            let text = corpus.get(id)?;
            let score = lexical_score(&q_tokens, text);
            Some((score, id.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut pool: Vec<String> = scored
        .into_iter()
        .take(pool_size)
        .map(|(_, id)| id)
        .collect();
    if gold_infuse {
        for g in golds {
            if !pool.contains(g) && corpus.contains_key(g) {
                if pool.len() >= pool_size {
                    pool.pop();
                }
                pool.push(g.clone());
            }
        }
    }
    let mut rng = XorShift64::new(seed.max(1));
    shuffle(&mut pool, &mut rng);
    pool
}

/// Harder verification: inject **one** gold into a pool of lexical near-miss
/// distractors (high surface overlap, not gold). Forces the model to find a
/// single latent needle among confusable math problems.
fn build_hard_verify_pool(
    query: &str,
    corpus: &HashMap<String, String>,
    all_ids: &[String],
    golds: &[String],
    block: &BTreeSet<String>,
    pool_size: usize,
    seed: u64,
) -> Vec<String> {
    let mut rng = XorShift64::new(seed.max(1));
    let mut gold_pool: Vec<String> = golds
        .iter()
        .filter(|id| corpus.contains_key(id.as_str()))
        .cloned()
        .collect();
    shuffle(&mut gold_pool, &mut rng);
    let gold = match gold_pool.first() {
        Some(g) => g.clone(),
        None => return build_verify_pool(corpus, all_ids, golds, block, pool_size, seed),
    };

    let q_tokens = tokenize(query);
    let mut near: Vec<(f64, String)> = all_ids
        .iter()
        .filter(|id| !block.contains(id.as_str()) && corpus.contains_key(id.as_str()))
        .filter_map(|id| {
            let text = corpus.get(id)?;
            let score = lexical_score(&q_tokens, text);
            if score <= 0.0 {
                return None;
            }
            Some((score, id.clone()))
        })
        .collect();
    near.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let need = pool_size.saturating_sub(1);
    let mut pool: Vec<String> = near.into_iter().take(need).map(|(_, id)| id).collect();
    // Pad with random distractors if the query is too short for near-misses.
    if pool.len() < need {
        let have: BTreeSet<_> = pool
            .iter()
            .cloned()
            .chain(std::iter::once(gold.clone()))
            .collect();
        let mut pad: Vec<String> = all_ids
            .iter()
            .filter(|id| !have.contains(id.as_str()) && !block.contains(id.as_str()))
            .cloned()
            .collect();
        shuffle(&mut pad, &mut rng);
        pool.extend(pad.into_iter().take(need - pool.len()));
    }
    pool.push(gold);
    shuffle(&mut pool, &mut rng);
    pool
}

// ── metrics ────────────────────────────────────────────────────────────────

fn recall_at_k(ranked: &[String], golds: &BTreeSet<String>, k: usize) -> f64 {
    if golds.is_empty() {
        return 0.0;
    }
    let hit = ranked
        .iter()
        .take(k)
        .filter(|id| golds.contains(id.as_str()))
        .count();
    hit as f64 / golds.len() as f64
}

fn ndcg_at_k(ranked: &[String], golds: &BTreeSet<String>, k: usize) -> f64 {
    if golds.is_empty() {
        return 0.0;
    }
    let mut dcg = 0.0;
    for (i, id) in ranked.iter().take(k).enumerate() {
        if golds.contains(id.as_str()) {
            let rel = 1.0;
            dcg += (2f64.powf(rel) - 1.0) / ((i as f64 + 2.0).log2());
        }
    }
    let ideal_hits = golds.len().min(k);
    let mut idcg = 0.0;
    for i in 0..ideal_hits {
        idcg += (2f64.powf(1.0) - 1.0) / ((i as f64 + 2.0).log2());
    }
    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

// ── IO ─────────────────────────────────────────────────────────────────────

struct QueryRec {
    id: String,
    text: String,
}

fn load_corpus(dir: &Path) -> Result<HashMap<String, String>, CoreError> {
    let path = dir.join("corpus.jsonl");
    let file = File::open(&path).map_err(|e| prog_err(format!("open {}: {e}", path.display())))?;
    let mut out = HashMap::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| prog_err(format!("read corpus line {i}: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line)
            .map_err(|e| prog_err(format!("parse corpus line {i}: {e}")))?;
        let id = v
            .get("_id")
            .or_else(|| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| prog_err(format!("corpus line {i} missing _id")))?
            .to_string();
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.insert(id, text);
    }
    Ok(out)
}

fn load_queries(dir: &Path) -> Result<Vec<QueryRec>, CoreError> {
    let path = dir.join("queries.jsonl");
    let file = File::open(&path).map_err(|e| prog_err(format!("open {}: {e}", path.display())))?;
    let mut out = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| prog_err(format!("read queries line {i}: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line)
            .map_err(|e| prog_err(format!("parse queries line {i}: {e}")))?;
        let id = v
            .get("_id")
            .or_else(|| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| prog_err(format!("queries line {i} missing _id")))?
            .to_string();
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.push(QueryRec { id, text });
    }
    Ok(out)
}

fn load_qrels(dir: &Path) -> Result<HashMap<String, Vec<String>>, CoreError> {
    let path = dir.join("qrels.tsv");
    let file = File::open(&path).map_err(|e| prog_err(format!("open {}: {e}", path.display())))?;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| prog_err(format!("read qrels line {i}: {e}")))?;
        if i == 0 && line.to_ascii_lowercase().contains("query-id") {
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = if line.contains('\t') {
            line.split('\t').collect()
        } else {
            line.split_whitespace().collect()
        };
        if parts.len() < 2 {
            continue;
        }
        // TREC: qid [iter] docid score  OR  qid docid score
        let (qid, docid) = if parts.len() >= 4 {
            (parts[0], parts[2])
        } else if parts.len() == 3 {
            (parts[0], parts[1])
        } else {
            (parts[0], parts[1])
        };
        out.entry(qid.to_string())
            .or_default()
            .push(docid.to_string());
    }
    Ok(out)
}

fn load_excluded(dir: &Path) -> Result<HashMap<String, BTreeSet<String>>, CoreError> {
    let path = dir.join("per_query_excluded_ids.json");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| prog_err(format!("read {}: {e}", path.display())))?;
    let v: Value =
        serde_json::from_str(&raw).map_err(|e| prog_err(format!("parse excluded: {e}")))?;
    let mut out = HashMap::new();
    if let Some(obj) = v.as_object() {
        for (qid, ids) in obj {
            let set = ids
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            out.insert(qid.clone(), set);
        }
    }
    Ok(out)
}

fn select_query_ids(queries: &[QueryRec], max_queries: usize, seed: u64) -> Vec<String> {
    let mut ids: Vec<String> = queries.iter().map(|q| q.id.clone()).collect();
    let mut rng = XorShift64::new(seed.max(1));
    shuffle(&mut ids, &mut rng);
    ids.truncate(max_queries.max(1));
    ids.sort(); // stable panel order
    ids
}

// ── helpers ────────────────────────────────────────────────────────────────

fn prog_err(msg: String) -> CoreError {
    CoreError::from(LedgerError::TypeMismatch(msg))
}

fn str_arg(inputs: &Value, ledger: &Ledger, key: &str, default: &str) -> String {
    inputs
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| ledger.get(key).and_then(Value::as_str))
        .unwrap_or(default)
        .to_string()
}

fn usize_arg(inputs: &Value, ledger: &Ledger, key: &str, default: usize) -> usize {
    inputs
        .get(key)
        .and_then(Value::as_u64)
        .or_else(|| ledger.get(key).and_then(Value::as_u64))
        .map(|v| v as usize)
        .unwrap_or(default)
}

fn u64_arg(inputs: &Value, ledger: &Ledger, key: &str, default: u64) -> u64 {
    inputs
        .get(key)
        .and_then(Value::as_u64)
        .or_else(|| ledger.get(key).and_then(Value::as_u64))
        .unwrap_or(default)
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn tokenize(s: &str) -> BTreeSet<String> {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .map(str::to_string)
        .collect()
}

fn lexical_score(q_tokens: &BTreeSet<String>, doc: &str) -> f64 {
    if q_tokens.is_empty() {
        return 0.0;
    }
    let d = tokenize(doc);
    let hit = q_tokens.iter().filter(|t| d.contains(t.as_str())).count();
    hit as f64 / q_tokens.len() as f64
}

fn round4(x: f64) -> f64 {
    (x * 10000.0).round() / 10000.0
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut XorShift64) {
    for i in (1..items.len()).rev() {
        let j = (rng.next_u64() as usize) % (i + 1);
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndcg_perfect_is_one() {
        let ranked = vec!["a".into(), "b".into(), "c".into()];
        let golds: BTreeSet<String> = ["a".into(), "b".into()].into_iter().collect();
        assert!((ndcg_at_k(&ranked, &golds, 2) - 1.0).abs() < 1e-9);
        assert!((recall_at_k(&ranked, &golds, 2) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ndcg_miss_is_low() {
        let ranked = vec!["x".into(), "y".into(), "a".into()];
        let golds: BTreeSet<String> = ["a".into()].into_iter().collect();
        let n = ndcg_at_k(&ranked, &golds, 3);
        assert!(n > 0.0 && n < 1.0);
    }
}
