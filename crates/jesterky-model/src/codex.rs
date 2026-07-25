//! [`CodexModel`] — a [`Model`] that drives `codex exec` (the headless codex
//! CLI) using codex's own **ChatGPT-bundle auth** (`~/.codex/auth.json`). It
//! **never** sets an OpenAI API key (hard house rule). This is heavyweight —
//! one agent session per completion — but it is the auth-compliant model access
//! we have here; a DeepSeek-through-proxy [`Model`] can slot in beside it later
//! without touching [`ModelActor`](crate::ModelActor).

use crate::limiter::AdaptiveLimiter;
use crate::{build_prompt, Model, ModelError, ModelRequest};
use async_trait::async_trait;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

/// How many times a retryable (transient / rate-limit) codex call is re-attempted
/// before giving up. Deterministic failures (auth/config/parse) never retry.
const MAX_ATTEMPTS: u32 = 4;

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
    /// AIMD concurrency ceiling for this model+provider. `None` = unlimited (the
    /// map's own width is the only bound); set it so 429s throttle in-flight calls
    /// and clean calls climb the ceiling back up.
    pub limiter: Option<Arc<AdaptiveLimiter>>,
}

impl CodexModel {
    pub fn new(model: impl Into<String>, effort: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            effort: effort.into(),
            cwd: None,
            codex_home: None,
            binary: "codex".to_string(),
            limiter: None,
        }
    }

    /// Attach an [`AdaptiveLimiter`] shared across every shard on this
    /// model+provider — the dynamic (AIMD) concurrency gate.
    pub fn with_limiter(mut self, limiter: Arc<AdaptiveLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
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
        let mut attempt = 0u32;
        loop {
            // Take a permit from the AIMD gate (if any). It releases when this
            // attempt's guard drops — including *before* a backoff sleep, so a
            // waiting shard can proceed at the (possibly lowered) ceiling.
            let permit = match &self.limiter {
                Some(limiter) => Some(limiter.acquire().await),
                None => None,
            };
            match self.exec_once(req, &prompt, attempt).await {
                Ok(reply) => {
                    if let Some(limiter) = &self.limiter {
                        limiter.on_success();
                    }
                    return Ok(reply);
                }
                Err(err) => {
                    // A 429 backs the ceiling off (multiplicative decrease) even
                    // when we've exhausted retries — the next shard benefits.
                    if err.is_rate_limit() {
                        if let Some(limiter) = &self.limiter {
                            limiter.on_rate_limited();
                        }
                    }
                    if !(err.is_retryable() && attempt + 1 < MAX_ATTEMPTS) {
                        return Err(err);
                    }
                    drop(permit);
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                }
            }
        }
    }
}

