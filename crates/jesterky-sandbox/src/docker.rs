//! Docker backend: the workspace is a host dir bind-mounted into a container at
//! `/workspace`, and commands run INSIDE the container via `docker exec` — so the
//! actor's tools use the image's toolchain, isolated from the host. Because the
//! mount is shared, capture reads the host side directly.
//!
//! `SandboxMode::WorkspaceWrite` is the norm here; the container is the isolation
//! boundary. Seeding stages files on the host mount BEFORE the container starts,
//! then `seed.setup` runs via `docker exec`. Requires a reachable daemon
//! (OrbStack) and a native-arch image (no QEMU — house rule).
//!
//! Running codex ITSELF in the container (auth + proxy reachability) is layered
//! on top by the caller via `command()`; the mechanics here (lifecycle, exec,
//! capture) are backend-agnostic.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use jesterky_contract::sandbox::{SandboxBackend, SandboxConfig, SandboxMode};
use tokio::process::Command;

use crate::{
    collect_globs, run_setup, stage_seed_files, FileBlob, Sandbox, SandboxError, SandboxProvider,
    SeedCtx,
};

/// The fixed in-container workspace path (codex `--cd`, `docker exec -w`).
const CONTAINER_WORKDIR: &str = "/workspace";

pub struct DockerSandboxProvider;

#[async_trait]
impl SandboxProvider for DockerSandboxProvider {
    async fn create(
        &self,
        cfg: &SandboxConfig,
        ctx: &SeedCtx,
    ) -> Result<Box<dyn Sandbox>, SandboxError> {
        let (image, backend_setup, mounts, env) = match &cfg.backend {
            SandboxBackend::Docker {
                image,
                setup,
                mounts,
                env,
            } => (image.clone(), setup.clone(), mounts.clone(), env.clone()),
            SandboxBackend::Local => {
                return Err(SandboxError::Backend("docker provider given a local config".into()))
            }
        };

        // Stage workspace content on the host side of the bind mount.
        let host_root = unique_mount_dir();
        tokio::fs::create_dir_all(&host_root)
            .await
            .map_err(|e| SandboxError::Io(format!("mkdir mount {}: {e}", host_root.display())))?;
        stage_seed_files(&host_root, &cfg.seed, ctx).await?;

        // Start a detached, long-lived container with the workspace mounted.
        let mut run = Command::new("docker");
        run.arg("run").arg("-d").arg("--rm");
        if !cfg.network {
            run.arg("--network").arg("none");
        }
        run.arg("-v")
            .arg(format!("{}:{CONTAINER_WORKDIR}", host_root.display()))
            .arg("-w")
            .arg(CONTAINER_WORKDIR);
        // Extra bind mounts (e.g. codex auth), with ${HOME}/$VAR expansion.
        for m in &mounts {
            run.arg("-v").arg(expand_env(m));
        }
        run.arg(&image)
            // Keep it alive so we can `exec` repeatedly across the retry loop.
            .arg("sleep")
            .arg("infinity");
        let out = run
            .output()
            .await
            .map_err(|e| SandboxError::Backend(format!("docker run (is the daemon up?): {e}")))?;
        if !out.status.success() {
            return Err(SandboxError::Backend(format!(
                "docker run failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let container = String::from_utf8_lossy(&out.stdout).trim().to_string();

        let sandbox = DockerSandbox {
            container,
            host_root,
            mode: cfg.mode,
            network: cfg.network,
            env,
        };

        // Setup runs inside the container (image toolchain), rooted at the mount:
        // image-level first, then workspace-level.
        let all_setup: Vec<String> = backend_setup.into_iter().chain(cfg.seed.setup.clone()).collect();
        run_setup(&all_setup, |cmd| sandbox.exec_shell(cmd)).await?;

        Ok(Box::new(sandbox))
    }
}

#[derive(Debug)]
pub struct DockerSandbox {
    container: String,
    /// Host side of the bind mount — where capture reads from.
    host_root: PathBuf,
    mode: SandboxMode,
    network: bool,
    env: Vec<(String, String)>,
}

impl DockerSandbox {
    /// A `docker exec … sh -c "<cmd>"` in the container workdir.
    fn exec_shell(&self, cmd: &str) -> Command {
        let mut c = Command::new("docker");
        c.arg("exec").arg("-w").arg(CONTAINER_WORKDIR).arg(&self.container);
        c.arg("sh").arg("-c").arg(cmd);
        c
    }
}

#[async_trait]
impl Sandbox for DockerSandbox {
    fn workdir(&self) -> &Path {
        Path::new(CONTAINER_WORKDIR)
    }

    fn mode(&self) -> SandboxMode {
        self.mode
    }

    fn network(&self) -> bool {
        self.network
    }

    fn env(&self) -> &[(String, String)] {
        &self.env
    }

    fn actor_self_sandbox(&self) -> bool {
        false // the container is the isolation boundary
    }

    fn command(&self, program: &str, args: &[String], env: &[(String, String)]) -> Command {
        // `docker exec [-e K=V …] -w /workspace <cid> <program> <args…>`.
        let mut c = Command::new("docker");
        c.arg("exec").arg("-w").arg(CONTAINER_WORKDIR);
        for (k, v) in env {
            c.arg("-e").arg(format!("{k}={v}"));
        }
        c.arg(&self.container).arg(program).args(args);
        c
    }

    async fn collect(&self, globs: &[String]) -> Result<Vec<FileBlob>, SandboxError> {
        // The mount is shared, so container writes are visible on the host side.
        collect_globs(&self.host_root, globs).await
    }
}

impl Drop for DockerSandbox {
    fn drop(&mut self) {
        // Best-effort teardown: force-remove the container (also stops it; the
        // `--rm` then reaps it) and delete the host mount dir.
        let _ = std::process::Command::new("docker")
            .args(["rm", "-f", &self.container])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.host_root);
    }
}

/// Expand `${VAR}` and `$VAR` from the environment in a mount spec (so a spec can
/// say `${HOME}/.codex:/codex-home:ro` portably). Unset vars expand to empty.
fn expand_env(spec: &str) -> String {
    let mut out = String::with_capacity(spec.len());
    let bytes = spec.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let (name, next) = if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                let end = spec[i + 2..].find('}').map(|e| i + 2 + e);
                match end {
                    Some(e) => (&spec[i + 2..e], e + 1),
                    None => (&spec[i..i], i + 1),
                }
            } else {
                let rest = &spec[i + 1..];
                let len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                (&rest[..len], i + 1 + len)
            };
            if name.is_empty() {
                out.push('$');
                i += 1;
            } else {
                out.push_str(&std::env::var(name).unwrap_or_default());
                i = next;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn unique_mount_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("jesterky-dsbx-{pid}-{n}"))
}
