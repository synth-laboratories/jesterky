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
pub mod limiter;

pub use codex::CodexModel;
pub use limiter::AdaptiveLimiter;

use async_trait::async_trait;
use jesterky_contract::{LiveBus, NodePath};
use jesterky_core::{Actor, ActorRequest, ActorResult, HostError};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// How many times [`ModelActor`] re-asks a model whose reply didn't parse as a
/// JSON object. The flake is non-deterministic, so a few tries recover it; keep it
/// small so a genuinely broken actor fails fast.
const PARSE_ATTEMPTS: u32 = 3;
/// Appended to the role prompt on a re-ask after a parse failure.
const PARSE_NUDGE: &str = "\n\nCRITICAL: your previous reply could not be parsed as JSON. \
     Respond with EXACTLY ONE JSON object and NOTHING else — no prose, no explanation, \
     no markdown fences.";
/// Appended to the role prompt on a re-ask after the reply parsed but violated
/// the declared output schema. Carries the specific violation so the model can fix it.
const SCHEMA_NUDGE: &str = "\n\nCRITICAL: your previous JSON did not match the required output \
     schema. Fix it and respond with EXACTLY ONE JSON object matching the schema. Violation: ";

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
    /// Optional JSON Schema file for codex `--output-schema` (per-actor).
    pub output_schema: Option<PathBuf>,
    /// The shard's node path (host-side identity) so a streaming model can key
    /// live progress onto the right shard. `None` for non-map calls.
    pub node_path: Option<NodePath>,
    /// Host-side live-progress stream (never serialized, never replayed). Set by
    /// [`ModelActor`] when a bus is registered; a streaming [`Model`] *publishes*
    /// tokens / steps / last-action here as its subprocess emits events, and the
    /// renderer folds the stream. A publish with no live consumer is a no-op.
    pub live: Option<Arc<LiveBus>>,
    /// The seeded execution workspace for this call, if the actor declared a
    /// sandbox. An agentic [`Model`] (codex) runs `--cd` here at `mode()`; reused
    /// across parse-retries so the agent's incremental work persists. `None` =
    /// no sandbox (the model runs in its own default cwd, as before).
    pub sandbox: Option<Arc<dyn jesterky_sandbox::Sandbox>>,
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

impl ModelError {
    /// Worth retrying the *same* route (not a fallback): a transient blip or a
    /// rate-limit. Auth/config/parse are deterministic — retrying just wastes time.
    pub fn is_retryable(&self) -> bool {
        matches!(self, ModelError::Transient(_) | ModelError::Quota(_))
    }

    /// A rate-limit / 429 specifically — the signal to back the concurrency
    /// ceiling off (multiplicative decrease), not just retry.
    pub fn is_rate_limit(&self) -> bool {
        matches!(self, ModelError::Quota(_))
    }
}

/// A host [`Actor`] backed by a [`Model`]. Generic over the model so the real
/// codex path and the test stub share one adapter — the actor logic (prompt
/// build, JSON parse, error classification) is exercised without a network.
pub struct ModelActor<M: Model> {
    model: M,
    roles: HashMap<String, String>,
    output_schemas: HashMap<String, PathBuf>,
    /// Actor name → sandbox declaration (host-only; from `HostConfig.sandboxes`).
    sandboxes: HashMap<String, jesterky_contract::sandbox::SandboxConfig>,
    /// Spec directory — `seed.copy_from` paths resolve against it.
    spec_dir: PathBuf,
    live: Option<Arc<LiveBus>>,
}

impl<M: Model> ModelActor<M> {
    pub fn new(model: M) -> Self {
        Self {
            model,
            roles: HashMap::new(),
            output_schemas: HashMap::new(),
            sandboxes: HashMap::new(),
            spec_dir: PathBuf::from("."),
            live: None,
        }
    }

    /// Register a sandbox declaration for an actor. When set, `drive` seeds a
    /// fresh workspace, runs the model in it, and captures per the config.
    pub fn with_sandbox(
        mut self,
        actor: impl Into<String>,
        config: jesterky_contract::sandbox::SandboxConfig,
    ) -> Self {
        self.sandboxes.insert(actor.into(), config);
        self
    }

    /// The directory `seed.copy_from` paths resolve against (the spec's dir).
    pub fn with_spec_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.spec_dir = dir.into();
        self
    }

    /// Register a host-side [`LiveBus`] so a streaming model *publishes* live
    /// per-shard tokens / steps / last-action, keyed by the shard's node path.
    pub fn with_live(mut self, bus: Arc<LiveBus>) -> Self {
        self.live = Some(bus);
        self
    }

    /// Register a system prompt for an actor name. Unregistered actors get a
    /// generic instruction built from their name + inputs.
    pub fn with_role(mut self, actor: impl Into<String>, system: impl Into<String>) -> Self {
        self.roles.insert(actor.into(), system.into());
        self
    }

    /// Register a JSON Schema path for an actor (codex `--output-schema`).
    pub fn with_output_schema(
        mut self,
        actor: impl Into<String>,
        schema: impl Into<PathBuf>,
    ) -> Self {
        self.output_schemas.insert(actor.into(), schema.into());
        self
    }
}

