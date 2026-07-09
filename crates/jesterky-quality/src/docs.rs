//! Synth **Mintlify docs** quality scan — one map item per MDX page under the
//! docs tree, scored against family-2 docs standards + D1–D16 audit protocol.
//! Produces a per-page matrix at reduce time.

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DOCS_AUDITOR: &str = "docs_auditor";
pub const DOCS_MATRIX_RECORDER: &str = "docs_matrix_recorder";
const PAGE_CONTEXT_BYTES: usize = 10_000;
const CONTEXT_FILE_BYTES: usize = 1_500;
const STANDARDS_CONTEXT_BYTES: usize = 5_000;
const PRODUCT_CONTEXT_BYTES: usize = 4_000;
const REFERENCE_CONTEXT_BYTES: usize = 3_000;

pub fn register(programs: &mut ProgramRegistry) {
    programs.register("docs.expand", Arc::new(expand));
    programs.register("docs.aggregate", Arc::new(aggregate));
}

/// Page slugs for live viz preseed (all MDX pages under `docs_dir`).
pub fn page_slugs(docs_dir: &str) -> Result<Vec<String>, CoreError> {
    discover_pages(Path::new(docs_dir), None)
        .map(|pages| pages.into_iter().map(|page| page.slug).collect())
}

pub fn roles() -> [(&'static str, &'static str); 2] {
    [
        (DOCS_AUDITOR, DOCS_AUDITOR_PROMPT),
        (DOCS_MATRIX_RECORDER, DOCS_MATRIX_RECORDER_PROMPT),
    ]
}

pub fn host_config() -> jesterky_contract::HostConfig {
    use jesterky_contract::{HostConfig, HostRole, HostVizConfig};
    use std::collections::BTreeMap;
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
        DOCS_AUDITOR.to_string(),
        "docs_verdict.schema.json".to_string(),
    );
    output_schemas.insert(
        DOCS_MATRIX_RECORDER.to_string(),
        "docs_matrix.schema.json".to_string(),
    );
    HostConfig {
        roles: role_map,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            item_labels_op: Some("docs.expand".to_string()),
            item_jobs_field: None,
            item_label_field: Some("slug".to_string()),
            map_node: Some("audit_pages".to_string()),
            matrix_report_field: Some("matrix_report".to_string()),
        }),
    }
}

