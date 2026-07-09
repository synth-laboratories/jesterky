//! Craftax / container **v4 trace** annotation workloads for GEPA and GELO proposers.
//!
//! Topology: expand trace corpus → map annotate per trace → reduce theme registry →
//! recorder actor echoes the registry for downstream optimizers.

use jesterky_core::ledger::Ledger;
use jesterky_core::{CoreError, ProgramRegistry};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::sync::Arc;

pub const GEPA_TRACE_ANNOTATOR: &str = "gepa_trace_annotator";
pub const GELO_TRACE_ANNOTATOR: &str = "gelo_trace_annotator";
pub const GEPA_TRACE_RECORDER: &str = "gepa_trace_recorder";
pub const GELO_TRACE_RECORDER: &str = "gelo_trace_recorder";

const DEFAULT_TRACE_DIR: &str = "proof/craftax_v4_traces";

pub fn register(programs: &mut ProgramRegistry) {
    programs.register("trace.expand", Arc::new(expand));
    programs.register("trace.aggregate_gepa", Arc::new(aggregate_gepa));
    programs.register("trace.aggregate_gelo", Arc::new(aggregate_gelo));
    programs.register("trace.record_gepa_registry", Arc::new(record_gepa_registry));
    programs.register("trace.record_gelo_registry", Arc::new(record_gelo_registry));
}

pub fn gepa_roles() -> [(&'static str, &'static str); 2] {
    [
        (GEPA_TRACE_ANNOTATOR, GEPA_TRACE_ANNOTATOR_PROMPT),
        (GEPA_TRACE_RECORDER, GEPA_TRACE_RECORDER_PROMPT),
    ]
}

pub fn gelo_roles() -> [(&'static str, &'static str); 2] {
    [
        (GELO_TRACE_ANNOTATOR, GELO_TRACE_ANNOTATOR_PROMPT),
        (GELO_TRACE_RECORDER, GELO_TRACE_RECORDER_PROMPT),
    ]
}

pub fn gepa_host_config() -> jesterky_contract::HostConfig {
    host_config_for(
        gepa_roles(),
        "gepa_trace_verdict.schema.json",
        "trace_theme_registry.schema.json",
        "annotate_traces",
        "trace_id",
    )
}

pub fn gelo_host_config() -> jesterky_contract::HostConfig {
    host_config_for(
        gelo_roles(),
        "gelo_trace_verdict.schema.json",
        "trace_theme_registry.schema.json",
        "annotate_traces",
        "trace_id",
    )
}

fn host_config_for(
    roles: [(&'static str, &'static str); 2],
    verdict_schema: &str,
    registry_schema: &str,
    map_node: &str,
    label_field: &str,
) -> jesterky_contract::HostConfig {
    use jesterky_contract::{HostConfig, HostRole, HostVizConfig};
    use std::collections::BTreeMap;
    let mut role_map = BTreeMap::new();
    for (name, prompt) in roles {
        role_map.insert(
            name.to_string(),
            HostRole {
                prompt: Some(prompt.to_string()),
                prompt_file: None,
            },
        );
    }
    let mut output_schemas = BTreeMap::new();
    output_schemas.insert(roles[0].0.to_string(), verdict_schema.to_string());
    output_schemas.insert(roles[1].0.to_string(), registry_schema.to_string());
    HostConfig {
        roles: role_map,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            item_labels_op: Some("trace.expand".to_string()),
            item_jobs_field: None,
            item_label_field: Some(label_field.to_string()),
            map_node: Some(map_node.to_string()),
            matrix_report_field: Some("theme_matrix".to_string()),
        }),
    }
}

const GEPA_TRACE_ANNOTATOR_PROMPT: &str = "\
You annotate one Craftax GameBench rollout trace for a **GEPA proposer**. You receive \
`job` with `trace_id`, `path` (absolute v4 JSON), and `summary` (seed, reward, \
achievements, llm_turns). Read ONLY that trace file at `job.path`. Do NOT load skills, \
run shell commands, or browse other files. Return ONE JSON object: \
`trace_id`, `optimizer` (\"gepa\"), `failure_modes` ([{code, severity, evidence, \
fix_hint}]), `reusable_rules` ([{rule_id, when, then, confidence}]), \
`prompt_harness_notes` (<=40 words), `reward` (number), `achievement_count` (int), \
`theme_tags` ([string]), `severity` (none|low|medium|high|critical), `blocker` (bool). \
Focus on parse/repair loops, invalid actions, stalled resource gathering, early death, \
and achievement gaps that a prompt rewrite could fix. Stop after the JSON object.";

