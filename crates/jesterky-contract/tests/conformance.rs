use jesterky_contract::{RunManifest, WorkflowSpec};
use jsonschema::JSONSchema;
use serde_json::json;
use std::path::Path;

#[test]
fn checked_in_workflow_examples_conform_to_schema() {
    let root = workspace_root();
    let schema = read_json(root.join("jesterky.schema.json"));

    for fixture in ["quality_min.json", "quality_scan.json"] {
        let instance = read_json(root.join("examples").join(fixture));
        assert_schema_valid(&schema, &instance);
        let workflow: WorkflowSpec =
            serde_json::from_value(instance).expect("workflow fixture deserializes");
        workflow
            .validate_and_hash()
            .unwrap_or_else(|err| panic!("{fixture} validates and hashes: {err}"));
    }
}

#[test]
fn sample_run_manifest_conforms_to_schema_and_deserializes() {
    let root = workspace_root();
    let schema = read_json(root.join("jesterky.manifest.schema.json"));
    let instance = sample_manifest_json();

    assert_schema_valid(&schema, &instance);
    let manifest: RunManifest =
        serde_json::from_value(instance).expect("manifest fixture deserializes");
    assert_eq!(manifest.run_id, "conformance-run");
    assert_eq!(manifest.events.len(), 3);
    assert_eq!(manifest.recorded.len(), 1);
}

fn assert_schema_valid(schema: &serde_json::Value, instance: &serde_json::Value) {
    let compiled = JSONSchema::compile(schema).expect("schema compiles");
    let messages = match compiled.validate(instance) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.map(|error| error.to_string()).collect::<Vec<_>>(),
    };
    if !messages.is_empty() {
        panic!("schema validation failed:\n{}", messages.join("\n"));
    }
}

fn sample_manifest_json() -> serde_json::Value {
    json!({
        "run_id": "conformance-run",
        "workflow_name": "quality_min",
        "spec_hash": "2fa9c50d3d9b3290acb15da27ceae519ce9f823c09ded95f270690b2c668d3bf",
        "args": {},
        "events": [
            {
                "addr": addr(&[], 0),
                "kind": { "kind": "workflow_started" },
                "payload": {},
                "wall_ms": 0
            },
            {
                "addr": addr(&["scan"], 0),
                "kind": { "kind": "node_started" },
                "payload": null,
                "wall_ms": 1
            },
            {
                "addr": addr(&["scan"], 1),
                "kind": { "kind": "actor_invoked" },
                "payload": { "actor": "quality_scanner" },
                "wall_ms": 2
            }
        ],
        "recorded": [
            {
                "addr": addr(&["scan"], 1),
                "call": {
                    "call": "actor",
                    "actor": "quality_scanner"
                },
                "outputs": {
                    "target": "alpha",
                    "rubric": "minimal quality pass"
                },
                "score": null,
                "signal": null,
                "artifacts": []
            }
        ],
        "checkpoints": [],
        "trace": {
            "addr": addr(&[], 0),
            "label": "workflow:quality_min",
            "inputs": {},
            "outputs": null,
            "score": null,
            "signal": null,
            "artifacts": [],
            "children": [
                {
                    "addr": addr(&["scan"], 0),
                    "label": "scan",
                    "inputs": null,
                    "outputs": null,
                    "score": null,
                    "signal": null,
                    "artifacts": [],
                    "children": [
                        {
                            "addr": addr(&["scan"], 1),
                            "label": "actor:quality_scanner",
                            "inputs": null,
                            "outputs": {
                                "target": "alpha",
                                "rubric": "minimal quality pass"
                            },
                            "score": null,
                            "signal": null,
                            "artifacts": [],
                            "children": []
                        }
                    ]
                }
            ]
        },
        "status": "completed"
    })
}

fn addr(path: &[&str], local_seq: u32) -> serde_json::Value {
    json!({
        "run_id": "conformance-run",
        "node_path": path
            .iter()
            .map(|segment| json!({ "node": segment }))
            .collect::<Vec<_>>(),
        "iteration": 0,
        "local_seq": local_seq
    })
}

fn read_json(path: impl AsRef<Path>) -> serde_json::Value {
    let bytes = std::fs::read(path.as_ref()).expect("fixture reads");
    serde_json::from_slice(&bytes).expect("fixture parses as JSON")
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("contract crate has workspace root ancestor")
        .to_path_buf()
}
