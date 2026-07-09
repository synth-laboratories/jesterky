//! Synth OSS source-module quality scan.
//!
//! One map item is one source directory from a public OSS repo. The auditor gets
//! bounded file contents plus bounded Synth style/standards context and returns a
//! structured soundness verdict.

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const OSS_CODE_AUDITOR: &str = "oss_code_auditor";

const SOURCE_EXTS: &[&str] = &["py", "rs", "ts", "tsx", "js", "jsx", "go"];
const DEFAULT_MAX_FILES: usize = 8;
const DEFAULT_MAX_FILE_BYTES: usize = 6_000;
const DEFAULT_MIN_LOC: usize = 40;
const DEFAULT_LIMIT: usize = 36;
const DEFAULT_PER_REPO: usize = 12;
const STANDARDS_FILE_BYTES: usize = 2_000;
const STANDARDS_CONTEXT_BYTES: usize = 8_000;

const SKIP_DIRS: &[&str] = &[
    ".cargo",
    ".git",
    ".idea",
    ".mypy_cache",
    ".next",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".turbo",
    ".venv",
    ".vscode",
    "__pycache__",
    "__snapshots__",
    "build",
    "coverage",
    "dist",
    "external",
    "htmlcov",
    "node_modules",
    "site-packages",
    "target",
    "third-party",
    "third_party",
    "vendor",
    "vendored",
    "venv",
];

pub fn register(programs: &mut ProgramRegistry) {
    programs.register("oss_code.expand", Arc::new(expand));
    programs.register("oss_code.aggregate", Arc::new(aggregate));
}

pub fn roles() -> [(&'static str, &'static str); 1] {
    [(OSS_CODE_AUDITOR, OSS_CODE_AUDITOR_PROMPT)]
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
        OSS_CODE_AUDITOR.to_string(),
        "oss_code_verdict.schema.json".to_string(),
    );
    HostConfig {
        roles: role_map,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            item_labels_op: Some("oss_code.expand".to_string()),
            item_jobs_field: None,
            item_label_field: Some("module_id".to_string()),
            map_node: Some("audit_modules".to_string()),
            matrix_report_field: Some("matrix_report".to_string()),
        }),
    }
}

const OSS_CODE_AUDITOR_PROMPT: &str = "\
You are a Synth OSS code-quality auditor. You receive one `module` with bounded \
source files plus Synth engineering-soundness/style context. Return EXACTLY ONE \
JSON object matching the schema. Do NOT run tools or inspect paths beyond the \
supplied file excerpts. Keep arrays short: at most 4 violations and 6 red_flags. \
\
Audit against Synth style and standards: typed contracts at every seam; one \
correct path per operation; no fallbacks, defensive shape probing, or silent \
normalization; informative errors with auth/quota/config/transient classes; \
closed vocabularies as enums/exhaustive matches; unique nouns; earned \
abstractions; explicit state transitions; clear public module boundaries; \
docstrings/comments only where they clarify non-obvious contracts. \
\
Required scoring criteria, each 1-10: typed_seams, no_fallbacks, \
error_legibility, exhaustive_enums, unique_nouns, earned_abstraction, \
explicit_over_implicit, organization, docstrings_comments. The first three are \
load-bearing: if any is <=3 on a runtime/public seam, set hold=true and cap \
score at <=3. If the only weak seam is an intentionally dynamic JSON boundary, \
score the quality of its typed envelope, explicit validation, and error \
classification instead of demanding JSON removal. A centralized strict object \
reader at ingestion and final JSON construction from typed domain values are \
sound boundary patterns; do not call either domain `dict soup`. Deterministic \
required-null validation (for example, a strict reader method that rejects any \
non-null value) is exhaustive contract enforcement, not defensive probing or \
field skipping. A recursive typed JSON AST may contain a mapping internally \
when the external format has open-ended object keys; score its decoder and \
typed access surface rather than demanding a closed record for arbitrary JSON. \
identifier formatting is also not S7: reserve S7 for code that infers a verdict, \
kind, role, state, or authority decision by searching or parsing free text. \
Score is the profile, not a \
mean. role_tier is strict for authority/lifecycle/budget/durable state and \
standard for local tooling/examples. \
\
Use concrete evidence from supplied file snippets: every violation should cite \
`path:L<n>` when possible. Prefer normalized violation codes: S1 typed seam gap, \
S2 fallback or shape laundering, S3 vague/silent error, S4 stringly authority \
or non-exhaustive closed set, S5 noun drift, S6 unearned/duplicated abstraction, \
S7 implicit decision from strings/substrings, S8 organization/module boundary \
problem, S9 docstring/comment contract gap, S10 positive exemplar. Do not put \
`unknown` in red_flags; omit weak flags instead. Each violation must be exactly \
`{code,path,line,description,severity}` with `line` set to null when the excerpt \
does not prove one. `red_flags` is an array of short strings, never objects. \
\
Required fields: item, repo, module_id, score, severity, hold, role_tier, \
role_rationale, finding, fix, criteria, violations, red_flags. `severity` must \
be one of none|low|medium|high|critical. `role_tier` must be standard|strict. \
Never put standard, strict, blocker, or fail in a severity field. Stop after JSON.";

fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let config = scan_config(inputs, ledger)?;
    let standards_context = context_from_files(
        &config.standards_roots,
        STANDARDS_FILE_BYTES,
        STANDARDS_CONTEXT_BYTES,
    );

    let mut by_repo: BTreeMap<String, Vec<ModuleJob>> = BTreeMap::new();
    let mut matched_modules = HashSet::new();
    for repo in &config.repos {
        if !repo.root.is_dir() {
            return Err(CoreError::Config(format!(
                "repo `{}` root does not exist or is not a directory: {}",
                repo.name,
                repo.root.display()
            )));
        }
        let mut modules = discover_modules(
            &repo.name,
            &repo.root,
            config.max_files,
            config.max_file_bytes,
        )?
        .into_iter()
        .filter(|module| module.source_loc >= config.min_loc)
        .filter(|module| {
            config
                .include_modules
                .as_ref()
                .map(|included| included.contains(&module.module_id))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
        matched_modules.extend(modules.iter().map(|module| module.module_id.clone()));
        modules.sort_by(|a, b| {
            b.source_loc
                .cmp(&a.source_loc)
                .then_with(|| a.module_id.cmp(&b.module_id))
        });
        if config.per_repo > 0 {
            modules.truncate(config.per_repo);
        }
        by_repo.insert(repo.name.clone(), modules);
    }
    if let Some(included) = &config.include_modules {
        let mut missing = included
            .difference(&matched_modules)
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();
        if !missing.is_empty() {
            return Err(CoreError::Config(format!(
                "include_modules contains undiscovered module ids: {}",
                missing.join(", ")
            )));
        }
    }

    let mut jobs = Vec::new();
    let repo_names = by_repo.keys().cloned().collect::<Vec<_>>();
    let mut idx = 0usize;
    while jobs.len() < config.limit && repo_names.iter().any(|repo| idx < by_repo[repo].len()) {
        for repo in &repo_names {
            if jobs.len() >= config.limit {
                break;
            }
            if let Some(module) = by_repo.get(repo).and_then(|modules| modules.get(idx)) {
                let mut module = module.clone();
                module.standards_context = standards_context.clone();
                module.analysis_goal =
                    "audit this OSS module against Synth style and engineering soundness"
                        .to_string();
                jobs.push(module);
            }
        }
        idx += 1;
    }

    let jobs = serde_json::to_value(&jobs)
        .map_err(|err| CoreError::Json(format!("unable to serialize oss scan jobs: {err}")))?;
    Ok(json!({ "jobs": jobs }))
}

fn aggregate(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let audits = inputs
        .get("audits")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            CoreError::Config("oss_code.aggregate requires array `audits`".to_string())
        })?;
    let jobs = inputs
        .get("jobs")
        .and_then(Value::as_array)
        .ok_or_else(|| CoreError::Config("oss_code.aggregate requires array `jobs`".to_string()))?;
    let mut rows = Vec::new();
    let mut total_score = 0.0f64;
    let mut holds = 0usize;
    let mut by_repo: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_red_flag: BTreeMap<String, usize> = BTreeMap::new();
    let mut low_criteria: BTreeMap<String, usize> = BTreeMap::new();
    let mut low_score_modules = Vec::new();
    let mut malformed_audits = 0usize;

    for (index, audit) in audits.iter().enumerate() {
        let row = match serde_json::from_value::<RawAudit>(audit.clone()) {
            Ok(raw) => AuditRow::try_from(raw)?,
            Err(err) => {
                malformed_audits += 1;
                malformed_audit_row(index, jobs.get(index), &err.to_string())
            }
        };
        let score = row.score;
        total_score += score;
        if row.hold {
            holds += 1;
        }
        if score < 6.0 {
            low_score_modules.push(json!({
                "module_id": row.module_id.clone(),
                "score": score,
                "finding": row.finding.clone(),
            }));
        }
        *by_repo.entry(row.repo.clone()).or_default() += 1;
        count_red_flags(&row, &mut by_red_flag);
        count_low_criteria(&row, &mut low_criteria);
        rows.push(row);
    }

    rows.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.module_id.cmp(&b.module_id))
    });
    low_score_modules.truncate(12);
    let total = rows.len();
    let shard_failures = jobs.len().saturating_sub(total);
    let average_score = if total == 0 {
        0.0
    } else {
        total_score / total as f64
    };
    let stats = json!({
        "total_modules": jobs.len(),
        "scanned_modules": total,
        "shard_failures": shard_failures,
        "malformed_audits": malformed_audits,
        "held_modules": holds,
        "average_score": (average_score * 10.0).round() / 10.0,
        "by_repo": by_repo,
        "by_red_flag": by_red_flag,
        "low_criteria": low_criteria,
        "low_score_modules": low_score_modules,
    });
    let matrix_report = render_matrix(&rows, &stats);
    let failed = holds + shard_failures;
    let verdict = if failed == 0 { "pass" } else { "fail" };
    let modules = serde_json::to_value(&rows)
        .map_err(|err| CoreError::Json(format!("unable to serialize oss code rows: {err}")))?;
    Ok(json!({
        "summary": {
            "verdict": verdict,
            "total": jobs.len(),
            "passed": jobs.len().saturating_sub(failed),
            "failed": failed,
            "holds": holds,
            "quality_stats": stats,
            "modules": modules,
            "matrix_report": matrix_report,
        }
    }))
}