const DOCS_AUDITOR_PROMPT: &str = "\
You are a Synth Mintlify docs auditor. You receive one `job` with page MDX plus \
bounded reference, standards, and product-spec context. Return EXACTLY ONE JSON \
object matching the schema. Keep arrays short: at most 3 items per array, only \
the highest-value findings. Required fields: `item`, `score`, `severity`, \
`blocker`, `finding`, `fix`, `violations`, `sdk_api_discrepancies`, \
`modal_bar_gaps`, `objective_standard_findings`, `product_spec_gaps`, \
`algorithm_verdict`, `page_type`, `surface`, `claim_tier`. Prefer \
`algorithm_verdict` values SOUND, FRAGILE, bogus-headline, or BOGUS; use \
inconclusive only when the supplied evidence cannot support a stronger verdict. \
\
**Runtime rule:** default `audit_mode` is `fast`. If `allow_source_tools=false`, do \
NOT run shell commands or inspect paths. Use only the supplied `page_mdx`, \
`reference_context`, `standards_context`, and `product_context`. If context is \
insufficient for a concrete SDK/API discrepancy, emit D8/D9 and name the needed \
deterministic check. Only use filesystem tools when `allow_source_tools=true`. \
\
**Audit criteria:** docs are runnable product. Check copy-paste quickstart, public \
SDK/API/reference alignment, reference-as-contract, errors with cause+action, evidence \
tier/output/artifact, freshness, machine-readable commands/config/schema, IA, consistent \
nouns/CTAs, safety/privacy, and whether the page helps the user succeed. For objective \
standards, cite both MDX `path:line` and standard `file:line`. For product coverage, \
check Optimizers, Managed Research/Tag/Factory, Stack cockpit, and Public Evidence \
against `product_context`. For SDK/API drift, require two-sided evidence; if unavailable, \
emit D8/D9 with the deterministic check needed rather than inventing D7/D13. For the \
Modal-grade bar, use normalized areas when possible: front_door, quickstart_runnability, \
expected_output, example_to_reference_bridge, reference_contract, ia_lane, next_action, \
local_remote_semantics, copy_paste_polish, other. \
\
**Axes (inform score):** A task success, B reference integrity, C information \
architecture. **Violation codes (emit one per fired rule with file:line evidence):** \
D1 quickstart step fails or missing prerequisite (blocker), D2 nonexistent/renamed \
API in example (blocker), D3 reference missing defaults/side-effects on load-bearing \
param, D4 undocumented internal prerequisite implied public, D5 stale version/date, \
D6 broken link/anchor/route/nav mismatch, D7 page drifts from public code or generated \
reference (imports, signatures, CLI flags, enum values, config keys, source links), \
D8 claim lacks evidence tier, expected output, run id, source artifact, or limitation, \
D9 page is not machine-readable enough for agents or automation (missing canonical \
command, config, schema, env vars, success condition, or structured next action), \
D10 page contradicts another public surface or reuses a noun/CTA inconsistently, \
D11 secret/privacy/destructive-command risk (raw key, token leak, unsafe rm/reset, \
private path, or credential-bearing payload), D12 rendered/frontmatter/MDX quality risk \
(bad slug, broken JSX, malformed table, missing title/description/sidebarTitle, or \
likely mobile/render overflow), D13 concrete SDK/API mismatch with two-sided evidence \
(blocker), D14 top-doc quality gap versus the Modal-grade bar on front-door/reference/ \
cookbook pages, D15 violates an objective quality standard with cited standard evidence, \
D16 omits or misstates a core product relative to product specs. \
D1/D2/D7/D11/D13/D15 → blocker=true when the evidence is concrete and user-facing. \
If a public-code drift is only suspected because this one-page audit lacks repo context, \
emit D8 or D9 instead of D7 and say what deterministic check should verify it. \
\
**Mintlify context:** pages live as `.mdx` under `docs_dir`; routing from \
`docs.json` navigation. Navbar front-door quickstart is `/prompt-optimization-gepa`. \
Default public source roots are supplied when present: `synth-ai/synth_ai` for SDK, \
selected `backend/app` API roots, and `docs/reference/sdk` generated references. \
Private standards or product specs are optional run-time inputs via \
`quality_standard_roots` / `product_spec_roots`; do not assume they exist. \
Each supplied context block starts with `FILE /absolute/path` and then `L<n>:` lines; \
cite evidence as `/absolute/path:L<n>`. Stop after the JSON object.";

const DOCS_MATRIX_RECORDER_PROMPT: &str = "\
You receive `summary` with `matrix_report` (per-page scores/violations table), \
`blockers`, `total`, `passed`, `failed`, and `violation_stats`. Echo `verdict` \
(pass if blockers==0 else fail), counts, `violation_stats`, and `matrix_report` \
unchanged in `matrix_report`. Add a one-sentence \
`headline` on overall Mintlify docs corpus quality. No tools.";

fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let docs_dir = inputs
        .get("docs_dir")
        .and_then(Value::as_str)
        .or_else(|| ledger.get("docs_dir").and_then(Value::as_str))
        .ok_or_else(|| {
            CoreError::Config(
                "docs.expand requires `docs_dir` in node inputs or run args".to_string(),
            )
        })?;
    // `docs_json` (the Mintlify nav) is optional: when not given, derive it as
    // `<docs_dir>/docs.json` so any docs location works from `docs_dir` alone —
    // no hard binding, no machine-specific absolute default.
    let docs_json = inputs
        .get("docs_json")
        .and_then(Value::as_str)
        .or_else(|| ledger.get("docs_json").and_then(Value::as_str))
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(docs_dir).join("docs.json"));
    let docs_path = Path::new(docs_dir);
    let sdk_roots =
        path_list_arg(inputs, ledger, "sdk_roots").unwrap_or_else(|| default_sdk_roots(docs_path));
    let api_roots =
        path_list_arg(inputs, ledger, "api_roots").unwrap_or_else(|| default_api_roots(docs_path));
    let reference_roots = path_list_arg(inputs, ledger, "reference_roots")
        .unwrap_or_else(|| default_reference_roots(docs_path));
    let quality_standard_roots = path_list_arg(inputs, ledger, "quality_standard_roots")
        .unwrap_or_else(|| default_quality_standard_roots(docs_path));
    let product_spec_roots = path_list_arg(inputs, ledger, "product_spec_roots")
        .unwrap_or_else(|| default_product_spec_roots(docs_path));
    let audit_mode = string_arg(inputs, ledger, "audit_mode").unwrap_or_else(|| "fast".to_string());
    let allow_source_tools =
        bool_arg(inputs, ledger, "allow_source_tools").unwrap_or_else(|| audit_mode == "full");
    let limit =
        usize_arg(inputs, ledger, "limit").or_else(|| usize_arg(inputs, ledger, "max_pages"));
    let standards_context = context_from_files(
        &quality_standard_roots,
        CONTEXT_FILE_BYTES,
        STANDARDS_CONTEXT_BYTES,
    );
    let product_context = context_from_files(
        &product_spec_roots,
        CONTEXT_FILE_BYTES,
        PRODUCT_CONTEXT_BYTES,
    );
    let pages = discover_pages(Path::new(docs_dir), Some(&docs_json))?;
    let jobs: Vec<Value> = pages
        .into_iter()
        .take(limit.unwrap_or(usize::MAX))
        .map(|page| {
            let page_mdx = line_numbered_file(Path::new(&page.path), PAGE_CONTEXT_BYTES)
                .unwrap_or_else(|| format!("unable to read {}", page.path));
            let reference_context = reference_context_for_page(
                &reference_roots,
                &page.slug,
                CONTEXT_FILE_BYTES,
                REFERENCE_CONTEXT_BYTES,
            );
            json!({
                "slug": page.slug,
                "path": page.path,
                "docs_dir": docs_dir,
                "nav_group": page.nav_group,
                "in_nav": page.in_nav,
                "audit_mode": audit_mode,
                "allow_source_tools": allow_source_tools,
                "page_mdx": page_mdx,
                "reference_context": reference_context,
                "standards_context": standards_context,
                "product_context": product_context,
                "sdk_roots": if allow_source_tools { json!(sdk_roots) } else { json!([]) },
                "api_roots": if allow_source_tools { json!(api_roots) } else { json!([]) },
                "reference_roots": if allow_source_tools { json!(reference_roots) } else { json!([]) },
                "quality_standard_roots": if allow_source_tools { json!(quality_standard_roots) } else { json!([]) },
                "product_spec_roots": if allow_source_tools { json!(product_spec_roots) } else { json!([]) },
                "dimension": page.slug,
            })
        })
        .collect();
    Ok(json!({ "jobs": jobs }))
}

