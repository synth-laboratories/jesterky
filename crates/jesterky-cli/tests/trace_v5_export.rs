use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn trace_export_delegates_schema_authority_to_synth_trace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let python = temp.path().join("trace-python");
    let calls = temp.path().join("calls.txt");
    fs::write(
        &python,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            calls.display()
        ),
    )
    .expect("write fake trace interpreter");
    let mut permissions = fs::metadata(&python).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&python, permissions).expect("chmod");

    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/trace_v5_manifest.json");
    let status = Command::new(env!("CARGO_BIN_EXE_jesterky"))
        .arg("trace-export")
        .arg(&fixture)
        .arg("--bundle")
        .arg(temp.path().join("bundle"))
        .arg("--atif")
        .env("SYNTH_TRACE_PYTHON", &python)
        .env("SYNTH_TRACE_CONTAINERS_WHEEL_PATH", &fixture)
        .status()
        .expect("run jesterky trace-export");
    assert!(status.success());
    let recorded = fs::read_to_string(calls).expect("read calls");
    assert!(recorded.contains("-m synth_containers.tracing.cli import --format jesterky --input"));
    assert!(recorded.contains("-m synth_containers.tracing.cli validate"));
    assert!(recorded.contains("-m synth_containers.tracing.cli project"));
    assert!(recorded.contains("--format atif"));
}
