//! Seeded execution workspaces for jesterky agentic actors.
//!
//! A [`Sandbox`] is a workspace an actor (codex) runs tools inside: a directory
//! seeded from ledger inputs and/or host source, a permission mode, and a way to
//! read the produced files back out. This crate is host-side runtime — the core
//! contract never references it (a subprocess "is the host's private concern").
//!
//! Backends share all seeding/capture logic ([`seed_workspace`], [`collect_globs`])
//! and differ only in *where commands run*: [`local::LocalSandbox`] runs on the
//! host; `DockerSandbox` (next slice) runs codex inside a container.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use jesterky_contract::sandbox::{SandboxConfig, SandboxMode, SandboxSeed};
use tokio::process::Command;

pub mod docker;
pub mod local;

/// One file read out of a workspace — the capture unit, shaped like the
/// `{path, content}` objects actors already emit.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FileBlob {
    pub path: String,
    pub content: String,
}

/// Everything a provider needs to seed a fresh workspace for one invocation.
pub struct SeedCtx {
    /// Directory the spec lives in — `copy_from` paths resolve against it.
    pub spec_dir: PathBuf,
    /// The value of the `seed.files_input` ledger field, if the config names one.
    pub files: Option<Vec<FileBlob>>,
}

/// A live workspace for ONE actor invocation. Dropping it cleans up (temp dir /
/// container).
#[async_trait]
pub trait Sandbox: Send + Sync + std::fmt::Debug {
    /// Host-visible workspace path (codex `--cd`).
    fn workdir(&self) -> &Path;
    /// Permission level the actor self-applies (codex `--sandbox`).
    fn mode(&self) -> SandboxMode;
    /// Whether the actor's tools may reach the network (e.g. `cargo` fetching
    /// crates). Local: codex workspace-write network access. Docker: applied at
    /// container start.
    fn network(&self) -> bool;
    /// Env the actor's process must use INSIDE the sandbox, overriding its own
    /// (e.g. an in-container `CODEX_HOME` for mounted codex auth). Empty for local.
    fn env(&self) -> &[(String, String)] {
        &[]
    }
    /// Whether the actor should apply its OWN process sandbox. True for local
    /// (codex's landlock/seatbelt enforces `mode`). False when the sandbox is
    /// itself the isolation boundary (docker) — codex's host-level sandbox can't
    /// initialize in a container, so it runs unconfined *within* the container.
    fn actor_self_sandbox(&self) -> bool {
        true
    }
    /// Build a command that runs INSIDE the sandbox. `env` pairs are injected
    /// (e.g. `CODEX_HOME`). Local: on the host with cwd = workdir. Docker:
    /// `docker exec` into the container.
    fn command(&self, program: &str, args: &[String], env: &[(String, String)]) -> Command;
    /// Read matching files back out of the workspace (capture).
    async fn collect(&self, globs: &[String]) -> Result<Vec<FileBlob>, SandboxError>;
}

/// Creates fresh, isolated sandboxes — one per actor invocation / map shard.
#[async_trait]
pub trait SandboxProvider: Send + Sync {
    async fn create(
        &self,
        cfg: &SandboxConfig,
        ctx: &SeedCtx,
    ) -> Result<Box<dyn Sandbox>, SandboxError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox io: {0}")]
    Io(String),
    #[error("sandbox setup command failed: `{cmd}` exited {code}: {detail}")]
    Setup {
        cmd: String,
        code: i32,
        detail: String,
    },
    #[error("sandbox backend unavailable: {0}")]
    Backend(String),
}

/// Stage the workspace CONTENT into `root`: write `files`, copy `copy_from` dirs.
/// Setup commands are separate ([`run_setup`]) because backends differ on *where*
/// they run — the host for local, inside the container for docker (after the
/// bind-mounted content is already staged here).
pub async fn stage_seed_files(
    root: &Path,
    seed: &SandboxSeed,
    ctx: &SeedCtx,
) -> Result<(), SandboxError> {
    if let Some(files) = &ctx.files {
        for f in files {
            write_file(root, &f.path, &f.content).await?;
        }
    }
    for rel in &seed.copy_from {
        let src = ctx.spec_dir.join(rel);
        let name = Path::new(rel)
            .file_name()
            .map(|n| n.to_owned())
            .ok_or_else(|| SandboxError::Io(format!("copy_from `{rel}` has no final component")))?;
        copy_dir(&src, &root.join(name)).await?;
    }
    Ok(())
}

/// Run each seed `setup` command through `make_cmd` (host `sh -c` for local,
/// `docker exec … sh -c` for docker); the first non-zero exit fails the seed.
pub async fn run_setup(
    setup: &[String],
    make_cmd: impl Fn(&str) -> Command,
) -> Result<(), SandboxError> {
    for cmd in setup {
        let status = make_cmd(cmd)
            .status()
            .await
            .map_err(|e| SandboxError::Io(format!("spawning setup `{cmd}`: {e}")))?;
        if !status.success() {
            return Err(SandboxError::Setup {
                cmd: cmd.clone(),
                code: status.code().unwrap_or(-1),
                detail: "see stderr".to_string(),
            });
        }
    }
    Ok(())
}

