//! [`CodexModel`] — a [`Model`] that drives `codex exec` (the headless codex
//! CLI) using codex's own **ChatGPT-bundle auth** (`~/.codex/auth.json`). It
//! **never** sets an OpenAI API key (hard house rule). This is heavyweight —
//! one agent session per completion — but it is the auth-compliant model access
//! we have here; a DeepSeek-through-proxy [`Model`] can slot in beside it later
//! without touching [`ModelActor`](crate::ModelActor).

use crate::{build_prompt, Model, ModelError, ModelRequest};
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::process::Command;

pub struct CodexModel {
    /// Model id passed to `codex exec -m`. `gpt-5.5` for the ChatGPT bundle, or a
    /// proxy route id like `deepseek/deepseek-v4-pro-direct`.
    pub model: String,
    /// Reasoning effort (`model_reasoning_effort`): `low|medium|high|xhigh`. An
    /// EMPTY string omits the flag — non-ChatGPT routes may not accept it.
    pub effort: String,
    /// Working root for the agent (`--cd`). `None` = codex's default. Set this to
    /// the repo under audit so the read-only sandbox can read its files.
    pub cwd: Option<PathBuf>,
    /// `CODEX_HOME` for the subprocess — a sandboxed config dir holding the
    /// proxy `config.toml` / auth. `None` = inherit the caller's `~/.codex`.
    pub codex_home: Option<PathBuf>,
    /// The codex binary (overridable so tests can point at a fake).
    pub binary: String,
}

impl CodexModel {
    pub fn new(model: impl Into<String>, effort: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            effort: effort.into(),
            cwd: None,
            codex_home: None,
            binary: "codex".to_string(),
        }
    }

    /// The sensible default for a quality/audit actor: gpt-5.5 at high effort.
    pub fn gpt55() -> Self {
        Self::new("gpt-5.5", "high")
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Point the subprocess at a sandboxed `CODEX_HOME` (proxy config + auth).
    pub fn with_codex_home(mut self, codex_home: impl Into<PathBuf>) -> Self {
        self.codex_home = Some(codex_home.into());
        self
    }

    /// Override the binary (tests point this at a stub script).
    pub fn with_binary(mut self, binary: impl Into<String>) -> Self {
        self.binary = binary.into();
        self
    }
}

#[async_trait]
impl Model for CodexModel {
    async fn complete(&self, req: &ModelRequest) -> Result<String, ModelError> {
        let prompt = build_prompt(req);
        let mut cmd = Command::new(&self.binary);
        cmd.arg("exec").arg("-m").arg(&self.model);
        // Omit the effort flag for routes that don't accept it (empty effort).
        if !self.effort.is_empty() {
            cmd.arg("-c")
                .arg(format!("model_reasoning_effort=\"{}\"", self.effort));
        }
        cmd.arg("--sandbox")
            .arg("read-only")
            .arg("--skip-git-repo-check");
        if let Some(cwd) = &self.cwd {
            cmd.arg("--cd").arg(cwd);
        }
        if let Some(codex_home) = &self.codex_home {
            cmd.env("CODEX_HOME", codex_home);
        }
        cmd.arg(&prompt);
        // No orphaned codex if this future is dropped mid-flight (M2 DoD).
        cmd.kill_on_drop(true);
        // Intentionally do NOT set OPENAI_API_KEY — codex uses its own auth
        // (ChatGPT bundle, or the proxy config under CODEX_HOME). Other env
        // (e.g. SYNTH_API_KEY for the proxy) is inherited from the parent.

        let output = cmd.output().await.map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                ModelError::Config(format!("codex binary `{}` not found on PATH", self.binary))
            }
            _ => ModelError::Transient(format!("failed to spawn codex: {err}")),
        })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(classify_codex_failure(&stderr))
        }
    }
}

/// Map codex's stderr to a failure class so the caller can react (re-auth,
/// switch route on quota, retry a transient) instead of guessing.
fn classify_codex_failure(stderr: &str) -> ModelError {
    let lower = stderr.to_lowercase();
    let msg = stderr.trim().to_string();
    if lower.contains("usage limit") || lower.contains("usage_limit") || lower.contains("quota") {
        ModelError::Quota(msg)
    } else if lower.contains("unauthorized")
        || lower.contains("401")
        || lower.contains("auth")
        || lower.contains("login")
    {
        ModelError::Auth(msg)
    } else {
        ModelError::Transient(msg)
    }
}
