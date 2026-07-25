//! Route resolution: map a codex-facing model id to a provider chat endpoint.

/// Where a model route sends its chat/completions calls.
#[derive(Clone, Debug)]
pub struct ProviderRoute {
    pub provider: ProviderKind,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    DeepSeek,
    Gemini,
    Custom,
}

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("missing custom proxy configuration: `{0}` is unset or empty")]
    MissingConfig(&'static str),
    #[error("invalid custom proxy configuration `{key}`: {message}")]
    InvalidConfig {
        key: &'static str,
        message: &'static str,
    },
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeepSeek => "deepseek",
            Self::Gemini => "gemini",
            Self::Custom => "custom",
        }
    }
}

const DEEPSEEK_CHAT_URL: &str = "https://api.deepseek.com/v1/chat/completions";
const GEMINI_CHAT_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelRouteId<'a> {
    Native,
    DeepSeek,
    Gemini(&'a str),
    Custom,
    Unknown,
}

impl<'a> ModelRouteId<'a> {
    fn parse(model: &'a str) -> Self {
        if is_native_chatgpt_model(model) {
            return Self::Native;
        }
        if model == "deepseek" {
            return Self::DeepSeek;
        }
        if let Some((namespace, upstream)) = model.split_once('/') {
            return match (namespace, upstream.is_empty()) {
                ("deepseek", false) => Self::DeepSeek,
                ("gemini", false) => Self::Gemini(upstream),
                ("custom" | "proxy", false) => Self::Custom,
                _ => Self::Unknown,
            };
        }
        if model.starts_with("gemini-") {
            return Self::Gemini(model);
        }
        Self::Unknown
    }
}

/// Whether `model` is an OpenAI GPT-5 family id that Codex may run with its
/// built-in `openai` provider and ChatGPT authentication.
///
/// This deliberately excludes broad `gpt-*` matching: `gpt-oss-*` and arbitrary
/// provider-like names must not bypass the managed-proxy fail-closed path.
pub fn is_native_chatgpt_model(model: &str) -> bool {
    let Some(suffix) = model.strip_prefix("gpt-5") else {
        return false;
    };
    if suffix.is_empty() {
        return true;
    }
    matches!(suffix.as_bytes().first(), Some(b'.' | b'-'))
        && suffix[1..]
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
}

/// Resolve a codex route id to a provider chat endpoint. Built-in defaults:
///  - `deepseek/...`  -> deepseek.com chat, `DEEPSEEK_API_KEY`, upstream `deepseek-chat`.
///  - `gemini/<id>` or `gemini-<id>` -> Google OpenAI-compat chat, `GEMINI_API_KEY`,
///    upstream = the id after `gemini/` (or the whole `gemini-...` string).
///
/// Returns `None` for a route with no built-in mapping (e.g. `gpt-5.5` — native
/// codex, no proxy).
///
/// Custom env route: only explicit `custom/...` or `proxy/...` model ids may use
/// `JESTERKY_PROXY_CHAT_URL` / `JESTERKY_PROXY_KEY_ENV` /
/// `JESTERKY_PROXY_UPSTREAM_MODEL`. Unknown model ids do not silently route.
pub fn resolve_route(model: &str) -> Option<ProviderRoute> {
    resolve_route_checked(model).ok().flatten()
}

pub fn resolve_route_checked(model: &str) -> Result<Option<ProviderRoute>, RouteError> {
    match ModelRouteId::parse(model) {
        ModelRouteId::Native | ModelRouteId::Unknown => Ok(None),
        ModelRouteId::DeepSeek => Ok(Some(ProviderRoute {
            provider: ProviderKind::DeepSeek,
            chat_url: DEEPSEEK_CHAT_URL.to_string(),
            api_key_env: "DEEPSEEK_API_KEY".to_string(),
            upstream_model: "deepseek-chat".to_string(),
            supports_json_schema: false,
        })),
        ModelRouteId::Gemini(upstream_model) => Ok(Some(ProviderRoute {
            provider: ProviderKind::Gemini,
            chat_url: GEMINI_CHAT_URL.to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            upstream_model: upstream_model.to_string(),
            supports_json_schema: true,
        })),
        ModelRouteId::Custom => custom_env_route().map(Some),
    }
}

fn custom_env_route() -> Result<ProviderRoute, RouteError> {
    let chat_url = required_env("JESTERKY_PROXY_CHAT_URL")?;
    if !(chat_url.starts_with("https://") || chat_url.starts_with("http://127.0.0.1:")) {
        return Err(RouteError::InvalidConfig {
            key: "JESTERKY_PROXY_CHAT_URL",
            message: "must use https:// or loopback http://127.0.0.1:",
        });
    }
    let api_key_env = required_env("JESTERKY_PROXY_KEY_ENV")?;
    if !api_key_env
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(RouteError::InvalidConfig {
            key: "JESTERKY_PROXY_KEY_ENV",
            message: "must contain only uppercase ASCII letters, digits, or underscores",
        });
    }
    let upstream_model = required_env("JESTERKY_PROXY_UPSTREAM_MODEL")?;
    // Default to strict schema; opt out with JESTERKY_PROXY_JSON_SCHEMA=0 for a
    // provider (like DeepSeek) that only accepts json_object.
    let supports_json_schema = custom_json_schema_capability()?;
    Ok(ProviderRoute {
        provider: ProviderKind::Custom,
        chat_url,
        api_key_env,
        upstream_model,
        supports_json_schema,
    })
}

fn custom_json_schema_capability() -> Result<bool, RouteError> {
    match std::env::var("JESTERKY_PROXY_JSON_SCHEMA") {
        Err(std::env::VarError::NotPresent) => Ok(true),
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "supported" => Ok(true),
            "0" | "false" | "unsupported" => Ok(false),
            _ => Err(RouteError::InvalidConfig {
                key: "JESTERKY_PROXY_JSON_SCHEMA",
                message: "must be one of 1, true, supported, 0, false, or unsupported",
            }),
        },
        Err(std::env::VarError::NotUnicode(_)) => Err(RouteError::InvalidConfig {
            key: "JESTERKY_PROXY_JSON_SCHEMA",
            message: "must be valid Unicode",
        }),
    }
}

fn required_env(key: &'static str) -> Result<String, RouteError> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(RouteError::MissingConfig(key)),
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
    fn only_gpt5_family_routes_are_native_chatgpt() {
        assert!(is_native_chatgpt_model("gpt-5"));
        assert!(is_native_chatgpt_model("gpt-5.4-mini"));
        assert!(is_native_chatgpt_model("gpt-5.5"));
        assert!(!is_native_chatgpt_model("gpt-oss-120b"));
        assert!(!is_native_chatgpt_model("gpt-custom-provider"));
        assert!(!is_native_chatgpt_model("gpt-5/custom"));
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