impl CodexModel {
    /// One codex subprocess: build the command, stream its JSONL, assemble the
    /// reply, classify a non-zero exit. Retry / limiter concerns stay in
    /// [`Model::complete`] so this is a single clean attempt.
    async fn exec_once(
        &self,
        req: &ModelRequest,
        prompt: &str,
        attempt: u32,
    ) -> Result<String, ModelError> {
        // The workspace + permission come from the per-call sandbox if the actor
        // declared one; else the model runs read-only in `self.cwd` (as before).
        // The workdir is set both as codex's `--cd` and the process cwd so a
        // `workspace-write` run has an unambiguous writable root.
        let sandbox = req.sandbox.as_ref();
        // In a container (actor_self_sandbox = false) codex can't init its
        // host-level sandbox — the container isolates it, so it runs unconfined
        // WITHIN the container. Locally, codex enforces `mode` itself.
        let in_container = sandbox.map(|s| !s.actor_self_sandbox()).unwrap_or(false);
        let sandbox_flag = if in_container {
            "danger-full-access"
        } else {
            sandbox
                .map(|s| s.mode().codex_flag())
                .unwrap_or("read-only")
        };
        let workdir = sandbox
            .map(|s| s.workdir().to_path_buf())
            .or_else(|| self.cwd.clone());

        let mut args: Vec<String> = vec!["exec".into(), "-m".into(), self.model.clone()];
        // Omit the effort flag for routes that don't accept it (empty effort).
        if !self.effort.is_empty() {
            args.push("-c".into());
            args.push(format!("model_reasoning_effort=\"{}\"", self.effort));
        }
        args.push("--sandbox".into());
        args.push(sandbox_flag.into());
        // A workspace-write actor that must `cargo fetch` / `uv sync` needs network;
        // codex's workspace-write sandbox denies it by default. Only relevant when
        // codex self-sandboxes (local) — in a container, network is the container's.
        if let Some(sb) = sandbox {
            if !in_container && sb.mode() == jesterky_contract::sandbox::SandboxMode::WorkspaceWrite
            {
                args.push("-c".into());
                args.push(format!(
                    "sandbox_workspace_write.network_access={}",
                    sb.network()
                ));
            }
        }
        args.push("--skip-git-repo-check".into());
        args.push("--ephemeral".into());
        // Stream events as JSONL so we can surface live per-shard progress
        // (tokens, steps, latest action) instead of one opaque blocking call.
        args.push("--json".into());
        // `--output-schema` points at a HOST file; codex-in-container can't read it.
        // The host-side `ModelActor` validates the reply against the same schema on
        // every backend, so in a container we rely on that + the prompt's shape spec
        // and skip the flag (dropping only codex's in-process steer, not the gate).
        let proxy_codex_home = self
            .codex_home
            .as_deref()
            .is_some_and(codex_home_uses_proxy_provider);
        if let Some(schema) = &req.output_schema {
            if !in_container && !proxy_codex_home {
                args.push("--output-schema".into());
                args.push(schema.to_string_lossy().into_owned());
            }
        }
        if let Some(cwd) = &workdir {
            args.push("--cd".into());
            args.push(cwd.to_string_lossy().into_owned());
        }
        args.push(prompt.to_string());

        // Env: codex uses its own auth (ChatGPT bundle / proxy config under
        // CODEX_HOME) — intentionally NOT OPENAI_API_KEY.
        let mut env: Vec<(String, String)> = Vec::new();
        // A sandbox command does not necessarily inherit the host environment.
        // Carry the central trace context explicitly and unchanged, then add
        // Jesterky's structural child identity as join metadata. The Containers
        // importer remains the schema/registration authority.
        const TRACE_ENV: &[&str] = &[
            "SYNTH_TRACE_ID",
            "SYNTH_CAPTURE_ID",
            "SYNTH_ACTOR_ID",
            "SYNTH_ACTOR_SESSION_ID",
            "SYNTH_PARENT_ACTOR_ID",
            "SYNTH_PARENT_ACTOR_SESSION_ID",
            "SYNTH_PARENT_SPAN_ID",
            "SYNTH_DELEGATION_ID",
            "SYNTH_WORKFLOW_ADDRESS",
            "SYNTH_TRACE_BINDING_PATH",
            "SYNTH_TRACE_COLLECTOR_URL",
            "SYNTH_TRACE_COLLECTOR_TOKEN",
            "SYNTH_TRACE_OUTPUT_DIR",
            "TRACEPARENT",
        ];
        for key in TRACE_ENV {
            if let Ok(value) = std::env::var(key) {
                env.push(((*key).to_string(), value));
            }
        }
        let workflow_address = req
            .node_path
            .as_ref()
            .and_then(|path| serde_json::to_string(path).ok())
            .unwrap_or_else(|| "[]".to_string());
        let mut child_capture_id: Option<String> = None;
        if std::env::var("SYNTH_TRACE_ID").is_ok() {
            let child_env = register_trace_child(req, attempt, &workflow_address).await?;
            child_capture_id = child_env
                .iter()
                .find(|(key, _)| key == "SYNTH_CAPTURE_ID")
                .map(|(_, value)| value.clone());
            for (key, value) in child_env {
                env.retain(|(existing, _)| existing != &key);
                env.push((key, value));
            }
        }
        env.push(("SYNTH_JESTERKY_ATTEMPT".into(), attempt.to_string()));
        env.push(("SYNTH_JESTERKY_NATIVE_ADDR".into(), workflow_address));
        if let Some(codex_home) = &self.codex_home {
            env.push((
                "CODEX_HOME".into(),
                codex_home.to_string_lossy().into_owned(),
            ));
        }
        // Sandbox-provided env wins (e.g. an in-container CODEX_HOME pointing at
        // mounted auth): drop any key the sandbox overrides, then append its env.
        if let Some(sb) = sandbox {
            let over: std::collections::HashSet<&str> = sb
                .env()
                .iter()
                .filter(|(key, _)| {
                    !key.starts_with("SYNTH_TRACE")
                        && !key.starts_with("SYNTH_ACTOR")
                        && key.as_str() != "SYNTH_PARENT_ACTOR_ID"
                        && key.as_str() != "SYNTH_PARENT_ACTOR_SESSION_ID"
                        && key.as_str() != "SYNTH_PARENT_SPAN_ID"
                        && key.as_str() != "SYNTH_DELEGATION_ID"
                        && key.as_str() != "SYNTH_WORKFLOW_ADDRESS"
                        && key.as_str() != "TRACEPARENT"
                })
                .map(|(k, _)| k.as_str())
                .collect();
            env.retain(|(k, _)| !over.contains(k.as_str()));
            env.extend(
                sb.env()
                    .iter()
                    .filter(|(key, _)| {
                        !key.starts_with("SYNTH_TRACE")
                            && !key.starts_with("SYNTH_ACTOR")
                            && key.as_str() != "SYNTH_PARENT_ACTOR_ID"
                            && key.as_str() != "SYNTH_PARENT_ACTOR_SESSION_ID"
                            && key.as_str() != "SYNTH_PARENT_SPAN_ID"
                            && key.as_str() != "SYNTH_DELEGATION_ID"
                            && key.as_str() != "SYNTH_WORKFLOW_ADDRESS"
                            && key.as_str() != "TRACEPARENT"
                    })
                    .cloned(),
            );
        }

        // Build the command IN the sandbox (docker: `docker exec`; local: host
        // with cwd = workspace) or, with no sandbox, a plain host invocation.
        let mut cmd = match sandbox {
            Some(sb) => sb.command(&self.binary, &args, &env),
            None => {
                let mut c = Command::new(&self.binary);
                c.args(&args);
                for (k, v) in &env {
                    c.env(k, v);
                }
                c
            }
        };
        cmd.stdin(Stdio::null());
        // No orphaned codex if this future is dropped mid-flight (M2 DoD).
        cmd.kill_on_drop(true);
        // Intentionally do NOT set OPENAI_API_KEY — codex uses its own auth
        // (ChatGPT bundle, or the proxy config under CODEX_HOME). Other env
        // (e.g. SYNTH_API_KEY for the proxy) is inherited from the parent.

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut native_trace_sink = match (
            child_capture_id.as_deref(),
            std::env::var("SYNTH_JESTERKY_CODEX_JSONL_DIR").ok(),
        ) {
            (Some(capture_id), Some(directory)) => {
                std::fs::create_dir_all(&directory).map_err(|err| {
                    ModelError::Config(format!(
                        "failed to create Jesterky Codex JSONL directory: {err}"
                    ))
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                        .map_err(|err| {
                            ModelError::Config(format!(
                                "failed to protect Jesterky Codex JSONL directory: {err}"
                            ))
                        })?;
                }
                let path = PathBuf::from(directory).join(format!("{capture_id}.jsonl"));
                let mut options = std::fs::OpenOptions::new();
                options.create(true).write(true).truncate(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt as _;
                    options.mode(0o600);
                }
                let sink = options.open(&path).map_err(|err| {
                    ModelError::Config(format!(
                        "failed to open Codex native trace `{}`: {err}",
                        path.display()
                    ))
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    sink.set_permissions(std::fs::Permissions::from_mode(0o600))
                        .map_err(|err| {
                            ModelError::Config(format!(
                                "failed to protect Codex native trace `{}`: {err}",
                                path.display()
                            ))
                        })?;
                }
                Some(sink)
            }
            _ => None,
        };

        let mut child = cmd.spawn().map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                ModelError::Config(format!("codex binary `{}` not found on PATH", self.binary))
            }
            _ => ModelError::Transient(format!("failed to spawn codex: {err}")),
        })?;

