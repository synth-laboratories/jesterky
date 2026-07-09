//! `jesterky-proxy` — a native, self-contained Responses↔chat proxy so the
//! `codex` CLI (which speaks ONLY the OpenAI Responses API) can drive chat-only
//! models (DeepSeek, Gemini, any OpenAI-chat-compatible provider).
//!
//! # What it does
//! Runs a localhost HTTP/1.1 server. codex POSTs an OpenAI **Responses** request
//! to `http://127.0.0.1:<port>/v1/responses`; the proxy converts it to an OpenAI
//! **chat/completions** request, calls the provider's real chat endpoint (HTTPS)
//! buffered (non-streaming), then re-emits the reply as the exact **Responses
//! SSE** event sequence codex expects. Also serves `GET /health` -> 200
//! `{"status":"ok"}`.
//!
//! # Usage
//! ```no_run
//! # async fn run() -> Result<(), jesterky_proxy::ProxyError> {
//! if let Some(proxy) = jesterky_proxy::ChatProxy::spawn("deepseek/deepseek-v4-pro-direct").await? {
//!     // Point codex at it: CODEX_HOME=proxy.codex_home(), which pins the proxy port.
//!     let _home = proxy.codex_home();
//!     // Keep `proxy` alive for the whole run; dropping it aborts the server.
//! }
//! # Ok(()) }
//! ```

mod convert;
mod route;
mod server;
mod sse;

pub use convert::ConvertError;
pub use route::{resolve_route, ProviderRoute};

use server::ServerState;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Errors raised while starting a proxy.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Filesystem / bind I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The provider's `api_key_env` env var is unset or empty.
    #[error("missing api key: environment variable `{0}` is unset or empty")]
    MissingKey(String),
    /// Failed to bind an ephemeral localhost port.
    #[error("failed to bind localhost port: {0}")]
    Bind(String),
}

/// A running proxy. Aborts its server task on drop. Keep it alive for the whole run.
pub struct ChatProxy {
    port: u16,
    task: JoinHandle<()>,
    codex_home: PathBuf,
}

impl ChatProxy {
    /// Resolve `model` to a provider, bind an ephemeral localhost port, spawn the
    /// server, and materialize a sandboxed `CODEX_HOME` whose `config.toml` points
    /// codex at this proxy (`wire_api = "responses"`). Returns `Ok(None)` if the
    /// model has no proxy mapping (native codex route). Errors if the provider's
    /// `api_key_env` is unset.
    pub async fn spawn(model: &str) -> Result<Option<ChatProxy>, ProxyError> {
        let route = match resolve_route(model) {
            Some(r) => r,
            None => return Ok(None),
        };

        let api_key = match std::env::var(&route.api_key_env) {
            Ok(v) if !v.trim().is_empty() => v,
            _ => return Err(ProxyError::MissingKey(route.api_key_env.clone())),
        };

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| ProxyError::Bind(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| ProxyError::Bind(e.to_string()))?
            .port();

        let codex_home = materialize_codex_home(model, port, &route.api_key_env)?;

        let state = Arc::new(ServerState {
            route,
            api_key,
            port,
            counter: AtomicU64::new(0),
            client: reqwest::Client::new(),
            codex_model: model.to_string(),
            signatures: std::sync::Mutex::new(std::collections::HashMap::new()),
        });

        let task = tokio::spawn(server::serve(listener, state));

        Ok(Some(ChatProxy {
            port,
            task,
            codex_home,
        }))
    }

    /// The bound localhost port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The generated `CODEX_HOME` dir (contains `config.toml`). Pass this to codex
    /// via the `CODEX_HOME` environment variable.
    pub fn codex_home(&self) -> &Path {
        &self.codex_home
    }
}

impl Drop for ChatProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Write a fresh sandboxed CODEX_HOME with a `config.toml` that pins codex to this
/// proxy, and copy `~/.codex/auth.json` in if present (absence is not fatal).
fn materialize_codex_home(
    model: &str,
    port: u16,
    api_key_env: &str,
) -> Result<PathBuf, ProxyError> {
    // NOT under $TMPDIR: codex refuses to create its PATH-alias helper binaries
    // inside a temp dir (and then the model exec silently degrades). Use a stable
    // per-user cache dir instead.
    let base = home_dir()
        .map(|h| h.join(".cache").join("jesterky"))
        .unwrap_or_else(std::env::temp_dir);
    let home = base.join(format!("proxy_{port}"));
    std::fs::create_dir_all(&home)?;

    let config = format!(
        "model = \"{model}\"\n\
         model_provider = \"jesterky_local_proxy\"\n\
         model_context_window = 128000\n\
         model_max_output_tokens = 16000\n\
         [model_providers.jesterky_local_proxy]\n\
         name = \"jesterky_local_proxy\"\n\
         base_url = \"http://127.0.0.1:{port}/v1\"\n\
         env_key = \"{api_key_env}\"\n\
         wire_api = \"responses\"\n"
    );
    std::fs::write(home.join("config.toml"), config)?;

    // codex may still want a session auth file; copy it if the user has one.
    if let Some(user_home) = home_dir() {
        let auth = user_home.join(".codex").join("auth.json");
        if auth.is_file() {
            let _ = std::fs::copy(&auth, home.join("auth.json"));
        }
    }

    Ok(home)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_route_returns_none() {
        // gpt-* is native codex; no proxy, and no env var read.
        let proxy = ChatProxy::spawn("gpt-5.5").await.expect("ok");
        assert!(proxy.is_none());
    }

    #[test]
    fn config_toml_pins_the_proxy() {
        let dir = std::env::temp_dir().join("jesterky_proxy_cfgtest_0");
        let _ = std::fs::remove_dir_all(&dir);
        let home =
            materialize_codex_home("deepseek/deepseek-v4-pro-direct", 54321, "DEEPSEEK_API_KEY")
                .expect("materialize");
        let cfg = std::fs::read_to_string(home.join("config.toml")).expect("config");
        assert!(cfg.contains("model = \"deepseek/deepseek-v4-pro-direct\""));
        assert!(cfg.contains("model_provider = \"jesterky_local_proxy\""));
        assert!(cfg.contains("base_url = \"http://127.0.0.1:54321/v1\""));
        assert!(cfg.contains("env_key = \"DEEPSEEK_API_KEY\""));
        assert!(cfg.contains("wire_api = \"responses\""));
        let _ = std::fs::remove_dir_all(&home);
    }
}
