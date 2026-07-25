//! Hand-rolled minimal HTTP/1.1 server. codex is the only client: one request
//! per connection, `Connection: close`, ephemeral localhost bind.

use crate::convert::{ResponsesRequest, responses_request_to_chat_payload};
use crate::route::ProviderRoute;
use crate::sse::{ChatResponse, build_events_validated, final_response_object, hex24};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Shared server state: the resolved provider route, the bearer key (read once at
/// spawn), the bound port, and a per-request id counter.
pub(crate) struct ServerState {
    pub route: ProviderRoute,
    pub api_key: String,
    /// Per-proxy bearer capability accepted from Codex. This is never sent
    /// upstream; `api_key` remains the only upstream authority.
    pub client_credential: String,
    pub port: u16,
    pub counter: AtomicU64,
    pub client: reqwest::Client,
    /// The codex-facing model id (what codex has in its config `-m`). Advertised
    /// by `GET /v1/models` so codex's model-refresh validates and proceeds.
    pub codex_model: String,
    /// Gemini (and other thinking models) return an opaque `thought_signature`
    /// on each tool call and REQUIRE it echoed back on later turns. It has no
    /// slot in the OpenAI/Responses tool protocol, so codex drops it — we keep a
    /// `tool_call_id -> signature` map here and re-inject it on the way out.
    pub signatures: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

/// Accept loop. Runs until the task is aborted (on `ChatProxy` drop).
pub(crate) async fn serve(listener: TcpListener, state: Arc<ServerState>) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let _ = handle_connection(stream, state).await;
                });
            }
            Err(err) => {
                eprintln!("jesterky proxy accept error: {err}");
                continue;
            }
        }
    }
}

struct Request {
    method: HttpMethod,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    fn parse(raw: &str) -> std::io::Result<Self> {
        match raw {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            other => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unsupported HTTP method `{other}`"),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxyEndpoint {
    Health,
    Models,
    Responses,
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProxyErrorCode {
    MalformedRequest,
    Unauthorized,
    NotFound,
    InvalidJson,
    InvalidResponsesRequest,
    ConversionFailed,
    UpstreamTransport,
    UpstreamStatus,
    UpstreamJson,
    UpstreamSchema,
}

#[derive(Debug, Serialize)]
struct ProxyErrorBody<'a> {
    error: ProxyErrorDetail<'a>,
}

#[derive(Debug, Serialize)]
struct ProxyErrorDetail<'a> {
    code: ProxyErrorCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_status: Option<u16>,
}

impl<'a> ProxyErrorBody<'a> {
    fn new(code: ProxyErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ProxyErrorDetail {
                code,
                message: message.into(),
                provider: None,
                upstream_status: None,
            },
        }
    }

    fn provider(
        code: ProxyErrorCode,
        route: &'a ProviderRoute,
        message: impl Into<String>,
    ) -> Self {
        Self {
            error: ProxyErrorDetail {
                code,
                message: message.into(),
                provider: Some(route.provider.as_str()),
                upstream_status: None,
            },
        }
    }

    fn upstream_status(route: &'a ProviderRoute, status: u16, message: impl Into<String>) -> Self {
        Self {
            error: ProxyErrorDetail {
                code: ProxyErrorCode::UpstreamStatus,
                message: message.into(),
                provider: Some(route.provider.as_str()),
                upstream_status: Some(status),
            },
        }
    }
}

