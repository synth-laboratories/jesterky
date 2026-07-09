//! Local backend: the workspace is a host temp dir and commands run on the host
//! with the host toolchain. The achievable first slice — enough for an actor to
//! read a seeded project, run `uv run python …` / `cargo`, and be captured.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use jesterky_contract::sandbox::{SandboxConfig, SandboxMode};
use tokio::process::Command;

use crate::{
    collect_globs, run_setup, stage_seed_files, FileBlob, Sandbox, SandboxError, SandboxProvider,
    SeedCtx,
};

pub struct LocalSandboxProvider;

#[async_trait]
impl SandboxProvider for LocalSandboxProvider {
    async fn create(
        &self,
        cfg: &SandboxConfig,
        ctx: &SeedCtx,
    ) -> Result<Box<dyn Sandbox>, SandboxError> {
        let root = unique_workspace_dir();
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| SandboxError::Io(format!("mkdir workspace {}: {e}", root.display())))?;
        stage_seed_files(&root, &cfg.seed, ctx).await?;
        let setup_root = root.clone();
        run_setup(&cfg.seed.setup, |cmd| {
            // Setup runs on the host, rooted at the workspace, via the shell.
            let mut c = Command::new("sh");
            c.arg("-c").arg(cmd).current_dir(&setup_root);
            c
        })
        .await?;
        Ok(Box::new(LocalSandbox {
            root,
            mode: cfg.mode,
            network: cfg.network,
        }))
    }
}

#[derive(Debug)]
pub struct LocalSandbox {
    root: PathBuf,
    mode: SandboxMode,
    network: bool,
}

#[async_trait]
impl Sandbox for LocalSandbox {
    fn workdir(&self) -> &Path {
        &self.root
    }

    fn mode(&self) -> SandboxMode {
        self.mode
    }

    fn network(&self) -> bool {
        self.network
    }

    fn command(&self, program: &str, args: &[String], env: &[(String, String)]) -> Command {
        let mut c = Command::new(program);
        c.args(args).current_dir(&self.root);
        for (k, v) in env {
            c.env(k, v);
        }
        c
    }

    async fn collect(&self, globs: &[String]) -> Result<Vec<FileBlob>, SandboxError> {
        collect_globs(&self.root, globs).await
    }
}

impl Drop for LocalSandbox {
    fn drop(&mut self) {
        // RAII cleanup: best-effort remove the temp workspace.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A collision-free workspace dir under `~/.cache/jesterky/workspaces` (NOT the OS
/// temp root — codex refuses to run under `$TMPDIR`). No `rand`/clock (kept
/// deterministic-friendly): a process-wide atomic counter + pid.
fn unique_workspace_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("jesterky")
        .join("workspaces");
    base.join(format!("sbx-{pid}-{n}"))
}
