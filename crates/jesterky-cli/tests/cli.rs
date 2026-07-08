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
}