fn classify_endpoint(method: HttpMethod, path: &str) -> ProxyEndpoint {
    let route = path.trim_end_matches('/');
    match method {
        HttpMethod::Get if route.ends_with("/health") => ProxyEndpoint::Health,
        HttpMethod::Get if route.ends_with("/models") => ProxyEndpoint::Models,
        HttpMethod::Post if route.ends_with("/responses") => ProxyEndpoint::Responses,
        _ => ProxyEndpoint::NotFound,
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<ServerState>) -> std::io::Result<()> {
    let request = match read_request(&mut stream).await {
        Ok(Some(req)) => req,
        Ok(None) => return Ok(()),
        Err(err) => {
            write_error(
                &mut stream,
                400,
                ProxyErrorBody::new(ProxyErrorCode::MalformedRequest, err.to_string()),
            )
            .await?;
            return Ok(());
        }
    };

    let path = request
        .path
        .split_once('?')
        .map(|(path, _query)| path)
        .unwrap_or(request.path.as_str())
        .to_string();

    let endpoint = classify_endpoint(request.method, &path);
    if matches!(endpoint, ProxyEndpoint::Models | ProxyEndpoint::Responses)
        && !request_is_authorized(request.authorization.as_deref(), &state.client_credential)
    {
        return write_error(
            &mut stream,
            401,
            ProxyErrorBody::new(
                ProxyErrorCode::Unauthorized,
                "missing or invalid Jesterky proxy-client capability",
            ),
        )
        .await;
    }

    match endpoint {
        ProxyEndpoint::Health => write_json(&mut stream, 200, &json!({"status": "ok"})).await,
        // codex refreshes its model catalog before an agentic session and decodes the
        // response into its OWN rich catalog schema — a 404 or a lean OpenAI-style
        // `{data:[...]}` both abort the run. Serve one entry in codex's catalog shape
        // (cloned from its built-in schema) for the model this proxy serves.
        ProxyEndpoint::Models => {
            write_json(&mut stream, 200, &models_catalog(&state.codex_model)).await
        }
        ProxyEndpoint::Responses => handle_responses(&mut stream, state, &request.body).await,
        ProxyEndpoint::NotFound => {
            write_error(
                &mut stream,
                404,
                ProxyErrorBody::new(ProxyErrorCode::NotFound, "proxy endpoint not found"),
            )
            .await
        }
    }
}

fn request_is_authorized(authorization: Option<&str>, expected: &str) -> bool {
    let Some(raw) = authorization else {
        return false;
    };
    let Some((scheme, credential)) = raw.split_once(' ') else {
        return false;
    };
    scheme.eq_ignore_ascii_case("bearer")
        && !credential.is_empty()
        && constant_time_eq(credential.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(candidate: &[u8], expected: &[u8]) -> bool {
    let width = candidate.len().max(expected.len());
    let mut difference = candidate.len() ^ expected.len();
    for index in 0..width {
        let candidate_byte = candidate.get(index).copied().unwrap_or(0);
        let expected_byte = expected.get(index).copied().unwrap_or(0);
        difference |= usize::from(candidate_byte ^ expected_byte);
    }
    difference == 0
}

async fn handle_responses(
    stream: &mut TcpStream,
    state: Arc<ServerState>,
    body_bytes: &[u8],
) -> std::io::Result<()> {
    let body: Value = if body_bytes.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(body_bytes) {
            Ok(v) => v,
            Err(e) => {
                return write_error(
                    stream,
                    400,
                    ProxyErrorBody::new(
                        ProxyErrorCode::InvalidJson,
                        format!("bad request body: {e}"),
                    ),
                )
                .await;
            }
        }
    };
    let request = match ResponsesRequest::from_value(&body) {
        Ok(request) => request,
        Err(err) => {
            return write_error(
                stream,
                400,
                ProxyErrorBody::new(ProxyErrorCode::InvalidResponsesRequest, err.to_string()),
            )
            .await;
        }
    };
    let model = request.model().to_string();
    let is_streaming = request.stream_enabled();

    let mut chat_payload = match responses_request_to_chat_payload(
        &request,
        &state.route.upstream_model,
        state.route.supports_json_schema,
    ) {
        Ok(p) => p,
        Err(e) => {
            return write_error(
                stream,
                400,
                ProxyErrorBody::new(ProxyErrorCode::ConversionFailed, e.to_string()),
            )
            .await;
        }
    };

    // Re-attach any stored `thought_signature` onto the assistant tool_calls we're
    // sending back — Gemini rejects a multi-turn tool sequence without it.
    reattach_signatures(&mut chat_payload, &state.signatures);

    // Call the provider FIRST (buffered) so an upstream failure returns a real
    // HTTP error status instead of a half-open SSE stream — loud, not silent.
    let upstream = state
        .client
        .post(&state.route.chat_url)
        .header("Authorization", format!("Bearer {}", state.api_key))
        .header("Content-Type", "application/json")
        .json(&chat_payload)
        .send()
        .await;

    let resp = match upstream {
        Ok(r) => r,
        Err(e) => {
            return write_error(
                stream,
                502,
                ProxyErrorBody::provider(
                    ProxyErrorCode::UpstreamTransport,
                    &state.route,
                    format!("chat upstream error: {e}"),
                ),
            )
            .await;
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let code = status.as_u16();
        let detail = match resp.text().await {
            Ok(text) => text,
            Err(err) => format!("unable to read upstream error body: {err}"),
        };
        let detail: String = detail.chars().take(2000).collect();
        return write_error(
            stream,
            code,
            ProxyErrorBody::upstream_status(
                &state.route,
                code,
                format!("chat upstream failed: {detail}"),
            ),
        )
        .await;
    }

    let chat_response_json: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return write_error(
                stream,
                502,
                ProxyErrorBody::provider(
                    ProxyErrorCode::UpstreamJson,
                    &state.route,
                    format!("chat upstream returned non-JSON: {e}"),
                ),
            )
            .await;
        }
    };

    // Harvest `thought_signature`s from this response's tool calls so we can
    // re-attach them when codex echoes the same tool calls back next turn.
    harvest_signatures(&chat_response_json, &state.signatures);

    let chat_response = match ChatResponse::from_value(chat_response_json) {
        Ok(response) => response,
        Err(err) => {
            return write_error(
                stream,
                502,
                ProxyErrorBody::provider(
                    ProxyErrorCode::UpstreamSchema,
                    &state.route,
                    err.to_string(),
                ),
            )
            .await;
        }
    };

    let n = state.counter.fetch_add(1, Ordering::Relaxed);
    let rid = format!("resp_{}", hex24(n, state.port));
    let msg_id = format!("msg_{}", hex24(n.wrapping_add(1), state.port));

    if !is_streaming {
        let obj = match final_response_object(&chat_response, &model, &rid, &msg_id) {
            Ok(obj) => obj,
            Err(err) => {
                return write_error(
                    stream,
                    502,
                    ProxyErrorBody::provider(
                        ProxyErrorCode::UpstreamSchema,
                        &state.route,
                        err.to_string(),
                    ),
                )
                .await;
            }
        };
        return write_json(stream, 200, &obj).await;
    }

    let events = match build_events_validated(&chat_response, &model, &rid, &msg_id) {
        Ok(events) => events,
        Err(err) => {
            return write_error(
                stream,
                502,
                ProxyErrorBody::provider(
                    ProxyErrorCode::UpstreamSchema,
                    &state.route,
                    err.to_string(),
                ),
            )
            .await;
        }
    };
    let mut payload = String::new();
    for ev in &events {
        payload.push_str(&ev.frame());
    }
    write_raw(
        stream,
        200,
        "text/event-stream",
        &[("Cache-Control", "no-cache")],
        payload.as_bytes(),
    )
    .await
}

/// Store `tool_call_id -> thought_signature` for every tool call in a chat
/// response that carries one (Gemini puts it in `extra_content.google`).
fn harvest_signatures(
    chat_response: &Value,
    store: &std::sync::Mutex<std::collections::HashMap<String, String>>,
) {
    let calls = chat_response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("tool_calls"))
        .and_then(Value::as_array);
    if let Some(calls) = calls {
        if let Ok(mut map) = store.lock() {
            for call in calls {
                let id = call.get("id").and_then(Value::as_str);
                let sig = call
                    .get("extra_content")
                    .and_then(|e| e.get("google"))
                    .and_then(|g| g.get("thought_signature"))
                    .and_then(Value::as_str);
                if let (Some(id), Some(sig)) = (id, sig) {
                    map.insert(id.to_string(), sig.to_string());
                }
            }
        }
    }
}

