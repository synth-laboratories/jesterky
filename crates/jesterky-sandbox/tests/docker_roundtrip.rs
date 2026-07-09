//! Proof of the docker backend mechanics against a REAL container: seed a host
//! mount, start the container, run a command via `docker exec`, capture files
//! written inside, and tear the container down. Skips (does not fail) when no
//! daemon is reachable or the image can't be pulled — CI without docker is fine.
//!
//! This validates lifecycle/exec/capture, NOT codex-in-container (auth/proxy),
//! which is layered on by the caller.

use jesterky_contract::sandbox::{
    SandboxBackend, SandboxCapture, SandboxConfig, SandboxMode, SandboxSeed,
};
use jesterky_sandbox::{provider_for, FileBlob, SeedCtx};

const IMAGE: &str = "alpine:3.20";

fn docker_ready() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ensure_image() -> bool {
    std::process::Command::new("docker")
        .args(["pull", IMAGE])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn docker_seed_exec_capture_cleanup() {
    if !docker_ready() {
        eprintln!("skipping: no docker daemon");
        return;
    }
    if !ensure_image() {
        eprintln!("skipping: could not pull {IMAGE}");
        return;
    }

    let cfg = SandboxConfig {
        backend: SandboxBackend::Docker {
            image: IMAGE.to_string(),
            setup: vec!["mkdir -p out".to_string()], // image-level setup, in-container
            mounts: vec![],
            env: vec![],
        },
        mode: SandboxMode::WorkspaceWrite,
        network: false,
        seed: SandboxSeed {
            copy_from: vec![],
            files_input: Some("job.files".to_string()),
            // Workspace-level setup, in-container: derive from the seeded file.
            setup: vec!["cp src.txt out/seeded_copy.txt".to_string()],
        },
        capture: Some(SandboxCapture {
            globs: vec!["out/**/*.txt".to_string()],
            into: "artifacts".to_string(),
        }),
    };
    let ctx = SeedCtx {
        spec_dir: std::env::temp_dir(),
        files: Some(vec![FileBlob {
            path: "src.txt".to_string(),
            content: "hello from host\n".to_string(),
        }]),
    };

    let provider = provider_for(&cfg).expect("docker provider");
    let sandbox = provider.create(&cfg, &ctx).await.expect("create container");

    // The actor's command runs INSIDE the container (alpine's `sh`).
    let status = sandbox
        .command("sh", &["-c".into(), "echo built-in-container > out/ran.txt".into()], &[])
        .status()
        .await
        .expect("docker exec");
    assert!(status.success());

    // Capture reads the shared mount host-side; both setup + exec outputs land.
    let blobs = sandbox.collect(&cfg.capture.clone().unwrap().globs).await.unwrap();
    let paths: Vec<&str> = blobs.iter().map(|b| b.path.as_str()).collect();
    assert!(paths.contains(&"out/seeded_copy.txt"), "in-container setup captured: {paths:?}");
    assert!(paths.contains(&"out/ran.txt"), "exec output captured: {paths:?}");
    let ran = blobs.iter().find(|b| b.path == "out/ran.txt").unwrap();
    assert_eq!(ran.content.trim(), "built-in-container");

    assert_eq!(sandbox.workdir().to_string_lossy(), "/workspace");
    drop(sandbox); // container force-removed, mount cleaned
}
