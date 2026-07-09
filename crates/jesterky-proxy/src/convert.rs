//! Request translation: OpenAI Responses request -> OpenAI chat/completions payload.
//!
//! Ported exactly from the proven codex Responses↔chat bridge. Only the
//! single-turn, no-tool case is handled; a tool-call item is refused loudly
//! rather than mis-translated.

use serde_json::{json, Map, Value};

/// A request that cannot be translated (e.g. a tool round-trip item). The server
/// surfaces this as a 400.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ConvertError(pub String);

/// Flatten a Responses content value (str, or list of typed parts) to text.
fn text_from_content(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    Value::String(s) => out.push_str(s),
                    Value::Object(map) => {
                        if let Some(Value::String(t)) = map.get("text") {
                            out.push_str(t);
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Map a Responses role to a chat role. codex emits `developer` for the system
/// prompt (Responses convention) — map it to `system`.
fn role_to_chat(role: &str) -> &'static str {
    match role {
        "developer" | "system" => "system",
        "assistant" => "assistant",
        _ => "user",
    }
}

/// Translate a codex Responses request body into an OpenAI chat/completions
/// payload. `upstream_model` is the model id sent upstream (may differ from the
/// codex-facing route id in the request body).
pub fn responses_request_to_chat(
    body: &Value,
    upstream_model: &str,
    supports_json_schema: bool,
) -> Result<Value, ConvertError> {
    let mut messages: Vec<Value> = Vec::new();

    if let Some(Value::String(instructions)) = body.get("instructions") {
        if !instructions.trim().is_empty() {
            messages.push(json!({"role": "system", "content": instructions}));
        }
    }

    if let Some(Value::Array(items)) = body.get("input") {
        for item in items {
            match item {
                Value::String(s) => {
                    messages.push(json!({"role": "user", "content": s}));
                }
                Value::Object(map) => {
                    let itype = map.get("type").and_then(Value::as_str);
                    match itype {
                        // A plain message (the common case + the system/user turns).
                        None | Some("message") => {
                            let text =
                                text_from_content(map.get("content").unwrap_or(&Value::Null));
                            let role =
                                map.get("role").and_then(Value::as_str).unwrap_or("user");
                            messages.push(json!({"role": role_to_chat(role), "content": text}));
                        }
                        // The assistant's prior tool call → a chat assistant message
                        // carrying one `tool_calls` entry (arguments is already a JSON
                        // string). One message per call keeps the tool-message pairing
                        // valid for every provider.
                        Some("function_call") => {
                            let call_id = map
                                .get("call_id")
                                .or_else(|| map.get("id"))
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let name = map.get("name").and_then(Value::as_str).unwrap_or_default();
                            let arguments = map
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or("{}");
                            messages.push(json!({
                                "role": "assistant",
                                "content": Value::Null,
                                "tool_calls": [{
                                    "id": call_id,
                                    "type": "function",
                                    "function": {"name": name, "arguments": arguments},
                                }],
                            }));
                        }
                        // The tool's result codex feeds back → a chat `tool` message.
                        Some("function_call_output") => {
                            let call_id = map
                                .get("call_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let content = match map.get("output") {
                                Some(Value::String(s)) => s.clone(),
                                Some(other) => other.to_string(),
                                None => String::new(),
                            };
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": call_id,
                                "content": content,
                            }));
                        }
                        // Reasoning traces and any other item kinds carry no chat
                        // equivalent — skip rather than refuse (the loop must survive
                        // a multi-turn agentic transcript).
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let mut chat = Map::new();
    chat.insert("model".to_string(), json!(upstream_model));
    chat.insert("messages".to_string(), Value::Array(messages));
    chat.insert("stream".to_string(), json!(false));

    // max_output_tokens (or max_tokens) -> chat max_tokens, if a positive int.
    let max_out = body
        .get("max_output_tokens")
        .and_then(Value::as_i64)
        .or_else(|| body.get("max_tokens").and_then(Value::as_i64));
    if let Some(n) = max_out {
        if n > 0 {
            chat.insert("max_tokens".to_string(), json!(n));
        }
    }

    // temperature passthrough, if numeric.
    if let Some(t) = body.get("temperature") {
        if t.is_number() {
            chat.insert("temperature".to_string(), t.clone());
        }
    }

    // Tools: Responses `{type:function, name, description, parameters, strict}` ->
    // chat `{type:function, function:{...}}`. This is what makes an AGENTIC codex
    // loop work through the proxy. `strict` is kept only for providers that accept
    // strict schemas (else it can be rejected). Non-function tool types (built-in
    // Responses tools) have no chat equivalent and are dropped.
    let has_tools = if let Some(Value::Array(tools)) = body.get("tools") {
        let chat_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let o = t.as_object()?;
                if o.get("type").and_then(Value::as_str) != Some("function") {
                    return None;
                }
                let mut f = Map::new();
                f.insert("name".to_string(), o.get("name")?.clone());
                if let Some(d) = o.get("description") {
                    f.insert("description".to_string(), d.clone());
                }
                f.insert(
                    "parameters".to_string(),
                    o.get("parameters").cloned().unwrap_or_else(|| json!({})),
                );
                if supports_json_schema {
                    if let Some(s) = o.get("strict") {
                        f.insert("strict".to_string(), s.clone());
                    }
                }
                Some(json!({"type": "function", "function": Value::Object(f)}))
            })
            .collect();
        let present = !chat_tools.is_empty();
        if present {
            chat.insert("tools".to_string(), Value::Array(chat_tools));
            if let Some(tc) = body.get("tool_choice") {
                chat.insert("tool_choice".to_string(), tc.clone());
            }
            if let Some(p) = body.get("parallel_tool_calls") {
                chat.insert("parallel_tool_calls".to_string(), p.clone());
            }
        }
        present
    } else {
        false
    };

    // Structured output: Responses `text.format` json_schema -> chat response_format.
    // Providers that accept strict json_schema get it verbatim; those that only
    // accept json_object (DeepSeek) get downgraded — and the schema is injected as
    // a system message so the model still produces the right shape.
    //
    // SKIP entirely when tools are present: an agentic turn must be free to emit a
    // tool call, and forcing `json_object` (esp. the DeepSeek downgrade) would coerce
    // prose-JSON instead. The final answer is validated host-side by `ModelActor`.
    if has_tools {
        return Ok(Value::Object(chat));
    }
    if let Some(Value::Object(text_obj)) = body.get("text") {
        if let Some(Value::Object(fmt)) = text_obj.get("format") {
            if fmt.get("type").and_then(Value::as_str) == Some("json_schema") {
                let schema = fmt.get("schema").cloned().unwrap_or_else(|| json!({}));
                if supports_json_schema {
                    let mut js = Map::new();
                    js.insert(
                        "name".to_string(),
                        fmt.get("name").cloned().unwrap_or_else(|| json!("output")),
                    );
                    js.insert("schema".to_string(), schema);
                    if let Some(strict) = fmt.get("strict") {
                        js.insert("strict".to_string(), strict.clone());
                    }
                    chat.insert(
                        "response_format".to_string(),
                        json!({"type": "json_schema", "json_schema": Value::Object(js)}),
                    );
                } else {
                    chat.insert(
                        "response_format".to_string(),
                        json!({"type": "json_object"}),
                    );
                    // Inject the schema so the model knows the exact shape json_object
                    // alone does not constrain. Prepend so later turns can't bury it.
                    if let Some(Value::Array(msgs)) = chat.get_mut("messages") {
                        msgs.insert(
                            0,
                            json!({
                                "role": "system",
                                "content": format!(
                                    "You MUST reply with a single JSON object conforming exactly to this JSON Schema (no prose, no markdown):\n{}",
                                    serde_json::to_string(&schema).unwrap_or_default()
                                ),
                            }),
                        );
                    }
                }
            }
        }
    }

    Ok(Value::Object(chat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_instructions_input_max_tokens_and_json_schema() {
        let body = json!({
            "model": "deepseek/deepseek-v4-pro-direct",
            "instructions": "You are helpful.",
            "input": [
                {"type": "message", "role": "user", "content": "hello"}
            ],
            "max_output_tokens": 512,
            "temperature": 0.5,
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "grade",
                    "schema": {"type": "object"},
                    "strict": true
                }
            }
        });

        let chat = responses_request_to_chat(&body, "deepseek-chat", true).expect("ok");

        assert_eq!(chat["model"], json!("deepseek-chat"));
        assert_eq!(chat["stream"], json!(false));
        assert_eq!(chat["max_tokens"], json!(512));
        assert_eq!(chat["temperature"], json!(0.5));

        let msgs = chat["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(
            msgs[0],
            json!({"role": "system", "content": "You are helpful."})
        );
        assert_eq!(msgs[1], json!({"role": "user", "content": "hello"}));

        let rf = &chat["response_format"];
        assert_eq!(rf["type"], json!("json_schema"));
        assert_eq!(rf["json_schema"]["name"], json!("grade"));
        assert_eq!(rf["json_schema"]["schema"], json!({"type": "object"}));
        assert_eq!(rf["json_schema"]["strict"], json!(true));
    }

    #[test]
    fn developer_role_becomes_system_and_string_item_is_user() {
        let body = json!({
            "input": [
                "plain string item",
                {"role": "developer", "content": [{"type": "input_text", "text": "sys"}]}
            ]
        });
        let chat = responses_request_to_chat(&body, "m", true).expect("ok");
        let msgs = chat["messages"].as_array().unwrap();
        assert_eq!(
            msgs[0],
            json!({"role": "user", "content": "plain string item"})
        );
        assert_eq!(msgs[1], json!({"role": "system", "content": "sys"}));
    }

    #[test]
    fn agentic_round_trip_translates_tools_and_tool_messages() {
        // A codex agentic turn: function tools + a prior tool call and its result.
        let body = json!({
            "instructions": "be an agent",
            "tools": [{"type": "function", "name": "exec_command",
                "description": "run a command", "parameters": {"type": "object"}, "strict": true}],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "input": [
                {"type": "message", "role": "user", "content": "build it"},
                {"type": "function_call", "call_id": "call_1", "name": "exec_command",
                    "arguments": "{\"cmd\":\"cargo build\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "ok, built"},
                {"type": "reasoning", "summary": "thinking"}
            ]
        });
        let chat = responses_request_to_chat(&body, "deepseek-chat", false).expect("ok");

        // Tools translated to chat function format; tool_choice passed through.
        let tools = chat["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], json!("function"));
        assert_eq!(tools[0]["function"]["name"], json!("exec_command"));
        assert_eq!(chat["tool_choice"], json!("auto"));
        // With tools present, no json_object coercion is applied.
        assert!(chat.get("response_format").is_none());

        // Messages: system(instructions), user, assistant(tool_calls), tool. The
        // reasoning item is dropped.
        let msgs = chat["messages"].as_array().unwrap();
        let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["system", "user", "assistant", "tool"]);
        assert_eq!(msgs[2]["tool_calls"][0]["id"], json!("call_1"));
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], json!("exec_command"));
        assert_eq!(msgs[3]["tool_call_id"], json!("call_1"));
        assert_eq!(msgs[3]["content"], json!("ok, built"));
    }

    #[test]
    fn json_schema_downgrades_to_json_object_and_injects_schema() {
        let body = json!({
            "input": [{"role": "user", "content": "grade it"}],
            "text": {"format": {"type": "json_schema", "name": "v",
                "schema": {"type": "object", "required": ["ok"]}, "strict": true}}
        });
        // supports_json_schema = false → json_object + schema injected as a system msg.
        let chat = responses_request_to_chat(&body, "deepseek-chat", false).expect("ok");
        assert_eq!(chat["response_format"], json!({"type": "json_object"}));
        let msgs = chat["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], json!("system"));
        assert!(msgs[0]["content"].as_str().unwrap().contains("JSON Schema"));
        assert!(msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("\"required\""));
    }
}
