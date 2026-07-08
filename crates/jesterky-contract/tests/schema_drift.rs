use jesterky_contract::{manifest_schema_json, workflow_schema_json};
use std::path::Path;

const REFRESH_HINT: &str =
    "schema artifact is stale; re-run `cargo run -p jesterky-contract --example emit_schema`";

#[test]
fn workflow_schema_artifact_matches_emitter() {
    let committed = read_root_file("jesterky.schema.json");
    let regenerated = workflow_schema_json();

    assert_eq!(
        committed, regenerated,
        "jesterky.schema.json {REFRESH_HINT}"
    );
}

#[test]
fn manifest_schema_artifact_matches_emitter() {
    let committed = read_root_file("jesterky.manifest.schema.json");
    let regenerated = manifest_schema_json();

    assert_eq!(
        committed, regenerated,
        "jesterky.manifest.schema.json {REFRESH_HINT}"
    );
}

fn read_root_file(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read `{}`: {err}", path.display()))
}