fn path_list_arg(inputs: &Value, ledger: &Ledger, key: &str) -> Option<Vec<String>> {
    let value = inputs.get(key).or_else(|| ledger.get(key))?;
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Some(Vec::new());
        }
        return Some(vec![trimmed.to_string()]);
    }
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn string_arg(inputs: &Value, ledger: &Ledger, key: &str) -> Option<String> {
    inputs
        .get(key)
        .or_else(|| ledger.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn bool_arg(inputs: &Value, ledger: &Ledger, key: &str) -> Option<bool> {
    inputs
        .get(key)
        .or_else(|| ledger.get(key))
        .and_then(Value::as_bool)
}

fn usize_arg(inputs: &Value, ledger: &Ledger, key: &str) -> Option<usize> {
    let value = inputs.get(key).or_else(|| ledger.get(key))?;
    value
        .as_u64()
        .and_then(|number| usize::try_from(number).ok())
        .or_else(|| value.as_str()?.trim().parse::<usize>().ok())
}

fn existing_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .collect()
}

fn infer_workspace_root(docs_dir: &Path) -> Option<PathBuf> {
    for ancestor in docs_dir.ancestors() {
        if ancestor.join("synth-ai").exists() || ancestor.join("backend").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn default_sdk_roots(docs_dir: &Path) -> Vec<String> {
    let Some(root) = infer_workspace_root(docs_dir) else {
        return Vec::new();
    };
    existing_paths([root.join("synth-ai").join("synth_ai")])
}

fn default_api_roots(docs_dir: &Path) -> Vec<String> {
    let Some(root) = infer_workspace_root(docs_dir) else {
        return Vec::new();
    };
    existing_paths([
        root.join("backend").join("app").join("api"),
        root.join("backend").join("app").join("smr"),
        root.join("backend").join("app").join("compute_pools"),
    ])
}

fn default_reference_roots(docs_dir: &Path) -> Vec<String> {
    existing_paths([docs_dir.join("reference").join("sdk")])
}

fn default_quality_standard_roots(docs_dir: &Path) -> Vec<String> {
    existing_paths([docs_dir.join("quality").join("standards.md")])
}

fn default_product_spec_roots(docs_dir: &Path) -> Vec<String> {
    existing_paths([
        docs_dir.join("product.md"),
        docs_dir.join("product").join("index.md"),
        docs_dir.join("product").join("specs"),
    ])
}

fn line_numbered_file(path: &Path, max_bytes: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut out = format!("FILE {}\n", path.display());
    for (idx, line) in text.lines().enumerate() {
        let row = format!("L{}: {}\n", idx + 1, line);
        if out.len() + row.len() > max_bytes {
            out.push_str("L…: [truncated]\n");
            break;
        }
        out.push_str(&row);
    }
    Some(out)
}

fn context_from_files(paths: &[String], max_each: usize, max_total: usize) -> String {
    let mut out = String::new();
    for path in paths {
        if out.len() >= max_total {
            break;
        }
        let path = Path::new(path);
        if path.is_dir() {
            continue;
        }
        let Some(mut text) = line_numbered_file(path, max_each) else {
            continue;
        };
        if out.len() + text.len() > max_total {
            text.truncate(max_total.saturating_sub(out.len()));
            text.push_str("\n[context truncated]\n");
        }
        out.push_str(&text);
        out.push('\n');
    }
    out
}

fn reference_context_for_page(
    roots: &[String],
    slug: &str,
    max_each: usize,
    max_total: usize,
) -> String {
    let mut candidates = Vec::new();
    let slug_tail = slug.rsplit('/').next().unwrap_or(slug);
    let slug_norm = slug_tail.replace('-', "_");
    for root in roots {
        collect_reference_candidates(Path::new(root), slug_tail, &slug_norm, &mut candidates);
    }
    candidates.sort();
    candidates.dedup();
    let selected = candidates
        .into_iter()
        .take(4)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    context_from_files(&selected, max_each, max_total)
}

fn collect_reference_candidates(
    root: &Path,
    slug_tail: &str,
    slug_norm: &str,
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_reference_candidates(&path, slug_tail, slug_norm, out);
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let name_norm = name.replace('-', "_");
        if name == "index"
            || name.contains(slug_tail)
            || slug_tail.contains(name)
            || name_norm.contains(slug_norm)
            || slug_norm.contains(&name_norm)
        {
            out.push(path);
        }
    }
}

fn aggregate(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let scans = inputs
        .get("scans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let job_count = inputs
        .get("jobs")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(scans.len());
    let mut rows: Vec<Value> = Vec::new();
    let mut blockers = 0usize;
    let mut failed = 0usize;
    let mut total_score = 0.0f64;
    let mut violation_codes: BTreeMap<String, usize> = BTreeMap::new();
    let mut violation_severities: BTreeMap<String, usize> = BTreeMap::new();
    let mut low_score_pages: Vec<Value> = Vec::new();
    for scan in &scans {
        let Some(row) = normalize_page_verdict(scan) else {
            continue;
        };
        if row.get("blocker").and_then(Value::as_bool).unwrap_or(false) {
            blockers += 1;
        }
        let score = row
            .get("score")
            .and_then(Value::as_f64)
            .map(normalize_score)
            .unwrap_or(10.0);
        total_score += score;
        if score < 6.0 {
            failed += 1;
            low_score_pages.push(json!({
                "item": row.get("item").cloned().unwrap_or(Value::Null),
                "score": score,
                "finding": row.get("finding").cloned().unwrap_or(Value::Null),
            }));
        }
        count_violations(&row, &mut violation_codes, &mut violation_severities);
        rows.push(row);
    }
    rows.sort_by(|a, b| {
        a.get("item")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("item").and_then(Value::as_str).unwrap_or(""))
    });
    let total = rows.len();
    let passed = total.saturating_sub(failed);
    low_score_pages.sort_by(|a, b| {
        let a_score = a.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        let b_score = b.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        a_score
            .partial_cmp(&b_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.get("item")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .cmp(b.get("item").and_then(Value::as_str).unwrap_or(""))
            })
    });
    low_score_pages.truncate(10);
    let total_violations = violation_codes.values().sum::<usize>();
    let average_score = if total == 0 {
        0.0
    } else {
        total_score / total as f64
    };
    let stats = json!({
        "total_pages": job_count,
        "scanned_pages": total,
        "shard_failures": job_count.saturating_sub(total),
        "total_violations": total_violations,
        "average_score": (average_score * 10.0).round() / 10.0,
        "by_code": violation_codes,
        "by_severity": violation_severities,
        "low_score_pages": low_score_pages,
    });
    let matrix_report = render_matrix(&rows, &stats);
    let verdict = if blockers == 0 { "pass" } else { "fail" };
    Ok(json!({
        "summary": {
            "verdict": verdict,
            "total": total,
            "passed": passed,
            "failed": failed,
            "blockers": blockers,
            "violation_stats": stats,
            "pages": rows,
            "matrix_report": matrix_report,
        }
    }))
}

fn count_violations(
    row: &Value,
    by_code: &mut BTreeMap<String, usize>,
    by_severity: &mut BTreeMap<String, usize>,
) {
    let Some(violations) = row.get("violations").and_then(Value::as_array) else {
        return;
    };
    for violation in violations {
        if let Some(code) = violation.get("code").and_then(Value::as_str) {
            let code = code.trim();
            if !code.is_empty() {
                *by_code.entry(code.to_string()).or_default() += 1;
            }
        }
        if let Some(severity) = violation.get("severity").and_then(Value::as_str) {
            let severity = normalize_severity(severity);
            if severity != "none" {
                *by_severity.entry(severity).or_default() += 1;
            }
        }
    }
}

fn normalize_severity(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "crit" | "critical" | "blocker" => "critical".to_string(),
        "hi" | "high" | "error" => "high".to_string(),
        "med" | "medium" | "warn" | "warning" => "medium".to_string(),
        "lo" | "low" => "low".to_string(),
        _ => "none".to_string(),
    }
}