/// Re-attach stored signatures onto assistant `tool_calls` in an outgoing chat
/// payload (by `tool_call.id`), so a multi-turn tool sequence satisfies Gemini.
fn reattach_signatures(
    chat_payload: &mut Value,
    store: &std::sync::Mutex<std::collections::HashMap<String, String>>,
) {
    let map = match store.lock() {
        Ok(m) if !m.is_empty() => m,
        _ => return,
    };
    let Some(messages) = chat_payload
        .get_mut("messages")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for msg in messages {
        let Some(calls) = msg.get_mut("tool_calls").and_then(Value::as_array_mut) else {
            continue;
        };
        for call in calls {
            let id = call.get("id").and_then(Value::as_str).map(str::to_string);
            if let Some(id) = id {
                if let Some(sig) = map.get(&id) {
                    call["extra_content"] = json!({"google": {"thought_signature": sig}});
                }
            }
        }
    }
}

/// codex's model-catalog response shape (`{"models":[<entry>]}`). The entry mirrors
/// codex's built-in catalog schema so its `StaticModelsManager` decodes it; only the
/// `slug`/`display_name` vary. Kept intentionally complete — codex rejects a lean
/// entry with `missing field ...`.
fn models_catalog(model: &str) -> Value {
    json!({
        "models": [{
            "slug": model,
            "display_name": model,
            "description": "Served via the jesterky Responses↔chat proxy.",
            "context_window": 128000,
            "max_context_window": 128000,
            "auto_compact_token_limit": null,
            "input_modalities": ["text"],
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": false,
            "prefer_websockets": false,
            "support_verbosity": false,
            "default_verbosity": "low",
            "apply_patch_tool_type": "freeform",
            "web_search_tool_type": "text_and_image",
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "minimal_client_version": "0.0.0",
            "priority": 0,
            "upgrade": null,
            "availability_nux": null,
            "reasoning_summary_format": "experimental",
            "default_reasoning_summary": "none",
            "default_reasoning_level": "medium",
            "truncation_policy": {"mode": "tokens", "limit": 10000},
            "supported_reasoning_levels": [
                {"effort": "low", "description": "Fast responses with lighter reasoning"},
                {"effort": "medium", "description": "Balances speed and reasoning depth"},
                {"effort": "high", "description": "Greater reasoning depth"}
            ],
            "base_instructions": "You are a coding agent. Use the provided tools to build and verify.",
            "supports_reasoning_summaries": false,
            "supports_search_tool": false,
            "additional_speed_tiers": [],
            "available_in_plans": [],
            "experimental_supported_tools": [],
            "service_tiers": [],
            "model_messages": {"instructions_template": "You are a coding agent. Use the provided tools to build and verify."},
        }]
    })
}