#[async_trait]
impl<M: Model> Actor for ModelActor<M> {
    async fn drive(&self, req: ActorRequest) -> Result<ActorResult, HostError> {
        let base_system = self.roles.get(&req.actor).cloned();
        // Load the declared output schema (if any) once, up front. Host-side
        // validation covers EVERY backend uniformly — the codex CLI enforces
        // `--output-schema` natively, but stub/proxy models do not, so a declared
        // schema is meaningless without this check. An unreadable/invalid schema
        // file is a config error: surface it immediately, do not retry.
        let schema = match self.output_schemas.get(&req.actor) {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|err| HostError::Actor {
                    actor: req.actor.clone(),
                    message: format!("output schema {} unreadable: {err}", path.display()),
                })?;
                let value = serde_json::from_str::<serde_json::Value>(&text).map_err(|err| {
                    HostError::Actor {
                        actor: req.actor.clone(),
                        message: format!(
                            "output schema {} is not valid JSON: {err}",
                            path.display()
                        ),
                    }
                })?;
                Some(value)
            }
            None => None,
        };
        // Seed a fresh execution workspace if the actor declared a sandbox. One
        // sandbox per invocation (per map shard); it spans the parse-retry loop so
        // the agent's incremental work persists, and is dropped (cleaned up) when
        // `drive` returns.
        let sandbox_cfg = self.sandboxes.get(&req.actor).cloned();
        let sandbox: Option<Arc<dyn jesterky_sandbox::Sandbox>> = match &sandbox_cfg {
            Some(cfg) => {
                let files = cfg
                    .seed
                    .files_input
                    .as_deref()
                    // Dotted path so a spec can seed from a nested field (e.g.
                    // `job.files`), not just a top-level input.
                    .and_then(|field| field.split('.').try_fold(&req.inputs, |v, k| v.get(k)))
                    .and_then(|v| serde_json::from_value::<Vec<jesterky_sandbox::FileBlob>>(v.clone()).ok());
                let ctx = jesterky_sandbox::SeedCtx {
                    spec_dir: self.spec_dir.clone(),
                    files,
                };
                let provider = jesterky_sandbox::provider_for(cfg)
                    .map_err(|e| HostError::Actor { actor: req.actor.clone(), message: e.to_string() })?;
                let sb = provider
                    .create(cfg, &ctx)
                    .await
                    .map_err(|e| HostError::Actor { actor: req.actor.clone(), message: format!("sandbox create: {e}") })?;
                Some(Arc::from(sb))
            }
            None => None,
        };

        let mut mreq = ModelRequest {
            actor: req.actor.clone(),
            system: base_system.clone(),
            inputs: req.inputs,
            output_schema: self.output_schemas.get(&req.actor).cloned(),
            node_path: Some(req.addr.node_path.clone()),
            live: self.live.clone(),
            sandbox: sandbox.clone(),
        };
        // Reasoning models occasionally return prose / a truncated reply instead
        // of the JSON object — a non-deterministic ~few-percent flake. Since it's
        // not deterministic, a bounded re-ask (with an escalating nudge) recovers
        // almost all of them, so one flaky shard doesn't fail the whole map. A
        // *model* error (auth/quota/config) is not a parse flake — surface it at
        // once (the codex model already retried transients internally).
        let mut last_parse_err = String::new();
        for _ in 0..PARSE_ATTEMPTS {
            let raw = self
                .model
                .complete(&mreq)
                .await
                .map_err(|err| HostError::Actor {
                    actor: req.actor.clone(),
                    // ModelError's Display carries the class prefix (auth:/quota:/…).
                    message: err.to_string(),
                })?;
            match extract_json(&raw) {
                Ok(mut outputs) => {
                    // A parsed reply that violates the declared schema is a
                    // wrong-shape flake, handled like a parse failure: re-ask with
                    // the specific violation, then fail the node after the budget.
                    if let Some(schema) = &schema {
                        if let Err(violation) = validate_json(&outputs, schema) {
                            last_parse_err = format!("schema violation: {violation}");
                            mreq.system = Some(format!(
                                "{}{SCHEMA_NUDGE}{violation}",
                                base_system.as_deref().unwrap_or("")
                            ));
                            continue;
                        }
                    }
                    // Capture: lift the files the agent wrote to the workspace into
                    // the outputs (additive, after schema validation) so downstream
                    // nodes / scoring read them from the ledger. This is how a
                    // sandbox actor's real work product (a built crate, results)
                    // leaves the workspace — the JSON reply need only carry notes.
                    if let (Some(cfg), Some(sb)) = (&sandbox_cfg, &sandbox) {
                        if let Some(cap) = &cfg.capture {
                            let blobs = sb.collect(&cap.globs).await.map_err(|e| HostError::Actor {
                                actor: req.actor.clone(),
                                message: format!("sandbox capture: {e}"),
                            })?;
                            if let Some(obj) = outputs.as_object_mut() {
                                obj.insert(cap.into.clone(), serde_json::to_value(&blobs).unwrap_or_default());
                            }
                        }
                    }
                    // v1: the whole reply object is the outputs. score/signal are
                    // optimizer slots we do NOT synthesize here — code never grades
                    // actor work (house rule); a verifier fills them downstream.
                    return Ok(ActorResult {
                        outputs,
                        score: None,
                        signal: None,
                        artifacts: Vec::new(),
                    });
                }
                Err(err) => {
                    last_parse_err = err;
                    // Escalate the instruction for the next try (rebuilt from base,
                    // never stacked) so the model corrects its format.
                    mreq.system = Some(format!(
                        "{}{PARSE_NUDGE}",
                        base_system.as_deref().unwrap_or("")
                    ));
                }
            }
        }
        Err(HostError::Actor {
            actor: req.actor.clone(),
            message: format!(
                "model reply was not a JSON object after {PARSE_ATTEMPTS} attempts: {last_parse_err}"
            ),
        })
    }
}