fn normalize_page_verdict(scan: &Value) -> Option<Value> {
    let item = scan
        .get("item")
        .or_else(|| scan.get("slug"))
        .and_then(Value::as_str)?;
    let score = numeric_value(scan.get("score"))
        .map(normalize_score)
        .unwrap_or(0.0);
    let severity = scan
        .get("severity")
        .and_then(Value::as_str)
        .map(normalize_severity)
        .unwrap_or_else(|| "none".to_string());
    let blocker = scan
        .get("blocker")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let algorithm = scan
        .get("algorithm_verdict")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let violations = scan.get("violations").cloned().unwrap_or_else(|| json!([]));
    let violation_codes = violations
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.get("code").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    Some(json!({
        "item": item,
        "score": score,
        "severity": severity,
        "blocker": blocker,
        "algorithm_verdict": algorithm,
        "violations": violations,
        "violation_codes": violation_codes,
        "finding": scan.get("finding").cloned().unwrap_or(Value::Null),
        "page_type": scan.get("page_type").cloned().unwrap_or(Value::Null),
        "surface": scan.get("surface").cloned().unwrap_or(Value::Null),
        "claim_tier": scan.get("claim_tier").cloned().unwrap_or(Value::Null),
        "sdk_api_discrepancies": scan.get("sdk_api_discrepancies").cloned().unwrap_or_else(|| json!([])),
        "modal_bar_gaps": scan.get("modal_bar_gaps").cloned().unwrap_or_else(|| json!([])),
        "objective_standard_findings": scan.get("objective_standard_findings").cloned().unwrap_or_else(|| json!([])),
        "product_spec_gaps": scan.get("product_spec_gaps").cloned().unwrap_or_else(|| json!([])),
        "sdk_api_discrepancy_count": scan.get("sdk_api_discrepancies").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "modal_bar_gap_count": scan.get("modal_bar_gaps").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "objective_standard_finding_count": scan.get("objective_standard_findings").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
        "product_spec_gap_count": scan.get("product_spec_gaps").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
    }))
}

