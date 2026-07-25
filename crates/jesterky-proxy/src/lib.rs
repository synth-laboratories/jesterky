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
//!     // Point codex at the host-owned config and supply only its scoped local
//!     // client value; the upstream key remains inside `proxy`.
//!     let _home = proxy.codex_home();
//!     let _client = (proxy.client_env_name(), proxy.client_credential());
//!     // Keep `proxy` alive for the whole run; dropping it aborts the server.
//! }
//! # Ok(()) }
//! ```

mod convert;
mod route;
mod server;
mod sse;

pub use convert::ConvertError;
pub use route::{
    is_native_chatgpt_model, resolve_route, resolve_route_checked, ProviderKind, ProviderRoute,
};

use server::ServerState;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// API-key-shaped, per-proxy child capability used only for Codex → ChatProxy
/// calls. The real upstream provider key never leaves the trusted proxy process.
pub const CHAT_PROXY_CLIENT_ENV: &str = "JESTERKY_PROXY_CLIENT_KEY";

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
    /// A stable per-user cache cannot be selected without a home directory.
    #[error("unable to materialize proxy CODEX_HOME: user home directory is unavailable")]
    MissingHome,
    /// Secure random proxy-client capability generation failed.
    #[error("unable to generate proxy-client capability: {0}")]
    Entropy(String),
    /// A custom provider route is incomplete or invalid.
    #[error(transparent)]
    Route(#[from] route::RouteError),
}

/// Opaque, host-issued binding for one running [`ChatProxy`].
///
/// Only `ChatProxy` can construct this value. Consumers may give its scoped
/// client capability to Codex, but it contains no upstream provider credential.
#[derive(Clone, Debug)]
pub struct ChatProxyBinding {
    port: u16,
    codex_home: PathBuf,
    client_credential: String,
}

impl ChatProxyBinding {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn codex_home(&self) -> &Path {
        &self.codex_home
    }

    pub fn client_env_name(&self) -> &'static str {
        CHAT_PROXY_CLIENT_ENV
    }

    pub fn client_credential(&self) -> &str {
        &self.client_credential
    }
}

/// A running proxy. Aborts its server task on drop. Keep it alive for the whole run.
pub struct ChatProxy {
    task: JoinHandle<()>,
    binding: ChatProxyBinding,
}

impl ChatProxy {
    /// Resolve `model` to a provider, bind an ephemeral localhost port, spawn the
    /// server, and materialize a host-owned `CODEX_HOME` whose `config.toml` points
    /// codex at this proxy (`wire_api = "responses"`). Returns `Ok(None)` if the
    /// model has no proxy mapping (native codex route). Errors if the provider's
    /// `api_key_env` is unset.
    pub async fn spawn(model: &str) -> Result<Option<ChatProxy>, ProxyError> {
        let route = match resolve_route_checked(model)? {
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

        let client_credential = proxy_client_credential()?;
        let codex_home = materialize_codex_home(model, port)?;

        let state = Arc::new(ServerState {
            route,
            api_key,
            client_credential: client_credential.clone(),
            port,
            counter: AtomicU64::new(0),
            client: reqwest::Client::new(),
            codex_model: model.to_string(),
            signatures: std::sync::Mutex::new(std::collections::HashMap::new()),
        });

        let task = tokio::spawn(server::serve(listener, state));

        Ok(Some(ChatProxy {
            task,
            binding: ChatProxyBinding {
                port,
                codex_home,
                client_credential,
            },
        }))
    }

    /// The bound localhost port.
    pub fn port(&self) -> u16 {
        self.binding.port()
    }

    /// The generated `CODEX_HOME` dir (contains `config.toml`). Pass this to codex
    /// via the `CODEX_HOME` environment variable.
    pub fn codex_home(&self) -> &Path {
        self.binding.codex_home()
    }

    /// Trusted loopback base URL pinned for the Codex child.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.binding.port())
    }

    /// Per-proxy child capability accepted only by this loopback server. It is
    /// distinct from, and cannot authorize against, the upstream.
    pub fn client_credential(&self) -> &str {
        self.binding.client_credential()
    }

    /// Fixed child environment name paired with [`Self::client_credential`].
    pub fn client_env_name(&self) -> &'static str {
        self.binding.client_env_name()
    }

    /// Opaque child binding for trusted host integrations.
    pub fn binding(&self) -> ChatProxyBinding {
        self.binding.clone()
    }
}

impl Drop for ChatProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Write a fresh host-owned CODEX_HOME with a `config.toml` that pins Codex to
/// this proxy. Non-native routes intentionally receive no ChatGPT auth bundle.
fn materialize_codex_home(model: &str, port: u16) -> Result<PathBuf, ProxyError> {
    // NOT under $TMPDIR: codex refuses to create its PATH-alias helper binaries
    // inside a temp dir (and then the model exec silently degrades). Use a stable
    // per-user cache dir instead.
    let user_home = home_dir().ok_or(ProxyError::MissingHome)?;
    let base = user_home.join(".cache").join("jesterky");
    let home = base.join(format!("proxy_{port}"));
    if home.exists() {
        std::fs::remove_dir_all(&home)?;
    }
    std::fs::create_dir_all(&home)?;

    let config = format!(
        "model = \"{model}\"\n\
         model_provider = \"jesterky_local_proxy\"\n\
         model_context_window = 128000\n\
         model_max_output_tokens = 16000\n\
         [model_providers.jesterky_local_proxy]\n\
         name = \"jesterky_local_proxy\"\n\
         base_url = \"http://127.0.0.1:{port}/v1\"\n\
         env_key = \"{CHAT_PROXY_CLIENT_ENV}\"\n\
         wire_api = \"responses\"\n"
    );
    std::fs::write(home.join("config.toml"), config)?;

    Ok(home)
}

fn proxy_client_credential() -> Result<String, ProxyError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| ProxyError::Entropy(error.to_string()))?;
    Ok(bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
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
        // The GPT-5 ChatGPT family is native Codex; no proxy or key lookup.
        let proxy = ChatProxy::spawn("gpt-5.5").await.expect("ok");
        assert!(proxy.is_none());
    }

    #[test]
    fn config_toml_pins_the_proxy() {
        let home =
            materialize_codex_home("deepseek/deepseek-v4-pro-direct", 54321).expect("materialize");
        let cfg = std::fs::read_to_string(home.join("config.toml")).expect("config");
        assert!(cfg.contains("model = \"deepseek/deepseek-v4-pro-direct\""));
        assert!(cfg.contains("model_provider = \"jesterky_local_proxy\""));
        assert!(cfg.contains("base_url = \"http://127.0.0.1:54321/v1\""));
        assert!(cfg.contains("env_key = \"JESTERKY_PROXY_CLIENT_KEY\""));
        assert!(!cfg.contains("DEEPSEEK_API_KEY"));
        assert!(!cfg.contains("GEMINI_API_KEY"));
        assert!(cfg.contains("wire_api = \"responses\""));
        assert!(!home.join("auth.json").exists());
        let first = proxy_client_credential().expect("secure client capability");
        let second = proxy_client_credential().expect("second secure client capability");
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&home);
    }
}
