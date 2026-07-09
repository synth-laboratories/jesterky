//! End-to-end proof of the local backend: seed a workspace (files + copy_from +
//! setup), run a command INSIDE it via `command()`, capture globs, and confirm
//! RAII cleanup on drop. No network / no codex — just the sandbox mechanics.

use std::path::Path;

use jesterky_contract::sandbox::{
    SandboxBackend, SandboxCapture, SandboxConfig, SandboxMode, SandboxSeed,
};
use jesterky_sandbox::{provider_for, FileBlob, SeedCtx};

#[tokio::test]
async fn local_seed_run_capture_cleanup() {
    // A source dir to `copy_from` (resolved against SeedCtx.spec_dir).
    let spec_dir = std::env::temp_dir().join(format!("jesterky-test-spec-{}", std::process::id()));
    std::fs::create_dir_all(spec_dir.join("gold")).unwrap();
    std::fs::write(spec_dir.join("gold/engine.py"), "print('gold')\n").unwrap();

    let cfg = SandboxConfig {
        backend: SandboxBackend::Local,
        mode: SandboxMode::WorkspaceWrite,
        network: false,
        seed: SandboxSeed {
            copy_from: vec!["gold".to_string()],
            files_input: Some("job.files".to_string()), // seeded via SeedCtx.files below
            // Setup runs in the workspace: derive a file from the seeded input so
            // we can prove the workspace is writable and the seed landed.
            setup: vec!["cat src/lib.rs > src/derived.txt".to_string()],
        },
        capture: Some(SandboxCapture {
            globs: vec!["src/**/*.txt".to_string(), "gold/**/*.py".to_string()],
            into: "artifacts".to_string(),
        }),
    };

    let ctx = SeedCtx {
        spec_dir: spec_dir.clone(),
        files: Some(vec![FileBlob {
            path: "src/lib.rs".to_string(),
            content: "// seeded source\n".to_string(),
        }]),
    };

    let provider = provider_for(&cfg).expect("local provider");
    let sandbox = provider.create(&cfg, &ctx).await.expect("create sandbox");
    let workdir = sandbox.workdir().to_path_buf();

    // Seeded file landed, copy_from landed, setup produced the derived file.
    assert!(workdir.join("src/lib.rs").is_file(), "files_input seeded");
    assert!(workdir.join("gold/engine.py").is_file(), "copy_from landed");
    assert!(workdir.join("src/derived.txt").is_file(), "setup wrote in workspace");

    // A command runs INSIDE the workspace (cwd = workdir).
    let status = sandbox
        .command("sh", &["-c".into(), "echo built > src/out.txt".into()], &[])
        .status()
        .await
        .unwrap();
    assert!(status.success());
    assert!(workdir.join("src/out.txt").is_file());

    // Capture picks up the txt files under src/ and the copied python.
    let blobs = sandbox.collect(&cfg.capture.unwrap().globs).await.unwrap();
    let paths: Vec<&str> = blobs.iter().map(|b| b.path.as_str()).collect();
    assert!(paths.contains(&"src/derived.txt"), "captured derived: {paths:?}");
    assert!(paths.contains(&"src/out.txt"), "captured out: {paths:?}");
    assert!(paths.contains(&"gold/engine.py"), "captured python: {paths:?}");

    // Mode is honored.
    assert_eq!(sandbox.mode(), SandboxMode::WorkspaceWrite);

    // RAII cleanup on drop.
    drop(sandbox);
    assert!(!Path::new(&workdir).exists(), "workspace removed on drop");

    std::fs::remove_dir_all(&spec_dir).ok();
}