fn numeric_value(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn normalize_score(score: f64) -> f64 {
    if score > 0.0 && score <= 1.0 {
        score * 10.0
    } else if score > 10.0 && score <= 100.0 {
        score / 10.0
    } else {
        score
    }
}

fn render_matrix(rows: &[Value], stats: &Value) -> String {
    let mut out = String::from("docs matrix — scores & violations per page\n");
    out.push_str(&format!(
        "{:<36} {:>5} {:>8} {:>14} {:>4} {:>4} {:>4} {:>4} {}\n",
        "page", "score", "severity", "algorithm", "sdk", "bar", "std", "prod", "violations"
    ));
    out.push_str(&format!("{}\n", "─".repeat(107)));
    for row in rows {
        let item = row.get("item").and_then(Value::as_str).unwrap_or("?");
        let score = row.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        let severity = row.get("severity").and_then(Value::as_str).unwrap_or("?");
        let algorithm = row
            .get("algorithm_verdict")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let sdk_count = row
            .get("sdk_api_discrepancy_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let bar_count = row
            .get("modal_bar_gap_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let standard_count = row
            .get("objective_standard_finding_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let product_count = row
            .get("product_spec_gap_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let codes = row
            .get("violation_codes")
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push_str(&format!(
            "{:<36} {:>5.1} {:>8} {:>14} {:>4} {:>4} {:>4} {:>4} {}\n",
            truncate_slug(item, 36),
            score,
            severity,
            algorithm,
            sdk_count,
            bar_count,
            standard_count,
            product_count,
            codes
        ));
    }
    out.push('\n');
    out.push_str("violation stats\n");
    out.push_str(&format!(
        "pages scanned: {}/{} · shard failures: {} · avg score: {:.1} · total violations: {}\n",
        stats
            .get("scanned_pages")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("total_pages")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("shard_failures")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("average_score")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        stats
            .get("total_violations")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    ));
    out.push_str("by code: ");
    out.push_str(&render_counts(
        stats.get("by_code").and_then(Value::as_object),
        16,
    ));
    out.push('\n');
    out.push_str("by severity: ");
    out.push_str(&render_counts(
        stats.get("by_severity").and_then(Value::as_object),
        8,
    ));
    out.push('\n');
    if let Some(pages) = stats.get("low_score_pages").and_then(Value::as_array) {
        if !pages.is_empty() {
            out.push_str("lowest scores: ");
            let parts = pages
                .iter()
                .filter_map(|page| {
                    let item = page.get("item").and_then(Value::as_str)?;
                    let score = page.get("score").and_then(Value::as_f64)?;
                    Some(format!("{} {:.1}", truncate_slug(item, 28), score))
                })
                .collect::<Vec<_>>();
            out.push_str(&parts.join(" · "));
            out.push('\n');
        }
    }
    out
}

fn render_counts(map: Option<&serde_json::Map<String, Value>>, limit: usize) -> String {
    let Some(map) = map else {
        return "none".to_string();
    };
    let mut counts = map
        .iter()
        .filter_map(|(key, value)| value.as_u64().map(|count| (key.as_str(), count)))
        .collect::<Vec<_>>();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let parts = counts
        .into_iter()
        .take(limit)
        .map(|(key, count)| format!("{key}={count}"))
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(", ")
    }
}

fn truncate_slug(slug: &str, max: usize) -> String {
    if slug.chars().count() <= max {
        slug.to_string()
    } else {
        slug.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[derive(Debug, Clone)]
struct DocPage {
    slug: String,
    path: String,
    nav_group: Option<String>,
    in_nav: bool,
}

fn discover_pages(docs_dir: &Path, docs_json: Option<&Path>) -> Result<Vec<DocPage>, CoreError> {
    if !docs_dir.is_dir() {
        return Err(CoreError::from(
            jesterky_core::ledger::LedgerError::TypeMismatch(format!(
                "docs_dir `{}` is not a directory",
                docs_dir.display()
            )),
        ));
    }
    let nav = docs_json
        .filter(|p| p.is_file())
        .map(parse_nav_pages)
        .transpose()?
        .unwrap_or_default();
    let ignore = load_mintignore(docs_dir);
    let mut pages = Vec::new();
    collect_mdx_pages(docs_dir, docs_dir, &ignore, &mut pages)?;
    pages.sort_by(|a, b| a.slug.cmp(&b.slug));
    pages.dedup_by(|a, b| a.slug == b.slug || a.path == b.path);

    for page in &mut pages {
        if let Some((group, _)) = nav.get(&page.slug) {
            page.in_nav = true;
            page.nav_group = Some(group.clone());
        }
    }
    Ok(pages)
}

fn parse_nav_pages(docs_json: &Path) -> Result<BTreeMap<String, (String, usize)>, CoreError> {
    let text = std::fs::read_to_string(docs_json).map_err(|err| {
        CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(format!(
            "read `{}`: {err}",
            docs_json.display()
        )))
    })?;
    let root: Value = serde_json::from_str(&text).map_err(|err| {
        CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(format!(
            "parse `{}`: {err}",
            docs_json.display()
        )))
    })?;
    let mut out = BTreeMap::new();
    let Some(groups) = root
        .get("navigation")
        .and_then(|nav| nav.get("groups"))
        .and_then(Value::as_array)
    else {
        return Ok(out);
    };
    for group in groups {
        let group_name = group
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("Navigation")
            .to_string();
        let Some(pages) = group.get("pages").and_then(Value::as_array) else {
            continue;
        };
        for (idx, page) in pages.iter().enumerate() {
            if let Some(slug) = page.as_str() {
                out.entry(slug.to_string())
                    .or_insert((group_name.clone(), idx));
            }
        }
    }
    Ok(out)
}

fn load_mintignore(docs_dir: &Path) -> HashSet<String> {
    let path = docs_dir.join(".mintignore");
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.trim_start_matches('/').to_string())
        .collect()
}

fn collect_mdx_pages(
    root: &Path,
    dir: &Path,
    ignore: &HashSet<String>,
    out: &mut Vec<DocPage>,
) -> Result<(), CoreError> {
    let entries = std::fs::read_dir(dir).map_err(|err| {
        CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(format!(
            "read_dir `{}`: {err}",
            dir.display()
        )))
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(
                err.to_string(),
            ))
        })?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "public" {
                continue;
            }
            collect_mdx_pages(root, &path, ignore, out)?;
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "mdx") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if should_ignore(&rel, ignore) {
                continue;
            }
            let slug = rel.strip_suffix(".mdx").unwrap_or(&rel).to_string();
            out.push(DocPage {
                slug,
                path: path.display().to_string(),
                nav_group: None,
                in_nav: false,
            });
        }
    }
    Ok(())
}

fn should_ignore(rel_path: &str, ignore: &HashSet<String>) -> bool {
    if ignore.contains(rel_path) {
        return true;
    }
    for pattern in ignore {
        if pattern.ends_with('/') && rel_path.starts_with(pattern) {
            return true;
        }
    }
    false
}
