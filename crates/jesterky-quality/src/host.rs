//! Default host wiring for reference quality workloads (prompts, schemas, viz).

use crate::{blog, docs, dungeongrid, obliq, roles, trace, SCANNER_ACTOR, SUMMARY_ACTOR};
use jesterky_contract::{HostConfig, HostRole, HostVizConfig};
use std::collections::BTreeMap;

/// Built-in host config for a reference workload spec name, when the spec file
/// does not embed its own `host` block.
pub fn host_config(name: &str) -> Option<HostConfig> {
    match name {
        "quality_scan" => Some(code_scan_host()),
        "quality_scan_blogs" => Some(blog::host_config()),
        "quality_scan_docs" => Some(docs::host_config()),
        "gepa_trace_annotate" => Some(trace::gepa_host_config()),
        "gelo_trace_annotate" => Some(trace::gelo_host_config()),
        "dungeongrid_4p" | "dungeongrid" => Some(dungeongrid::host_config()),
        "obliq_math_verify" | "obliq_math" => Some(obliq::host_config()),
        _ => None,
    }
}

fn code_scan_host() -> HostConfig {
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
        SCANNER_ACTOR.to_string(),
        "quality_verdict.schema.json".to_string(),
    );
    output_schemas.insert(
        SUMMARY_ACTOR.to_string(),
        "quality_summary.schema.json".to_string(),
    );
    HostConfig {
        roles: role_map,
        output_schemas,
        sandboxes: Default::default(),
        viz: Some(HostVizConfig {
            item_labels_op: Some("quality.expand".to_string()),
            item_jobs_field: None,
            item_label_field: Some("dimension".to_string()),
            map_node: Some("scan_jobs".to_string()),
            matrix_report_field: None,
        }),
    }
}