/// Read the request line + headers (until CRLFCRLF), then the body per
/// Content-Length. Returns Ok(None) on a cleanly closed empty connection.
async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut tmp = [0u8; 8192];

    // Read until we have the full header block.
    let header_end = loop {
        if let Some(pos) = find_subsequence(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before headers complete",
            ));
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > 8 * 1024 * 1024 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "header block too large",
            ));
        }
    };

    let header_text = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing HTTP request line")
    })?;
    let mut parts = request_line.split_whitespace();
    let method_raw = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing HTTP method")
        })?;
    let method = HttpMethod::parse(method_raw)?;
    let path = parts
        .next()
        .filter(|part| !part.is_empty())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing HTTP request path")
        })?
        .to_string();

    let mut content_length: usize = 0;
    let mut authorization: Option<String> = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().map_err(|err| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid Content-Length `{}`: {err}", value.trim()),
                    )
                })?;
            } else if name.trim().eq_ignore_ascii_case("authorization") {
                if authorization.is_some() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "duplicate Authorization header",
                    ));
                }
                authorization = Some(value.trim().to_string());
            }
        }
    }

    let body_start = header_end + 4;
    let mut body: Vec<u8> = buf[body_start..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "connection closed after {} body bytes, expected {content_length}",
                    body.len()
                ),
            ));
        }
        body.extend_from_slice(&tmp[..n]);
    }
    if body.len() > content_length {
        body.truncate(content_length);
    }

    Ok(Some(Request {
        method,
        path,
        authorization,
        body,
    }))
}

async fn write_json(stream: &mut TcpStream, code: u16, value: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(value).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unable to serialize JSON response: {err}"),
        )
    })?;
    write_raw(stream, code, "application/json", &[], &body).await
}

async fn write_error(
    stream: &mut TcpStream,
    http_status: u16,
    body: ProxyErrorBody<'_>,
) -> std::io::Result<()> {
    let value = serde_json::to_value(body).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unable to serialize JSON error response: {err}"),
        )
    })?;
    write_json(stream, http_status, &value).await
}

async fn write_raw(
    stream: &mut TcpStream,
    code: u16,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    let reason = reason_phrase(code);
    let mut head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    let _ = stream.shutdown().await;
    Ok(())
}

fn reason_phrase(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_client_capability_is_exact_and_not_upstream_authority() {
        let scoped = "0123456789abcdef";
        assert!(request_is_authorized(
            Some("Bearer 0123456789abcdef"),
            scoped
        ));
        assert!(request_is_authorized(
            Some("bearer 0123456789abcdef"),
            scoped
        ));
        assert!(!request_is_authorized(None, scoped));
        assert!(!request_is_authorized(
            Some("Bearer upstream-provider-key"),
            scoped
        ));
        assert!(!request_is_authorized(
            Some("Bearer 0123456789abcdee"),
            scoped
        ));
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