        // Consume the JSONL event stream as it arrives: assemble the final
        // `agent_message` (the model's actual reply — the return contract) and,
        // when a live [`LiveBus`] is registered, publish tokens / steps /
        // latest action for this shard on every event.
        let stdout = child
            .stdout
            .take()
            .expect("piped stdout is present after spawn");
        let mut lines = BufReader::new(stdout).lines();
        let mut reply = String::new();
        let mut progress = ShardStream::default();
        let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|err| ModelError::Transient(format!("reading codex stream: {err}")))?
        {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(sink) = native_trace_sink.as_mut() {
                writeln!(sink, "{line}").map_err(|err| {
                    ModelError::Transient(format!("writing Jesterky Codex native trace: {err}"))
                })?;
            }
            // Keep a bounded tail of the raw JSONL so a non-zero exit can surface
            // codex's own error events (which ride stdout, not stderr).
            tail.push_back(line.clone());
            if tail.len() > 12 {
                tail.pop_front();
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) {
                progress.ingest(&event, &mut reply);
                if let (Some(live), Some(path)) = (&req.live, &req.node_path) {
                    live.publish(
                        path,
                        progress.tokens_in,
                        progress.tokens_out,
                        progress.steps,
                        &progress.last_action,
                    );
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|err| ModelError::Transient(format!("waiting on codex: {err}")))?;
        if child_capture_id.is_some() {
            let terminal_status = if status.success() && !reply.trim().is_empty() {
                "completed"
            } else {
                "failed"
            };
            finish_trace_child(&env, terminal_status).await?;
        }

        if status.success() {
            if reply.trim().is_empty() {
                let tail_str: String = tail.iter().cloned().collect::<Vec<_>>().join("\n");
                let detail = if tail_str.trim().is_empty() {
                    "codex stream carried no agent_message text".to_string()
                } else {
                    format!("codex stream carried no agent_message text; stdout tail:\n{tail_str}")
                };
                return Err(ModelError::Parse(detail));
            }
            Ok(reply)
        } else {
            let mut stderr = String::new();
            if let Some(mut handle) = child.stderr.take() {
                let _ = handle.read_to_string(&mut stderr).await;
            }
            // codex's real error usually rides the stdout JSONL, not stderr; append
            // the tail so the failure is legible instead of a lone stderr line.
            let tail_str: String = tail.iter().cloned().collect::<Vec<_>>().join("\n");
            let combined = if tail_str.trim().is_empty() {
                stderr
            } else {
                format!("{stderr}\n--- codex stdout tail ---\n{tail_str}")
            };
            Err(classify_codex_failure(&combined))
        }
    }
}