/// Build the instruction a [`Model`] answers. Kept here (not in each `Model`) so
/// every backend gets the same contract: reply with exactly one JSON object.
pub fn build_prompt(req: &ModelRequest) -> String {
    let inputs =
        serde_json::to_string_pretty(&req.inputs).unwrap_or_else(|_| req.inputs.to_string());
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

/// Validate a value against a JSON-Schema **draft-07 subset** — dependency-free,
/// sized to the schemas jesterky workloads actually declare (`type`, `required`,
/// `properties`, `enum`, `additionalProperties`, `items`). Unknown keywords are
/// ignored (permissive), so a richer schema never spuriously rejects; the checks
/// that ARE implemented catch the real failure modes (missing required field,
/// wrong type, out-of-enum value, unexpected key). Returns the first violation.
pub fn validate_json(value: &serde_json::Value, schema: &serde_json::Value) -> Result<(), String> {
    let Some(schema) = schema.as_object() else {
        return Ok(()); // non-object schema (e.g. `true`): accept anything
    };
    if let Some(ty) = schema.get("type").and_then(|t| t.as_str()) {
        if !type_matches(value, ty) {
            return Err(format!("expected type `{ty}`, got `{}`", type_name(value)));
        }
    }
    if let Some(variants) = schema.get("enum").and_then(|e| e.as_array()) {
        if !variants.iter().any(|v| v == value) {
            return Err(format!(
                "value {value} is not one of the allowed enum variants"
            ));
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(|p| p.as_object());
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for field in required.iter().filter_map(|f| f.as_str()) {
                if !object.contains_key(field) {
                    return Err(format!("missing required field `{field}`"));
                }
            }
        }
        if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
            if let Some(properties) = properties {
                for key in object.keys() {
                    if !properties.contains_key(key) {
                        return Err(format!(
                            "unexpected field `{key}` (additionalProperties: false)"
                        ));
                    }
                }
            }
        }
        if let Some(properties) = properties {
            for (key, sub) in object {
                if let Some(sub_schema) = properties.get(key) {
                    validate_json(sub, sub_schema).map_err(|err| format!("`{key}`: {err}"))?;
                }
            }
        }
    }
    if let (Some(items_schema), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            validate_json(item, items_schema).map_err(|err| format!("[{index}]: {err}"))?;
        }
    }
    Ok(())
}

fn type_matches(value: &serde_json::Value, ty: &str) -> bool {
    match ty {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        _ => true, // unknown type keyword: don't reject
    }
}

fn type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Pull the answer JSON object out of a model reply.
///
/// Reasoning models don't obey "exactly one JSON object": they wrap the answer in
/// `<reasoning>…</reasoning>`, ```json fences, or split it across several messages,
/// and those preludes often contain their own braces. A naïve first-`{`-to-last-`}`
/// span then straddles a reasoning brace and the answer and fails with "extra
/// data". Instead: try the whole string, then scan for every *balanced* top-level
/// `{…}` (respecting string literals) and return the **last** one that parses as an
/// object — codex emits the final answer last. Returns the object or a reason.
pub fn extract_json(raw: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if value.is_object() {
            return Ok(value);
        }
    }
    let mut last_err = "no JSON object found in reply".to_string();
    let mut found_span = false;
    for span in balanced_object_spans(trimmed).into_iter().rev() {
        found_span = true;
        match serde_json::from_str::<serde_json::Value>(span) {
            Ok(value) if value.is_object() => return Ok(value),
            Ok(_) => {}
            Err(err) => last_err = format!("a brace span did not parse: {err}"),
        }
    }
    if !found_span {
        last_err = "no JSON object found in reply".to_string();
    }
    Err(last_err)
}

/// Every top-level balanced `{…}` substring, in order. Braces inside string
/// literals (and their escapes) are ignored, so quoted `{`/`}` never miscount.
fn balanced_object_spans(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut spans = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        // `{` and `}` are ASCII, so these are char boundaries.
                        spans.push(&s[start..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    spans
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
