//! `jesterky-model` — the first *real* host actor (M1→M2).
//!
//! The core defines the [`Actor`] seam (ADR #6); `FakeActor` proves the runtime
//! executes a topology. This crate turns the seam into a live model call so
//! jesterky runs a real workload. The design is a two-layer split so the
//! judgment-heavy IO stays isolated and the actor logic stays unit-testable:
//!
//! - [`Model`] — a single async completion (`prompt -> text`). The IO boundary.
//! - [`ModelActor`] — implements [`Actor`] over any [`Model`]: it builds the
//!   prompt from an [`ActorRequest`], parses the model's reply into typed
//!   outputs, and maps failures to a classified [`HostError`]. No IO of its own,
//!   so it is tested against [`StubModel`] with zero network.
//! - [`CodexModel`](codex::CodexModel) — the real [`Model`]: drives `codex exec`
//!   with the ChatGPT-bundle auth (`~/.codex/auth.json`). **Never an OpenAI API
//!   key** (house rule). This is the only model access we have here today; a
//!   DeepSeek-through-proxy `Model` can slot in beside it later without touching
//!   `ModelActor`.
//!
//! Recorded for replay like any actor (ADR #7): the runner records
//! `ModelActor`'s `ActorResult` by `Addr`, so a `ModelActor` run replays through
//! `ReplayActor` with no live model — the whole point of the seam.

pub mod codex;

pub use codex::CodexModel;

use async_trait::async_trait;
use jesterky_core::{Actor, ActorRequest, ActorResult, HostError};
use std::collections::HashMap;

/// One model completion. The IO boundary: a real impl talks to a model host, a
/// stub returns canned text. Errors are **classified** (auth / quota / config /
/// transient / parse) so the actor can surface the failure kind, not just a
/// string (house rule: informative errors).
#[async_trait]
pub trait Model: Send + Sync {
    async fn complete(&self, req: &ModelRequest) -> Result<String, ModelError>;
}

/// What [`ModelActor`] hands a [`Model`]: the actor role, its optional system
/// prompt, and the resolved typed inputs for this invocation.
#[derive(Debug, Clone)]
pub struct ModelRequest {
    /// The topology actor name (e.g. `quality_auditor`) — the role to play.
    pub actor: String,
    /// Optional per-role system prompt, set via [`ModelActor::with_role`].
    pub system: Option<String>,
    /// The resolved, typed inputs the model should act on.
    pub inputs: serde_json::Value,
}

/// A model failure, classified by kind so callers can react (retry a transient,
/// re-auth an auth failure, switch route on quota — house rule
/// `out_of_money_switch_route`).
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("auth: {0}")]
    Auth(String),
    #[error("quota: {0}")]
    Quota(String),
    #[error("config: {0}")]
    Config(String),
    #[error("transient: {0}")]
    Transient(String),
    #[error("parse: {0}")]
    Parse(String),
}

/// A host [`Actor`] backed by a [`Model`]. Generic over the model so the real
/// codex path and the test stub share one adapter — the actor logic (prompt
/// build, JSON parse, error classification) is exercised without a network.
pub struct ModelActor<M: Model> {
    model: M,
    roles: HashMap<String, String>,
}

impl<M: Model> ModelActor<M> {
    pub fn new(model: M) -> Self {
        Self {
            model,
            roles: HashMap::new(),
        }
    }

    /// Register a system prompt for an actor name. Unregistered actors get a
    /// generic instruction built from their name + inputs.
    pub fn with_role(mut self, actor: impl Into<String>, system: impl Into<String>) -> Self {
        self.roles.insert(actor.into(), system.into());
        self
    }
}

#[async_trait]
impl<M: Model> Actor for ModelActor<M> {
    async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError> {
        let mreq = ModelRequest {
            actor: req.actor.clone(),
            system: self.roles.get(&req.actor).cloned(),
            inputs: req.inputs,
        };
        let raw = self.model.complete(&mreq).await.map_err(|err| HostError::Actor {
            actor: req.actor.clone(),
            // ModelError's Display carries the class prefix (auth:/quota:/…).
            message: err.to_string(),
        })?;
        let outputs = extract_json(&raw).map_err(|err| HostError::Actor {
            actor: req.actor.clone(),
            message: format!("model reply was not a JSON object: {err}"),
        })?;
        // v1: the whole reply object is the outputs. score/signal are optimizer
        // slots we do NOT synthesize from the model here — code never grades
        // actor work (house rule: no code actor-quality verdicts); a verifier
        // fills them downstream.
        Ok(ActorResult {
            outputs,
            score: None,
            signal: None,
            artifacts: Vec::new(),
        })
    }
}

/// Build the instruction a [`Model`] answers. Kept here (not in each `Model`) so
/// every backend gets the same contract: reply with exactly one JSON object.
pub fn build_prompt(req: &ModelRequest) -> String {
    let inputs = serde_json::to_string_pretty(&req.inputs).unwrap_or_else(|_| req.inputs.to_string());
    let mut prompt = String::new();
    if let Some(system) = &req.system {
        prompt.push_str(system);
        prompt.push_str("\n\n");
    }
    prompt.push_str(&format!(
        "You are the workflow actor `{}`.\n\
         Act on the inputs below and respond with EXACTLY ONE JSON object and nothing else — \
         no prose, no markdown fences. The object's fields are this actor's outputs.\n\n\
         Inputs:\n{}\n",
        req.actor, inputs
    ));
    prompt
}

/// Pull a single JSON object out of a model reply. Tolerant of surrounding prose
/// or ```json fences (models add them): tries the whole string first, then the
/// span from the first `{` to the last `}`. Returns the parsed value or a reason.
pub fn extract_json(raw: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if value.is_object() {
            return Ok(value);
        }
    }
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    match (start, end) {
        (Some(start), Some(end)) if end > start => {
            let candidate = &trimmed[start..=end];
            serde_json::from_str::<serde_json::Value>(candidate)
                .map_err(|err| format!("found a brace span but it did not parse: {err}"))
        }
        _ => Err("no JSON object found in reply".to_string()),
    }
}

/// A [`Model`] test double: answers from an injected closure. The model-side
/// counterpart to `jesterky_actor::FakeActor` — lets `ModelActor` tests run with
/// zero network.
pub struct StubModel {
    #[allow(clippy::type_complexity)]
    reply: Box<dyn Fn(&ModelRequest) -> Result<String, ModelError> + Send + Sync>,
}

impl StubModel {
    pub fn new(
        reply: impl Fn(&ModelRequest) -> Result<String, ModelError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            reply: Box::new(reply),
        }
    }

    /// Always answer with this text (echoing back the inputs, say).
    pub fn replying(text: impl Into<String>) -> Self {
        let text = text.into();
        Self::new(move |_| Ok(text.clone()))
    }
}

#[async_trait]
impl Model for StubModel {
    async fn complete(&self, req: &ModelRequest) -> Result<String, ModelError> {
        (self.reply)(req)
    }
}