#[derive(Clone, Debug)]
struct ScanConfig {
    repos: Vec<RepoRoot>,
    max_files: usize,
    max_file_bytes: usize,
    min_loc: usize,
    limit: usize,
    per_repo: usize,
    include_modules: Option<HashSet<String>>,
    standards_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct RepoRoot {
    name: String,
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
struct ModuleJob {
    item: String,
    module_id: String,
    repo: String,
    repo_root: String,
    path: String,
    file_count: usize,
    source_loc: usize,
    truncated_files: usize,
    files: Vec<SourceFile>,
    standards_context: String,
    analysis_goal: String,
}

fn discover_modules(
    repo: &str,
    root: &Path,
    max_files: usize,
    max_file_bytes: usize,
) -> Result<Vec<ModuleJob>, CoreError> {
    let mut modules = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let skip_dirs = SKIP_DIRS.iter().copied().collect::<HashSet<_>>();
    while let Some(dir) = stack.pop() {
        let rel = dir.strip_prefix(root).unwrap_or(&dir);
        if rel
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .any(|part| skip_dirs.contains(part) || part.ends_with(".egg-info"))
        {
            continue;
        }
        let entries = std::fs::read_dir(&dir).map_err(|err| {
            CoreError::Config(format!("unable to read directory {}: {err}", dir.display()))
        })?;
        let mut source_files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|err| {
                CoreError::Config(format!(
                    "unable to read entry under {}: {err}",
                    dir.display()
                ))
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|err| {
                CoreError::Config(format!("unable to inspect {}: {err}", path.display()))
            })?;
            if file_type.is_dir() {
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    if !skip_dirs.contains(name) && !name.ends_with(".egg-info") {
                        stack.push(path);
                    }
                }
            } else if file_type.is_file() && is_source_file(&path) {
                source_files.push(path);
            }
        }
        source_files.sort();
        let meaningful = source_files.iter().any(|path| {
            !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("__init__.py" | "mod.rs")
            )
        });
        if !meaningful {
            continue;
        }

