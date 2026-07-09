use std::path::Path;
use std::process::Command;

#[test]
fn run_then_replay_quality_min_manifest() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("quality_min.manifest.json");
    let spec = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join("quality_min.json");

    let run = Command::new(bin)
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&manifest)
        .output()
        .expect("run command executes");
    assert!(
        run.status.success(),
        "run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(manifest.exists(), "manifest was written");
    assert!(
        manifest.with_extension("spec.json").exists(),
        "spec sidecar was written"
    );

    let replay = Command::new(bin)
        .arg("replay")
        .arg(&manifest)
        .output()
        .expect("replay command executes");
    assert!(
        replay.status.success(),
        "replay failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );

    let replay_with_spec = Command::new(bin)
        .arg("replay")
        .arg(&manifest)
        .arg("--spec")
        .arg(&spec)
        .output()
        .expect("replay --spec command executes");
    assert!(
        replay_with_spec.status.success(),
        "replay --spec failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&replay_with_spec.stdout),
        String::from_utf8_lossy(&replay_with_spec.stderr)
    );
}

#[test]
fn run_then_replay_quality_scan_map_reduce_manifest() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("quality_scan.manifest.json");
    let spec = example_path("quality_scan.json");

    let run = Command::new(bin)
        .arg("run")
        .arg(&spec)
        .arg("--out")
        .arg(&manifest)
        .output()
        .expect("run command executes");
    assert!(
        run.status.success(),
        "run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("workflow:quality_scan"));
    assert!(stdout.contains("actor:quality_scanner"));
    assert!(stdout.contains("status=completed"));

    let replay = Command::new(bin)
        .arg("replay")
        .arg(&manifest)
        .arg("--spec")
        .arg(&spec)
        .output()
        .expect("replay command executes");
    assert!(
        replay.status.success(),
        "replay failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
}

#[test]
fn replay_rejects_a_spec_that_does_not_match_the_manifest() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("quality_min.manifest.json");

    // Produce a manifest from quality_min...
    let run = Command::new(bin)
        .arg("run")
        .arg(example_path("quality_min.json"))
        .arg("--out")
        .arg(&manifest)
        .output()
        .expect("run command executes");
    assert!(run.status.success(), "run failed");

    // ...then replay it against a DIFFERENT spec. spec_hash must reject it
    // rather than silently re-driving the wrong topology.
    let replay = Command::new(bin)
        .arg("replay")
        .arg(&manifest)
        .arg("--spec")
        .arg(example_path("quality_scan.json"))
        .output()
        .expect("replay command executes");
    assert!(
        !replay.status.success(),
        "replay with a mismatched spec must fail, but it succeeded"
    );
    let stderr = String::from_utf8_lossy(&replay.stderr);
    assert!(
        stderr.contains("spec_hash"),
        "mismatch error should mention spec_hash; got:\n{stderr}"
    );
}

#[test]
fn run_accepts_explicit_run_id() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("quality_min.manifest.json");

    let run = Command::new(bin)
        .arg("run")
        .arg(example_path("quality_min.json"))
        .arg("--run-id")
        .arg("fixed-run-1")
        .arg("--out")
        .arg(&manifest)
        .output()
        .expect("run command executes");
    assert!(
        run.status.success(),
        "run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    let manifest_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).expect("manifest reads"))
            .expect("manifest parses");
    assert_eq!(manifest_json["run_id"], "fixed-run-1");
}

#[test]
fn schema_commands_emit_parseable_json_schema() {
    let bin = env!("CARGO_BIN_EXE_jesterky");

    for artifact in ["workflow", "manifest"] {
        let output = Command::new(bin)
            .arg("schema")
            .arg(artifact)
            .output()
            .expect("schema command executes");
        assert!(
            output.status.success(),
            "schema {artifact} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let schema: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("schema output parses as JSON");
        assert!(schema.get("$schema").is_some(), "schema has $schema");
    }
}

#[test]
fn validate_accepts_quality_min_and_prints_spec_hash() {
    let bin = env!("CARGO_BIN_EXE_jesterky");

    let output = Command::new(bin)
        .arg("validate")
        .arg(example_path("quality_min.json"))
        .output()
        .expect("validate command executes");
    assert!(
        output.status.success(),
        "validate failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("spec_hash "),
        "validate should print spec_hash; got:\n{stdout}"
    );
}

#[test]
fn validate_rejects_invalid_spec_and_prints_diagnostic() {
    let bin = env!("CARGO_BIN_EXE_jesterky");

    let output = Command::new(bin)
        .arg("validate")
        .arg(example_path("invalid_dangling_entrypoint.json"))
        .output()
        .expect("validate command executes");
    assert!(
        !output.status.success(),
        "validate invalid spec should fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout
            .contains("error entrypoint[0]: dangling entrypoint references unknown node `missing`"),
        "validate should print diagnostic; got:\n{stdout}"
    );
}

fn example_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join(name)
}

#[test]
fn piped_run_skips_live_panel_by_default() {
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let run = Command::new(bin)
        .arg("run")
        .arg(example_path("quality_scan.json"))
        .output()
        .expect("run executes");
    assert!(
        run.status.success(),
        "run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("workflow:quality_scan"),
        "piped run should use skeleton tree; got:\n{stdout}"
    );
    assert!(
        !stdout.contains("╭─┤ quality_scan ├"),
        "piped run should not render live panel; got:\n{stdout}"
    );
}

#[test]
fn events_out_ndjson_matches_manifest_events() {
    // `--events-out` streams the canonical event log as NDJSON. Every line must
    // parse as an Event, and the set must equal `manifest.events` (same events,
    // one per line) — the durable stream and the manifest cannot diverge.
    let bin = env!("CARGO_BIN_EXE_jesterky");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("scan.manifest.json");
    let events = temp.path().join("events.ndjson");

    let run = Command::new(bin)
        .arg("run")
        .arg(example_path("quality_scan.json"))
        .arg("--actor")
        .arg("fake")
        .arg("--out")
        .arg(&manifest)
        .arg("--events-out")
        .arg(&events)
        .output()
        .expect("run executes");
    assert!(
        run.status.success(),
        "run failed\nstderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let ndjson = std::fs::read_to_string(&events).expect("events file written");
    let lines: Vec<&str> = ndjson.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "events NDJSON should be non-empty");
    for line in &lines {
        let value: serde_json::Value =
            serde_json::from_str(line).expect("each NDJSON line parses as JSON");
        assert!(
            value.get("addr").is_some() && value.get("kind").is_some(),
            "each line is an Event with addr + kind: {line}"
        );
    }

    let manifest_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).expect("manifest written"))
            .expect("manifest parses");
    let event_count = manifest_json["events"]
        .as_array()
        .expect("events array")
        .len();
    assert_eq!(
        lines.len(),
        event_count,
        "one NDJSON line per manifest event ({} vs {})",
        lines.len(),
        event_count
    );
}
