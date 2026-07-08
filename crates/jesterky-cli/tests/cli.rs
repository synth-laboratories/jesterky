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

fn example_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("examples")
        .join(name)
}