        let mut files = Vec::new();
        let mut source_loc = 0usize;
        for path in source_files.into_iter().take(max_files) {
            let Some(file) = read_source_file(root, &path, max_file_bytes) else {
                continue;
            };
            source_loc += file.loc;
            files.push(file);
        }
        if files.is_empty() {
            continue;
        }
        let rel_id = if rel.as_os_str().is_empty() {
            "(root)".to_string()
        } else {
            rel.to_string_lossy().to_string()
        };
        let truncated_files = files.iter().filter(|file| file.truncated).count();
        modules.push(ModuleJob {
            item: format!("{repo}:{rel_id}"),
            module_id: format!("{repo}:{rel_id}"),
            repo: repo.to_string(),
            repo_root: root.to_string_lossy().to_string(),
            path: rel_id,
            file_count: files.len(),
            source_loc,
            truncated_files,
            files,
            standards_context: String::new(),
            analysis_goal: String::new(),
        });
    }
    Ok(modules)
}

#[derive(Clone, Debug, Serialize)]
struct SourceFile {
    path: String,
    loc: usize,
    truncated: bool,
    code: String,
}

fn read_source_file(root: &Path, path: &Path, max_bytes: usize) -> Option<SourceFile> {
    let raw = std::fs::read(path).ok()?;
    let truncated = raw.len() > max_bytes;
    let raw = &raw[..raw.len().min(max_bytes)];
    let text = String::from_utf8_lossy(raw).to_string();
    if text.trim().is_empty() {
        return None;
    }
    let loc = text.lines().count().max(1);
    let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    let code = text
        .lines()
        .enumerate()
        .map(|(idx, line)| format!("L{}: {}", idx + 1, line))
        .collect::<Vec<_>>()
        .join("\n");
    Some(SourceFile {
        path: rel.to_string(),
        loc,
        truncated,
        code,
    })
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SOURCE_EXTS.contains(&ext))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAudit {
    item: String,
    repo: String,
    module_id: String,
    score: f64,
    severity: String,
    hold: bool,
    role_tier: String,
    role_rationale: String,
    finding: String,
    fix: String,
    criteria: CriteriaScores,
    violations: Vec<ViolationRow>,
    red_flags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CriteriaScores {
    typed_seams: f64,
    no_fallbacks: f64,
    error_legibility: f64,
    exhaustive_enums: f64,
    unique_nouns: f64,
    earned_abstraction: f64,
    explicit_over_implicit: f64,
    organization: f64,
    docstrings_comments: f64,
}

impl CriteriaScores {
    fn into_map(self) -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("typed_seams".to_string(), self.typed_seams),
            ("no_fallbacks".to_string(), self.no_fallbacks),
            ("error_legibility".to_string(), self.error_legibility),
            ("exhaustive_enums".to_string(), self.exhaustive_enums),
            ("unique_nouns".to_string(), self.unique_nouns),
            ("earned_abstraction".to_string(), self.earned_abstraction),
            (
                "explicit_over_implicit".to_string(),
                self.explicit_over_implicit,
            ),
            ("organization".to_string(), self.organization),
            ("docstrings_comments".to_string(), self.docstrings_comments),
        ])
    }
}

#[derive(Clone, Debug, Serialize)]
struct AuditRow {
    item: String,
    repo: String,
    module_id: String,
    score: f64,
    severity: String,
    hold: bool,
    role_tier: String,
    role_rationale: String,
    finding: String,
    fix: String,
    criteria: BTreeMap<String, f64>,
    violations: Vec<ViolationRow>,
    red_flags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ViolationRow {
    code: String,
    path: String,
    line: Option<u64>,
    description: String,
    severity: String,
}

impl TryFrom<RawAudit> for AuditRow {
    type Error = CoreError;