const GELO_TRACE_ANNOTATOR_PROMPT: &str = "\
You annotate one Craftax GameBench rollout trace for a **GELO theme explorer**. You \
receive `job` with `trace_id`, `path`, and `summary`. Read ONLY that trace file at \
`job.path`. Do NOT load skills, run shell commands, or browse other files. \
Return ONE JSON object: `trace_id`, `optimizer` (\"gelo\"), `exploration_themes` \
([{theme, saturation, evidence}]), `underexplored_actions` ([string]), \
`behavioral_diversity_score` (0-1), `reward` (number), `achievement_count` (int), \
`theme_tags` ([string]), `severity` (none|low|medium|high|critical), `blocker` (bool). \
Focus on action diversity, repeated noop/idle patterns, map coverage, and theme \
saturation — not prompt wording. Stop after the JSON object.";

const GEPA_TRACE_RECORDER_PROMPT: &str = "\
You receive `summary` with `theme_registry` (aggregated GEPA trace annotations), \
`blockers`, `total`, `annotated`. Echo `verdict` (pass if blockers==0 else fail), \
counts, and `theme_registry` unchanged. Add a one-sentence `headline` on what the \
proposer should try next. No tools.";

const GELO_TRACE_RECORDER_PROMPT: &str = "\
You receive `summary` with `theme_registry` (aggregated GELO trace annotations), \
`blockers`, `total`, `annotated`. Echo `verdict`, counts, and `theme_registry`. Add \
a one-sentence `headline` on exploration gaps. No tools.";

fn expand(ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    let trace_dir = inputs
        .get("trace_dir")
        .and_then(Value::as_str)
        .or_else(|| ledger.get("trace_dir").and_then(Value::as_str))
        .unwrap_or(DEFAULT_TRACE_DIR);
    let root = Path::new(trace_dir);
    if !root.is_dir() {
        return Err(CoreError::from(
            jesterky_core::ledger::LedgerError::TypeMismatch(format!(
                "trace_dir `{trace_dir}` is not a directory"
            )),
        ));
    }
    let mut jobs = Vec::new();
    for entry in fs::read_dir(root).map_err(|err| {
        CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(format!(
            "read_dir `{trace_dir}`: {err}"
        )))
    })? {
        let entry = entry.map_err(|err| {
            CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(
                err.to_string(),
            ))
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !name.ends_with(".v4.json") {
            continue;
        }
        let trace_id = name
            .strip_suffix(".v4.json")
            .or_else(|| name.strip_suffix(".json"))
            .unwrap_or(name)
            .to_string();
        let summary = trace_summary(&path)?;
        jobs.push(json!({
            "trace_id": trace_id,
            "path": path_to_string(&path),
            "trace_dir": trace_dir,
            "summary": summary,
            "dimension": trace_id,
        }));
    }
    jobs.sort_by(|a, b| {
        a.get("trace_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("trace_id").and_then(Value::as_str).unwrap_or(""))
    });
    Ok(json!({ "jobs": jobs }))
}

fn trace_summary(path: &Path) -> Result<Value, CoreError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(format!(
            "read `{}`: {err}",
            path.display()
        )))
    })?;
    let doc: Value = serde_json::from_str(&raw).map_err(|err| {
        CoreError::from(jesterky_core::ledger::LedgerError::TypeMismatch(format!(
            "parse `{}`: {err}",
            path.display()
        )))
    })?;
    let summary = doc.get("summary").cloned().unwrap_or(json!({}));
    let metadata = doc.get("metadata").cloned().unwrap_or(json!({}));
    let seed = summary
        .get("seed")
        .or_else(|| metadata.get("seed"))
        .cloned()
        .unwrap_or(Value::Null);
    let reward = summary
        .get("outcome_reward")
        .or_else(|| summary.get("reward"))
        .cloned()
        .unwrap_or(Value::Null);
    let achievements = summary
        .get("achievements")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    let llm_turns = doc
        .get("span_count")
        .or_else(|| summary.get("llm_turns"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "seed": seed,
        "reward": reward,
        "achievement_count": achievements,
        "llm_turns": llm_turns,
        "status": doc.get("status").cloned().unwrap_or(Value::Null),
    }))
}

fn path_to_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn aggregate_trace_scans(inputs: &Value, optimizer: &str) -> Result<Value, CoreError> {
    let scans = inputs
        .get("scans")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut blockers = 0usize;
    let mut theme_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut rows = Vec::new();
    for scan in &scans {
        let trace_id = scan.get("trace_id").and_then(Value::as_str).unwrap_or("-");
        let blocker = scan
            .get("blocker")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if blocker {
            blockers += 1;
        }
        for tag in scan
            .get("theme_tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            *theme_counts.entry(tag.to_string()).or_default() += 1;
        }
        rows.push(json!({
            "trace_id": trace_id,
            "optimizer": scan.get("optimizer").cloned().unwrap_or(json!(optimizer)),
            "reward": scan.get("reward"),
            "achievement_count": scan.get("achievement_count"),
            "severity": scan.get("severity").and_then(Value::as_str).unwrap_or("none"),
            "blocker": blocker,
            "theme_tags": scan.get("theme_tags").cloned().unwrap_or(json!([])),
        }));
    }
    let themes: Vec<Value> = theme_counts
        .into_iter()
        .map(|(theme, count)| json!({ "theme": theme, "count": count }))
        .collect();
    let verdict = if blockers == 0 { "pass" } else { "fail" };
    Ok(json!({
        "summary": {
            "optimizer": optimizer,
            "total": scans.len(),
            "annotated": scans.len(),
            "blockers": blockers,
            "verdict": verdict,
            "theme_matrix": rows,
            "theme_registry": {
                "optimizer": optimizer,
                "themes": themes,
                "traces": rows,
            },
        }
    }))
}

