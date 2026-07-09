//! Synth **Mintlify docs** quality scan — one map item per MDX page under the
//! docs tree, scored against family-2 docs standards + D1–D12 audit protocol.
//! Produces a per-page matrix at reduce time.

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DOCS_AUDITOR: &str = "docs_auditor";
pub const DOCS_MATRIX_RECORDER: &str = "docs_matrix_recorder";

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
You are a Synth Mintlify docs auditor. You receive one `job` with `slug`, `path` \
(absolute MDX), `docs_dir`, optional `nav_group`, and `in_nav` (bool). Read ONLY \
that MDX file plus the rubric below. Return ONE JSON object with: `item` (slug), \
`score` (1–10), `severity` (none|low|medium|high|critical), `blocker` (bool), \
`finding` (<=22 words), `fix` (<=15 words), `violations` ([{code, severity, note}]), \
`algorithm_verdict` (SOUND|FRAGILE|bogus-headline|BOGUS), `page_type` \
(quickstart|reference|concept|guide|cookbook|changelog|error|overview|other), \
`surface` (docs), `claim_tier` (measured|dev_evidence|smoke|roadmap|unknown). \
\
**Family-2 docs criteria (Synth docs standards):** docs are runnable product, \
not prose. Check quickstart executability (install→auth→first run \
copy-pasteable, prerequisites explicit, expected output), public-code alignment \
(imports, symbols, CLI flags, config names, enum values, generated reference, \
source links), reference-as-contract (type/required/default/constraints/Raises \
per field), error documentation (cause + action, per-method Raises), evidence \
discipline (claim tier, output, run/source artifact), freshness signals \
(stackVersion|last_verified|updated on page), machine-readability (canonical \
commands/config/schema/next action), information architecture (front door, nav \
placement, next step), cross-surface consistency (same noun/claim/CTA means the \
same thing everywhere), safety/privacy (no raw secrets, unsafe destructive \
commands, private/internal prerequisites as public path), and \
instrument-vs-document (the page should help a user succeed, not merely describe \
the product). \
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
likely mobile/render overflow). D1/D2/D7/D11 → blocker=true when the evidence is concrete. \
If a public-code drift is only suspected because this one-page audit lacks repo context, \
emit D8 or D9 instead of D7 and say what deterministic check should verify it. \
\
**Mintlify context:** pages live as `.mdx` under `docs_dir`; routing from \
`docs.json` navigation. Navbar front-door quickstart is `/prompt-optimization-gepa`. \
Cite absolute `path:line` in violation notes. Stop after the JSON object.";

const DOCS_MATRIX_RECORDER_PROMPT: &str = "\
You receive `summary` with `matrix_report` (per-page scores/violations table), \
`blockers`, `total`, `passed`, `failed`. Echo `verdict` (pass if blockers==0 else \
fail), counts, and `matrix_report` unchanged in `matrix_report`. Add a one-sentence \
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
    let pages = discover_pages(Path::new(docs_dir), Some(&docs_json))?;
    let jobs: Vec<Value> = pages
        .into_iter()
        .map(|page| {
            json!({
                "slug": page.slug,
                "path": page.path,
                "docs_dir": docs_dir,
                "nav_group": page.nav_group,
                "in_nav": page.in_nav,
                "dimension": page.slug,
            })
        })
        .collect();
    Ok(json!({ "jobs": jobs }))
}

fn aggregate(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let scans = inputs
        .get("scans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut rows: Vec<Value> = Vec::new();
    let mut blockers = 0usize;
    let mut failed = 0usize;
    for scan in &scans {
        let Some(row) = normalize_page_verdict(scan) else {
            continue;
        };
        if row.get("blocker").and_then(Value::as_bool).unwrap_or(false) {
            blockers += 1;
        }
        if row.get("score").and_then(Value::as_f64).unwrap_or(10.0) < 6.0 {
            failed += 1;
        }
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
    let matrix_report = render_matrix(&rows);
    let verdict = if blockers == 0 { "pass" } else { "fail" };
    Ok(json!({
        "summary": {
            "verdict": verdict,
            "total": total,
            "passed": passed,
            "failed": failed,
            "blockers": blockers,
            "pages": rows,
            "matrix_report": matrix_report,
        }
    }))
}

fn normalize_page_verdict(scan: &Value) -> Option<Value> {
    let item = scan
        .get("item")
        .or_else(|| scan.get("slug"))
        .and_then(Value::as_str)?;
    let score = scan.get("score").and_then(Value::as_f64).unwrap_or(0.0);
    let severity = scan
        .get("severity")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
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
    }))
}

fn render_matrix(rows: &[Value]) -> String {
    let mut out = String::from("docs matrix — scores & violations per page\n");
    out.push_str(&format!(
        "{:<36} {:>5} {:>8} {:>14} {}\n",
        "page", "score", "severity", "algorithm", "violations"
    ));
    out.push_str(&format!("{}\n", "─".repeat(84)));
    for row in rows {
        let item = row.get("item").and_then(Value::as_str).unwrap_or("?");
        let score = row.get("score").and_then(Value::as_f64).unwrap_or(0.0);
        let severity = row.get("severity").and_then(Value::as_str).unwrap_or("?");
        let algorithm = row
            .get("algorithm_verdict")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let codes = row
            .get("violation_codes")
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push_str(&format!(
            "{:<36} {:>5.1} {:>8} {:>14} {}\n",
            truncate_slug(item, 36),
            score,
            severity,
            algorithm,
            codes
        ));
    }
    out
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