async fn write_file(root: &Path, rel: &str, content: &str) -> Result<(), SandboxError> {
    let target = safe_join(root, rel)?;
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| SandboxError::Io(format!("mkdir {}: {e}", parent.display())))?;
    }
    tokio::fs::write(&target, content)
        .await
        .map_err(|e| SandboxError::Io(format!("write {}: {e}", target.display())))
}

/// Join `rel` under `root`, rejecting `..`/absolute escapes — seed content is
/// model/ledger-sourced, so it must never write outside the workspace.
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, SandboxError> {
    let rel = Path::new(rel.trim_start_matches('/'));
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(SandboxError::Io(format!("unsafe path escapes workspace: {rel:?}")));
    }
    Ok(root.join(rel))
}

fn copy_dir<'a>(
    src: &'a Path,
    dst: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SandboxError>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dst)
            .await
            .map_err(|e| SandboxError::Io(format!("mkdir {}: {e}", dst.display())))?;
        let mut rd = tokio::fs::read_dir(src)
            .await
            .map_err(|e| SandboxError::Io(format!("read_dir {}: {e}", src.display())))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| SandboxError::Io(format!("read_dir entry: {e}")))?
        {
            let ft = entry
                .file_type()
                .await
                .map_err(|e| SandboxError::Io(format!("file_type: {e}")))?;
            let to = dst.join(entry.file_name());
            if ft.is_dir() {
                copy_dir(&entry.path(), &to).await?;
            } else if ft.is_file() {
                tokio::fs::copy(entry.path(), &to)
                    .await
                    .map_err(|e| SandboxError::Io(format!("copy {}: {e}", to.display())))?;
            }
        }
        Ok(())
    })
}

/// Collect files under `root` matching any of `globs` (relative paths). Supports
/// literal paths and `*` / `**` segments. Skips `target/` and `.git/` — build
/// output and VCS metadata are never part of a capture.
pub async fn collect_globs(root: &Path, globs: &[String]) -> Result<Vec<FileBlob>, SandboxError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| SandboxError::Io(format!("read_dir: {e}")))?
        {
            let path = entry.path();
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            let ft = entry
                .file_type()
                .await
                .map_err(|e| SandboxError::Io(format!("file_type: {e}")))?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if globs.iter().any(|g| glob_match(g, &rel)) {
                    let content = tokio::fs::read_to_string(&path)
                        .await
                        .map_err(|e| SandboxError::Io(format!("read {}: {e}", path.display())))?;
                    out.push(FileBlob { path: rel, content });
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Minimal glob: `/`-split segments; `**` matches any number of segments, `*`
/// matches any run of chars within one segment, else literal.
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let seg: Vec<&str> = path.split('/').collect();
    seg_match(&pat, &seg)
}

fn seg_match(pat: &[&str], seg: &[&str]) -> bool {
    match pat.first() {
        None => seg.is_empty(),
        Some(&"**") => {
            // `**` consumes zero or more path segments.
            (0..=seg.len()).any(|i| seg_match(&pat[1..], &seg[i..]))
        }
        Some(p) => {
            !seg.is_empty() && wildcard(p, seg[0]) && seg_match(&pat[1..], &seg[1..])
        }
    }
}

/// Single-segment wildcard match for `*` (greedy, backtracking).
fn wildcard(pat: &str, s: &str) -> bool {
    let (pb, sb) = (pat.as_bytes(), s.as_bytes());
    let (mut pi, mut si, mut star, mut mark) = (0usize, 0usize, None, 0usize);
    while si < sb.len() {
        if pi < pb.len() && (pb[pi] == b'*') {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if pi < pb.len() && pb[pi] == sb[si] {
            pi += 1;
            si += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < pb.len() && pb[pi] == b'*' {
        pi += 1;
    }
    pi == pb.len()
}

/// Build the provider a `SandboxConfig` selects. Docker lands in the next slice.
pub fn provider_for(cfg: &SandboxConfig) -> Result<Box<dyn SandboxProvider>, SandboxError> {
    match &cfg.backend {
        jesterky_contract::sandbox::SandboxBackend::Local => {
            Ok(Box::new(local::LocalSandboxProvider))
        }
        jesterky_contract::sandbox::SandboxBackend::Docker { .. } => {
            Ok(Box::new(docker::DockerSandboxProvider))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("Cargo.toml", "Cargo.toml"));
        assert!(glob_match("src/**/*.rs", "src/bin/scenario.rs"));
        assert!(glob_match("src/**/*.rs", "src/lib.rs"));
        assert!(glob_match("src/**", "src/a/b/c.txt"));
        assert!(glob_match("*.json", "score.json"));
        assert!(!glob_match("src/**/*.rs", "Cargo.toml"));
        assert!(!glob_match("*.json", "src/a.json"));
    }
}
