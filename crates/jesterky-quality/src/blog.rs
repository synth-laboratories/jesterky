//! Synth **blog** quality scan — one map item per published post, scored against
//! the blog strategy handoff + audit protocol (V1–V11, algorithm verdict, family-1
//! rubrics). Produces a per-post matrix at reduce time.

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub const BLOG_AUDITOR: &str = "blog_auditor";
pub const BLOG_MATRIX_RECORDER: &str = "blog_matrix_recorder";

pub fn register(programs: &mut ProgramRegistry) {
    programs.register("blog.expand", Arc::new(expand));
    programs.register("blog.aggregate", Arc::new(aggregate));
}

/// Published post slugs under `blog_dir` (draft:true omitted) for live viz preseed.
pub fn published_slugs(blog_dir: &str) -> Result<Vec<String>, CoreError> {
    discover_published_posts(Path::new(blog_dir))
        .map(|posts| posts.into_iter().map(|(slug, _)| slug).collect())
}

pub fn roles() -> [(&'static str, &'static str); 2] {
    [
        (BLOG_AUDITOR, BLOG_AUDITOR_PROMPT),
        (BLOG_MATRIX_RECORDER, BLOG_MATRIX_RECORDER_PROMPT),
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
        BLOG_AUDITOR.to_string(),
        "blog_verdict.schema.json".to_string(),
    );
    output_schemas.insert(
        BLOG_MATRIX_RECORDER.to_string(),
        "blog_matrix.schema.json".to_string(),
    );
    HostConfig {
        roles: role_map,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            item_labels_op: Some("blog.expand".to_string()),
            item_jobs_field: None,
            item_label_field: Some("slug".to_string()),
            map_node: Some("audit_posts".to_string()),
            matrix_report_field: Some("matrix_report".to_string()),
        }),
    }
}

const BLOG_AUDITOR_PROMPT: &str = "\
You are a Synth blog auditor. You receive one `job` with `slug`, `path` (absolute \
MDX), and `blog_dir`. Read ONLY that MDX file plus the rubric context below. \
Return ONE JSON object (not wrapped in verdicts[]) with: `item` (slug), `score` \
(1–10), `severity` (none|low|medium|high|critical), `blocker` (bool), `finding` \
(<=22 words), `fix` (<=15 words), `violations` ([{code, severity, note}]), \
`algorithm_verdict` (SOUND|FRAGILE|bogus-headline|BOGUS), `internal_type` \
(launch_post|feature_release|research_post|engineering_essay|tutorial|case_study|\
company_post|evergreen_article|…), `surface` (blog|changelog|docs|resources), \
`claim_tier` (measured|dev_evidence|smoke|roadmap|unknown). \
\
**Synth blog strategy (2026-07-08 handoff):** External Blog subtypes: Product, \
Engineering, Research, Tutorials, Case Studies, Company. Internal routing types \
include launch_post, feature_release, product_update, release_note, research_post, \
benchmark_report, engineering_essay, tutorial, cookbook, case_study, company_post. \
Required frontmatter/metadata to check: type, surface, status, owner, claim_tier, \
cta, release, proof, product_area, audience, date, version, canonical_url. \
Launch → launch_post + release_note + docs + proof; feature_release → release_note \
+ docs if API changes; technical claims → research_post + proof_page. \
\
**Gates:** blog_post_quality_checklist (thesis, limitation, CTA, claim tiers, \
rendered craft), blog_audit_protocol V1–V11 (V1 number w/o baseline=HIGH, V2 no \
limitations=HIGH, V3 no repro trail=HIGH, V4 fabricated figures=HIGH, V5–V11 per \
protocol), family-1 anchored rubrics (claim custody, negative-results honesty, \
eval independence, reproducibility trail, mechanism depth). \
\
Emit ONE violation per fired V-code with evidence. Score reflects family-1 profile. \
Stop after the JSON object.";

const BLOG_MATRIX_RECORDER_PROMPT: &str = "\
You receive `summary` with `matrix_report` (per-post scores/violations table), \
`blockers`, `total`, `passed`, `failed`. Echo `verdict` (pass if blockers==0 else \
fail), counts, and `matrix_report` unchanged in `matrix_report`. Add a one-sentence \
`headline` on overall blog corpus quality. No tools.";

fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let blog_dir = inputs
        .get("blog_dir")
        .and_then(Value::as_str)
        .or_else(|| ledger.get("blog_dir").and_then(Value::as_str))
        .ok_or_else(|| {
            CoreError::Config(
                "blog.expand requires `blog_dir` in node inputs or run args".to_string(),
            )
        })?;
    let posts = discover_published_posts(Path::new(blog_dir))?;
    let jobs: Vec<Value> = posts
        .into_iter()
        .map(|(slug, path)| {
            json!({
                "slug": slug,
                "path": path,
                "blog_dir": blog_dir,
                "dimension": slug,
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
        let Some(row) = normalize_post_verdict(scan) else {
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
            "posts": rows,
            "matrix_report": matrix_report,
        }
    }))
}

fn normalize_post_verdict(scan: &Value) -> Option<Value> {
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
        "internal_type": scan.get("internal_type").cloned().unwrap_or(Value::Null),
        "surface": scan.get("surface").cloned().unwrap_or(Value::Null),
        "claim_tier": scan.get("claim_tier").cloned().unwrap_or(Value::Null),
    }))
}

fn render_matrix(rows: &[Value]) -> String {
    let mut out = String::from("blog matrix — scores & violations per post\n");
    out.push_str(&format!(
        "{:<28} {:>5} {:>8} {:>14} {}\n",
        "post", "score", "severity", "algorithm", "violations"
    ));
    out.push_str(&format!("{}\n", "─".repeat(76)));
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
            "{:<28} {:>5.1} {:>8} {:>14} {}\n",
            truncate_slug(item, 28),
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

fn discover_published_posts(blog_dir: &Path) -> Result<Vec<(String, String)>, CoreError> {
    if !blog_dir.is_dir() {
        return Err(CoreError::from(
            jesterky_core::ledger::LedgerError::TypeMismatch(format!(
                "blog_dir `{}` is not a directory",
                blog_dir.display()
            )),
        ));
    }
    let mut posts = Vec::new();
    collect_posts(blog_dir, blog_dir, &mut posts)?;
    posts.sort_by(|a, b| a.0.cmp(&b.0));
    posts.dedup_by(|a, b| a.0 == b.0 || a.1 == b.1);
    Ok(posts)
}

fn collect_posts(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
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
            let index = path.join("index.mdx");
            if index.is_file() && !is_draft_mdx(&index) {
                if let Some(slug) = path.file_name().and_then(|s| s.to_str()) {
                    out.push((slug.to_string(), index.display().to_string()));
                }
            }
            collect_posts(root, &path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "mdx") {
            // `subdir/index.mdx` is indexed by the parent directory above — skip
            // the bare `index` stem so we never emit a duplicate `index` job.
            if path.file_stem().is_some_and(|stem| stem == "index") {
                continue;
            }
            if !is_draft_mdx(&path) {
                if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
                    out.push((slug.to_string(), path.display().to_string()));
                }
            }
        }
    }
    Ok(())
}

fn is_draft_mdx(path: &Path) -> bool {
    let Ok(head) = std::fs::read_to_string(path) else {
        return true;
    };
    let sample = head.chars().take(4096).collect::<String>();
    sample
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("draft: true") || line.contains("draft: true"))
}
