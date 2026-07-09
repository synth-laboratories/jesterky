//! Route resolution: map a codex-facing model id to a provider chat endpoint.

/// Where a model route sends its chat/completions calls.
#[derive(Clone, Debug)]
pub struct ProviderRoute {
    /// Full chat/completions URL, e.g. "https://api.deepseek.com/v1/chat/completions".
    pub chat_url: String,
    /// Env var holding the bearer key, e.g. "DEEPSEEK_API_KEY".
    pub api_key_env: String,
    /// The model id to send upstream in the chat payload (may differ from the
    /// codex-facing route id).
    pub upstream_model: String,
    /// Whether the provider's chat API accepts `response_format: json_schema`
    /// (strict structured outputs). DeepSeek does NOT (only `json_object`); when
    /// false the proxy downgrades to `json_object` and injects the schema into the
    /// prompt so the model still produces the right shape.
    pub supports_json_schema: bool,
}

const DEEPSEEK_CHAT_URL: &str = "https://api.deepseek.com/v1/chat/completions";
const GEMINI_CHAT_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

/// Resolve a codex route id to a provider chat endpoint. Built-in defaults:
///  - `deepseek/...`  -> deepseek.com chat, `DEEPSEEK_API_KEY`, upstream `deepseek-chat`.
///  - `gemini/<id>` or `gemini-<id>` -> Google OpenAI-compat chat, `GEMINI_API_KEY`,
///    upstream = the id after `gemini/` (or the whole `gemini-...` string).
///
/// Returns `None` for a route with no built-in mapping (e.g. `gpt-5.5` — native
/// codex, no proxy).
///
/// Env override: if `JESTERKY_PROXY_CHAT_URL` / `JESTERKY_PROXY_KEY_ENV` /
/// `JESTERKY_PROXY_UPSTREAM_MODEL` are ALL set, they provide a generic mapping for
/// any non-`gpt` route that has no built-in mapping.
pub fn resolve_route(model: &str) -> Option<ProviderRoute> {
    // gpt-* is native codex (Responses-native); never proxied.
    if model.starts_with("gpt") {
        return None;
    }

    if model.starts_with("deepseek/") || model == "deepseek" {
        // Every deepseek/* route maps to the single "deepseek-chat" upstream model.
        return Some(ProviderRoute {
            chat_url: DEEPSEEK_CHAT_URL.to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            upstream_model: "deepseek-chat".to_string(),
            // DeepSeek rejects json_schema ("response_format type unavailable"),
            // only json_object — the proxy downgrades + injects the schema.
            supports_json_schema: false,
        });
    }

    if let Some(rest) = model.strip_prefix("gemini/") {
        return Some(ProviderRoute {
            chat_url: GEMINI_CHAT_URL.to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            upstream_model: rest.to_string(),
            supports_json_schema: true,
        });
    }
    if model.starts_with("gemini-") {
        return Some(ProviderRoute {
            chat_url: GEMINI_CHAT_URL.to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            upstream_model: model.to_string(),
            supports_json_schema: true,
        });
    }

    // Generic provider via env override (fallback for otherwise-unmapped routes).
    generic_env_route()
}

fn generic_env_route() -> Option<ProviderRoute> {
    let chat_url = non_empty_env("JESTERKY_PROXY_CHAT_URL")?;
    let api_key_env = non_empty_env("JESTERKY_PROXY_KEY_ENV")?;
    let upstream_model = non_empty_env("JESTERKY_PROXY_UPSTREAM_MODEL")?;
    // Default to strict schema; opt out with JESTERKY_PROXY_JSON_SCHEMA=0 for a
    // provider (like DeepSeek) that only accepts json_object.
    let supports_json_schema = std::env::var("JESTERKY_PROXY_JSON_SCHEMA")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(true);
    Some(ProviderRoute {
        chat_url,
        api_key_env,
        upstream_model,
        supports_json_schema,
    })
}

fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_route_maps_to_deepseek_chat() {
        let r = resolve_route("deepseek/deepseek-v4-pro-direct").expect("route");
        assert_eq!(r.chat_url, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(r.api_key_env, "DEEPSEEK_API_KEY");
        assert_eq!(r.upstream_model, "deepseek-chat");

        let flash = resolve_route("deepseek/deepseek-v4-flash-direct").expect("route");
        assert_eq!(flash.upstream_model, "deepseek-chat");
    }

    #[test]
    fn gpt_route_is_native_none() {
        assert!(resolve_route("gpt-5.5").is_none());
    }

    #[test]
    fn gemini_slash_and_dash_forms() {
        let slash = resolve_route("gemini/gemini-2.5-pro").expect("route");
        assert_eq!(
            slash.chat_url,
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        assert_eq!(slash.api_key_env, "GEMINI_API_KEY");
        assert_eq!(slash.upstream_model, "gemini-2.5-pro");

        let dash = resolve_route("gemini-2.5-flash").expect("route");
        assert_eq!(dash.upstream_model, "gemini-2.5-flash");
    }
}