    fn try_from(raw: RawAudit) -> Result<Self, Self::Error> {
        require_score("score", raw.score)?;
        let criteria = raw.criteria.into_map();
        for (name, score) in &criteria {
            require_score(name, *score)?;
        }
        let load_bearing_hold = ["typed_seams", "no_fallbacks", "error_legibility"]
            .iter()
            .any(|name| criteria.get(*name).is_some_and(|score| *score <= 3.0));
        let hold = raw.hold || load_bearing_hold;
        let severity = calibrated_severity(raw.score, hold, Some(&raw.severity));
        Ok(Self {
            item: raw.item,
            repo: raw.repo,
            module_id: raw.module_id,
            score: raw.score,
            severity,
            hold,
            role_tier: raw.role_tier,
            role_rationale: raw.role_rationale,
            finding: raw.finding,
            fix: raw.fix,
            criteria,
            violations: raw.violations,
            red_flags: raw
                .red_flags
                .into_iter()
                .filter(|flag| !flag.trim().is_empty() && flag.trim() != "unknown")
                .collect(),
        })
    }
}

fn malformed_audit_row(index: usize, job: Option<&Value>, reason: &str) -> AuditRow {
    let item = job_string(job, "item").unwrap_or_else(|| format!("audit_index_{index}"));
    let repo = job_string(job, "repo").unwrap_or_else(|| "unattributed_repo".to_string());
    let module_id = job_string(job, "module_id").unwrap_or_else(|| item.clone());
    AuditRow {
        item,
        repo,
        module_id: module_id.clone(),
        score: 1.0,
        severity: "critical".to_string(),
        hold: true,
        role_tier: "standard".to_string(),
        role_rationale: "Auditor output failed the declared OSS audit JSON contract.".to_string(),
        finding: format!("The auditor returned malformed output for `{module_id}`: {reason}."),
        fix: "Rerun this shard or inspect the actor output; the reducer preserved it as a failed audit instead of dropping evidence.".to_string(),
        criteria: BTreeMap::from([
            ("typed_seams".to_string(), 1.0),
            ("no_fallbacks".to_string(), 1.0),
            ("error_legibility".to_string(), 1.0),
            ("exhaustive_enums".to_string(), 1.0),
            ("unique_nouns".to_string(), 1.0),
            ("earned_abstraction".to_string(), 1.0),
            ("explicit_over_implicit".to_string(), 1.0),
            ("organization".to_string(), 1.0),
            ("docstrings_comments".to_string(), 1.0),
        ]),
        violations: vec![ViolationRow {
            code: "S1".to_string(),
            path: module_id,
            line: None,
            description: format!("Auditor output did not match `oss_code_verdict.schema.json`: {reason}"),
            severity: "critical".to_string(),
        }],
        red_flags: vec!["malformed_auditor_output".to_string()],
    }
}

fn job_string(job: Option<&Value>, field: &str) -> Option<String> {
    job.and_then(|value| value.get(field))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn require_score(name: &str, score: f64) -> Result<(), CoreError> {
    if score.is_finite() && (1.0..=10.0).contains(&score) {
        Ok(())
    } else {
        Err(CoreError::Json(format!(
            "OSS audit criterion `{name}` must be a finite score from 1 through 10, got {score}"
        )))
    }
}

fn count_red_flags(row: &AuditRow, by_red_flag: &mut BTreeMap<String, usize>) {
    for flag in &row.red_flags {
        let text = flag.trim();
        if !text.is_empty() && text != "none" {
            *by_red_flag.entry(text.to_string()).or_default() += 1;
        }
    }
}

fn count_low_criteria(row: &AuditRow, low_criteria: &mut BTreeMap<String, usize>) {
    for (name, score) in &row.criteria {
        if *score <= 5.0 {
            *low_criteria.entry(name.to_string()).or_default() += 1;
        }
    }
}

fn render_matrix(rows: &[AuditRow], stats: &Value) -> String {
    let mut out = String::from("oss code matrix — Synth style / engineering soundness\n");
    out.push_str(&format!(
        "{:<34} {:>5} {:>8} {:>8} {:>6} {:>6} {:>6} {:>6} {}\n",
        "module", "score", "severity", "tier", "typed", "nofb", "errors", "hold", "red flags"
    ));
    out.push_str(
        "───────────────────────────────────────────────────────────────────────────────────────────────────────────\n",
    );
    for row in rows.iter().take(30) {
        let typed = criteria_score_field(&row.criteria, "typed_seams");
        let nofb = criteria_score_field(&row.criteria, "no_fallbacks");
        let errors = criteria_score_field(&row.criteria, "error_legibility");
        let hold = if row.hold { "YES" } else { "" };
        let flags = row
            .red_flags
            .iter()
            .filter(|flag| flag.as_str() != "none")
            .take(4)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&format!(
            "{:<34} {:>5.1} {:>8} {:>8} {:>6} {:>6} {:>6} {:>6} {}\n",
            truncate(&row.module_id, 34),
            row.score,
            truncate(&row.severity, 8),
            truncate(&row.role_tier, 8),
            typed,
            nofb,
            errors,
            hold,
            truncate(&flags, 36),
        ));
    }
    if rows.len() > 30 {
        out.push_str(&format!("… {} more modules\n", rows.len() - 30));
    }
    out.push('\n');
    out.push_str("quality stats\n");
    out.push_str(&format!(
        "modules scanned: {}/{} · shard failures: {} · malformed audits: {} · avg score: {:.1} · holds: {}\n",
        stats
            .get("scanned_modules")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("total_modules")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("shard_failures")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("malformed_audits")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        stats
            .get("average_score")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        stats
            .get("held_modules")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    ));
    out.push_str("by repo: ");
    out.push_str(&render_counts(
        stats.get("by_repo").and_then(Value::as_object),
        8,
    ));
    out.push('\n');
    out.push_str("red flags: ");
    out.push_str(&render_counts(
        stats.get("by_red_flag").and_then(Value::as_object),
        12,
    ));
    out.push('\n');
    out.push_str("low criteria: ");
    out.push_str(&render_counts(
        stats.get("low_criteria").and_then(Value::as_object),
        12,
    ));
    out.push('\n');
    out
}

fn criteria_score_field(criteria: &BTreeMap<String, f64>, name: &str) -> String {
    criteria
        .get(name)
        .map(|score| format!("{score:.0}"))
        .unwrap_or_else(|| "-".to_string())
}

fn render_counts(map: Option<&Map<String, Value>>, limit: usize) -> String {
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

fn scan_config(inputs: &Value, ledger: &Ledger) -> Result<ScanConfig, CoreError> {
    let standards_roots =
        path_list_arg(inputs, ledger, "standards_roots")?.unwrap_or_else(default_standards_roots);
    Ok(ScanConfig {
        repos: repo_roots(inputs, ledger)?,
        max_files: usize_arg(inputs, ledger, "max_files")?.unwrap_or(DEFAULT_MAX_FILES),
        max_file_bytes: usize_arg(inputs, ledger, "max_file_bytes")?
            .unwrap_or(DEFAULT_MAX_FILE_BYTES),
        min_loc: usize_arg(inputs, ledger, "min_loc")?.unwrap_or(DEFAULT_MIN_LOC),
        limit: usize_arg(inputs, ledger, "limit")?.unwrap_or(DEFAULT_LIMIT),
        per_repo: usize_arg(inputs, ledger, "per_repo")?.unwrap_or(DEFAULT_PER_REPO),
        include_modules: string_set_arg(inputs, ledger, "include_modules")?,
        standards_roots,
    })
}

fn repo_roots(inputs: &Value, ledger: &Ledger) -> Result<Vec<RepoRoot>, CoreError> {
    let Some(value) = inputs
        .get("repo_roots")
        .or_else(|| ledger.get("repo_roots"))
    else {
        return Err(CoreError::Config(
            "oss_code.expand requires explicit `repo_roots`; pass the repositories being scanned"
                .to_string(),
        ));
    };
    let roots = match value {
        Value::Object(map) => {
            let mut roots = Vec::new();
            for (name, path) in map {
                let path = path.as_str().ok_or_else(|| {
                    CoreError::Config(format!("repo_roots.{name} must be a string path"))
                })?;
                roots.push(RepoRoot {
                    name: name.clone(),
                    root: PathBuf::from(path),
                });
            }
            roots
        }
        Value::Array(items) => {
            let mut roots = Vec::new();
            for (idx, item) in items.iter().enumerate() {
                match item {
                    Value::String(path) => {
                        let root = PathBuf::from(path);
                        let name = root
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| {
                                CoreError::Config(format!(
                                    "repo_roots[{idx}] path has no repo directory name"
                                ))
                            })?
                            .to_string();
                        roots.push(RepoRoot { name, root });
                    }
                    Value::Object(map) => {
                        let name = map.get("name").and_then(Value::as_str).ok_or_else(|| {
                            CoreError::Config(format!(
                                "repo_roots[{idx}] object missing string `name`"
                            ))
                        })?;
                        let path = map.get("path").and_then(Value::as_str).ok_or_else(|| {
                            CoreError::Config(format!(
                                "repo_roots[{idx}] object missing string `path`"
                            ))
                        })?;
                        roots.push(RepoRoot {
                            name: name.to_string(),
                            root: PathBuf::from(path),
                        });
                    }
                    _ => {
                        return Err(CoreError::Config(format!(
                            "repo_roots[{idx}] must be a string path or {{name,path}} object"
                        )));
                    }
                }
            }
            roots
        }
        _ => {
            return Err(CoreError::Config(
                "repo_roots must be an object, array of paths, or array of {name,path} objects"
                    .to_string(),
            ));
        }
    };
    if roots.is_empty() {
        return Err(CoreError::Config(
            "repo_roots resolved to an empty repo list".to_string(),
        ));
    }
    Ok(roots)
}

fn default_standards_roots() -> Vec<PathBuf> {
    let Some(jstack) = std::env::var_os("JSTACK_ROOT").filter(|root| !root.is_empty()) else {
        return Vec::new();
    };
    let jstack = PathBuf::from(jstack);
    [
        jstack.join("quality/engineering_soundness.md"),
        jstack.join("style/synth_style.md"),
        jstack.join("tanha/standards/quality.md"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

fn context_from_files(paths: &[PathBuf], max_file_bytes: usize, max_total_bytes: usize) -> String {
    let mut out = String::new();
    for path in paths {
        if out.len() >= max_total_bytes {
            break;
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        out.push_str(&format!("FILE {}\n", path.display()));
        let remaining = max_total_bytes.saturating_sub(out.len());
        let take = raw.len().min(max_file_bytes).min(remaining);
        out.push_str(&take_chars_by_bytes(&raw, take));
        out.push_str("\n\n");
    }
    out
}

fn take_chars_by_bytes(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    out
}

fn path_list_arg(
    inputs: &Value,
    ledger: &Ledger,
    key: &str,
) -> Result<Option<Vec<PathBuf>>, CoreError> {
    let Some(value) = inputs.get(key).or_else(|| ledger.get(key)) else {
        return Ok(None);
    };
    match value {
        Value::String(path) => Ok(Some(vec![PathBuf::from(path)])),
        Value::Array(items) => {
            let mut paths = Vec::new();
            for (idx, item) in items.iter().enumerate() {
                let path = item.as_str().ok_or_else(|| {
                    CoreError::Config(format!("{key}[{idx}] must be a string path"))
                })?;
                paths.push(PathBuf::from(path));
            }
            Ok(Some(paths))
        }
        _ => Err(CoreError::Config(format!(
            "{key} must be a string path or array of string paths"
        ))),
    }
}

fn string_set_arg(
    inputs: &Value,
    ledger: &Ledger,
    key: &str,
) -> Result<Option<HashSet<String>>, CoreError> {
    let Some(value) = inputs.get(key).or_else(|| ledger.get(key)) else {
        return Ok(None);
    };
    let values = match value {
        Value::String(item) => vec![item.as_str()],
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                item.as_str().ok_or_else(|| {
                    CoreError::Config(format!("{key}[{idx}] must be a non-empty string"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(CoreError::Config(format!(
                "{key} must be a string or array of strings"
            )));
        }
    };
    let mut set = HashSet::new();
    for item in values {
        if item.trim().is_empty() {
            return Err(CoreError::Config(format!(
                "{key} values must be non-empty strings"
            )));
        }
        set.insert(item.to_string());
    }
    if set.is_empty() {
        return Err(CoreError::Config(format!("{key} must not be empty")));
    }
    Ok(Some(set))
}

fn usize_arg(inputs: &Value, ledger: &Ledger, key: &str) -> Result<Option<usize>, CoreError> {
    let Some(value) = inputs.get(key).or_else(|| ledger.get(key)) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(CoreError::Config(format!(
            "{key} must be an unsigned integer"
        )));
    };
    usize::try_from(value)
        .map(Some)
        .map_err(|_| CoreError::Config(format!("{key} is too large for this platform: {value}")))
}

fn calibrated_severity(score: f64, hold: bool, raw: Option<&str>) -> String {
    if hold && score <= 3.0 {
        return "critical".to_string();
    }
    if hold || score < 5.0 {
        return "high".to_string();
    }
    if score < 8.0 {
        return "medium".to_string();
    }
    let raw = raw.map(calibrate_severity_label).unwrap_or_default();
    if raw == "critical" || raw == "high" {
        "low".to_string()
    } else if raw.is_empty() || raw == "none" {
        "low".to_string()
    } else {
        raw
    }
}

fn calibrate_severity_label(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "crit" | "critical" | "blocker" => "critical".to_string(),
        "hi" | "high" | "error" => "high".to_string(),
        "med" | "medium" | "warn" | "warning" => "medium".to_string(),
        "lo" | "low" => "low".to_string(),
        _ => "none".to_string(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