async fn register_trace_child(
    req: &ModelRequest,
    attempt: u32,
    workflow_address: &str,
) -> Result<Vec<(String, String)>, ModelError> {
    let registrar = std::env::var("SYNTH_TRACE_CHILD_REGISTRAR").map_err(|_| {
        ModelError::Config(
            "SYNTH_TRACE_CHILD_REGISTRAR is required when Synth tracing is active".to_string(),
        )
    })?;
    let python = std::env::var("SYNTH_TRACE_CHILD_REGISTRAR_PYTHON")
        .unwrap_or_else(|_| "python3".to_string());
    let output = Command::new(&python)
        .arg(&registrar)
        .arg("--actor")
        .arg(&req.actor)
        .arg("--workflow-address")
        .arg(workflow_address)
        .arg("--attempt")
        .arg(attempt.to_string())
        .output()
        .await
        .map_err(|err| {
            ModelError::Config(format!(
                "failed to launch trace child registrar `{registrar}`: {err}"
            ))
        })?;
    if !output.status.success() {
        return Err(ModelError::Config(format!(
            "trace child registration failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let values: std::collections::BTreeMap<String, String> = serde_json::from_slice(&output.stdout)
        .map_err(|err| {
            ModelError::Config(format!(
                "trace child registrar returned invalid environment JSON: {err}"
            ))
        })?;
    for required in [
        "SYNTH_TRACE_ID",
        "SYNTH_CAPTURE_ID",
        "SYNTH_ACTOR_ID",
        "SYNTH_ACTOR_SESSION_ID",
        "SYNTH_PARENT_ACTOR_ID",
        "SYNTH_DELEGATION_ID",
        "SYNTH_TRACE_COLLECTOR_TOKEN",
    ] {
        if !values.contains_key(required) {
            return Err(ModelError::Config(format!(
                "trace child registrar omitted {required}"
            )));
        }
    }
    Ok(values.into_iter().collect())
}

async fn finish_trace_child(
    child_env: &[(String, String)],
    status: &str,
) -> Result<(), ModelError> {
    let registrar = std::env::var("SYNTH_TRACE_CHILD_REGISTRAR").map_err(|_| {
        ModelError::Config(
            "SYNTH_TRACE_CHILD_REGISTRAR is required when Synth tracing is active".to_string(),
        )
    })?;
    let python = std::env::var("SYNTH_TRACE_CHILD_REGISTRAR_PYTHON")
        .unwrap_or_else(|_| "python3".to_string());
    let mut command = Command::new(&python);
    command.arg(&registrar).arg("--finish-status").arg(status);
    for (key, value) in child_env {
        command.env(key, value);
    }
    let output = command.output().await.map_err(|err| {
        ModelError::Config(format!(
            "failed to launch trace child finisher `{registrar}`: {err}"
        ))
    })?;
    if !output.status.success() {
        return Err(ModelError::Config(format!(
            "trace child finish failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Running totals folded from the codex JSONL event stream for one shard.
#[derive(Default)]
struct ShardStream {
    steps: u32,
    tokens_in: u64,
    tokens_out: u64,
    last_action: String,
}

impl ShardStream {
    /// Fold one codex event; append any `agent_message` text into `reply`.
    fn ingest(&mut self, event: &serde_json::Value, reply: &mut String) {
        match event.get("type").and_then(|t| t.as_str()) {
            Some("item.completed") => {
                self.steps += 1;
                let item = event.get("item").unwrap_or(&serde_json::Value::Null);
                let kind = item
                    .get("type")
                    .or_else(|| item.get("item_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("step");
                if kind == "agent_message" {
                    if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                        if !reply.is_empty() {
                            reply.push('\n');
                        }
                        reply.push_str(text);
                    }
                } else if let Some(action) = action_label(kind, item) {
                    self.last_action = action;
                }
            }
            Some("turn.completed") => {
                if let Some(usage) = event.get("usage") {
                    // Sum output across turns; take input cumulatively (each turn
                    // re-sends context, so the latest input count is the total).
                    self.tokens_out += usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if let Some(input) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                        self.tokens_in = input;
                    }
                }
            }
            _ => {}
        }
    }
}

/// A short human label for a non-message step (`reading src/foo.rs`, `run cargo`).
fn action_label(kind: &str, item: &serde_json::Value) -> Option<String> {
    let text = match kind {
        "command_execution" => item
            .get("command")
            .and_then(|v| v.as_str())
            .map(|c| c.to_string()),
        "file_change" => item
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| format!("edit {p}")),
        "reasoning" => Some("thinking".to_string()),
        "mcp_tool_call" => item
            .get("tool")
            .and_then(|v| v.as_str())
            .map(|t| t.to_string()),
        "web_search" => Some("web search".to_string()),
        // Codex emits benign `error` items (e.g. "model metadata not found,
        // defaulting to fallback") mid-stream — don't let a warning masquerade as
        // the shard's action. A real failure surfaces as a non-zero exit / ✗ row.
        "error" => None,
        other => Some(other.replace('_', " ")),
    }?;
    Some(truncate_action(&text))
}

/// Keep a live action label compact (single line, ≤ 32 visible chars).
fn truncate_action(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= 32 {
        one_line
    } else {
        one_line.chars().take(31).collect::<String>() + "…"
    }
}

fn codex_home_uses_proxy_provider(codex_home: &std::path::Path) -> bool {
    let Ok(config) = std::fs::read_to_string(codex_home.join("config.toml")) else {
        return false;
    };
    config.contains("[model_providers.") || config.contains("base_url =")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact `codex exec --json` event stream shapes (captured from
    /// codex-cli 0.142.5) must fold to the right tokens / steps / reply.
    #[test]
    fn shard_stream_folds_real_codex_events() {
        let events = [
            r#"{"type":"thread.started","thread_id":"019f4211"}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"grep -rn unsafe src"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"ok"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":16117,"cached_input_tokens":4480,"output_tokens":5,"reasoning_output_tokens":0}}"#,
        ];
        let mut stream = ShardStream::default();
        let mut reply = String::new();
        for line in events {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            stream.ingest(&event, &mut reply);
        }
        assert_eq!(reply, "ok");
        assert_eq!(stream.tokens_in, 16117);
        assert_eq!(stream.tokens_out, 5);
        assert_eq!(stream.steps, 3);
        // Latest non-message action wins (the command, not "thinking").
        assert_eq!(stream.last_action, "grep -rn unsafe src");
    }

    #[test]
    fn agent_message_text_is_the_reply_not_the_events() {
        // Multiple agent messages concatenate; non-message items never leak in.
        let events = [
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"verdict\":\"pass\"}"}}"#,
        ];
        let mut stream = ShardStream::default();
        let mut reply = String::new();
        for line in events {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            stream.ingest(&event, &mut reply);
        }
        assert_eq!(reply, r#"{"verdict":"pass"}"#);
    }
}

/// Map codex's stderr to a failure class so the caller can react (re-auth,
/// back off + retry on rate-limit, retry a transient) instead of guessing.
///
/// Rate-limit signals (`429`, "rate limit", "too many requests", "overloaded")
/// map to [`ModelError::Quota`] so [`ModelError::is_rate_limit`] fires and the
/// AIMD ceiling drops; plain 5xx / timeout / connection blips stay `Transient`
/// (retry, but don't punish the ceiling).
fn classify_codex_failure(stderr: &str) -> ModelError {
    let lower = stderr.to_lowercase();
    let msg = stderr.trim().to_string();
    let rate_limited = lower.contains("usage limit")
        || lower.contains("usage_limit")
        || lower.contains("quota")
        || lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("too many requests")
        || lower.contains("overloaded");
    if rate_limited {
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

/// Exponential backoff with a small per-call jitter so retrying shards don't
/// re-collide in lockstep (thundering herd). `400ms · 2^attempt`, capped at 8s.
fn backoff(attempt: u32) -> Duration {
    static SPREAD: AtomicU64 = AtomicU64::new(0);
    let base = 400u64.saturating_mul(1u64 << attempt.min(5));
    let jitter = SPREAD.fetch_add(97, Ordering::Relaxed) % 300;
    Duration::from_millis(base.min(8_000) + jitter)
}