fn aggregate_gepa(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    aggregate_trace_scans(inputs, "gepa")
}

fn aggregate_gelo(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    aggregate_trace_scans(inputs, "gelo")
}

fn record_gepa_registry(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    record_registry(inputs, "gepa")
}

fn record_gelo_registry(_ledger: &Ledger, inputs: &Value) -> Result<Value, CoreError> {
    record_registry(inputs, "gelo")
}

fn record_registry(inputs: &Value, optimizer: &str) -> Result<Value, CoreError> {
    let summary = inputs.get("summary").cloned().unwrap_or_else(|| json!({}));
    let total = summary.get("total").and_then(Value::as_u64).unwrap_or(0);
    let blockers = summary.get("blockers").and_then(Value::as_u64).unwrap_or(0);
    let verdict = summary
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or(if blockers == 0 { "pass" } else { "fail" });
    let headline = format!(
        "{optimizer} trace registry: {total} annotated traces, {blockers} blockers ({verdict})"
    );
    let mut theme_registry = summary
        .get("theme_registry")
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = theme_registry.as_object_mut() {
        object.insert("headline".to_string(), json!(headline));
        object
            .entry("optimizer")
            .or_insert_with(|| json!(optimizer));
    } else {
        theme_registry = json!({
            "optimizer": optimizer,
            "themes": [],
            "traces": summary.get("theme_matrix").cloned().unwrap_or(json!([])),
            "headline": headline,
        });
    }
    // Optional sibling artifact dir for optimizer hooks (GELO Arm B materialize).
    // When ledger/args provide artifact_dir, write stable files next to the run.
    if let Some(dir) = inputs
        .get("artifact_dir")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let _ = write_optimizer_sidecar_artifacts(Path::new(dir), &theme_registry, &headline);
    }
    Ok(json!({
        "verdict": verdict,
        "total": total,
        "annotated": summary.get("annotated").cloned().unwrap_or(json!(total)),
        "blockers": blockers,
        "headline": headline,
        "theme_registry": theme_registry,
    }))
}

fn write_optimizer_sidecar_artifacts(
    dir: &Path,
    theme_registry: &Value,
    headline: &str,
) -> Result<(), CoreError> {
    fs::create_dir_all(dir)
        .map_err(|err| CoreError::Config(format!("mkdir {}: {err}", dir.display())))?;
    let registry_path = dir.join("jesterky_theme_registry.json");
    fs::write(
        &registry_path,
        serde_json::to_string_pretty(theme_registry).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|err| CoreError::Config(format!("write {}: {err}", registry_path.display())))?;

    let annotations_path = dir.join("jesterky_trace_annotations.jsonl");
    let mut lines = String::new();
    if let Some(traces) = theme_registry
        .get("traces")
        .or_else(|| theme_registry.get("theme_matrix"))
        .and_then(Value::as_array)
    {
        for row in traces {
            if let Ok(line) = serde_json::to_string(row) {
                lines.push_str(&line);
                lines.push('\n');
            }
        }
    }
    fs::write(&annotations_path, lines)
        .map_err(|err| CoreError::Config(format!("write {}: {err}", annotations_path.display())))?;

    let mut context = String::new();
    context.push_str("# jesterky proposer context\n\n");
    context.push_str(headline);
    context.push_str("\n\n");
    if let Some(themes) = theme_registry.get("themes").and_then(Value::as_array) {
        context.push_str("## Themes\n");
        for theme in themes {
            let name = theme
                .get("theme")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let count = theme.get("count").and_then(Value::as_u64).unwrap_or(0);
            context.push_str(&format!("- {name} (count={count})\n"));
        }
        context.push('\n');
    }
    context.push_str(
        "Use these themes and annotations as wall-safe evidence when proposing. \
         Cite theme names and trace_ids; do not invent heldout labels.\n",
    );
    let context_path = dir.join("jesterky_proposer_context.md");
    fs::write(&context_path, context)
        .map_err(|err| CoreError::Config(format!("write {}: {err}", context_path.display())))?;
    Ok(())
}
