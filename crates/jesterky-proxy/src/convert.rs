//! Request translation: OpenAI Responses request -> OpenAI chat/completions payload.
//!
//! Ported exactly from the proven codex Responses↔chat bridge. Only the
//! single-turn, no-tool case is handled; a tool-call item is refused loudly
//! rather than mis-translated.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

/// A request that cannot be translated (e.g. a tool round-trip item). The server
/// surfaces this as a 400.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("invalid Responses request shape: {detail}")]
    InvalidRequest { detail: String },
    #[error("unable to serialize {target}: {detail}")]
    Serialization {
        target: &'static str,
        detail: String,
    },
}

#[derive(Debug, Clone)]
struct NonEmptyString(String);

impl NonEmptyString {
    fn into_inner(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for NonEmptyString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.trim().is_empty() {
            return Err(D::Error::custom("must be a non-empty string"));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResponsesRequest {
    model: String,
    instructions: Option<String>,
    input: Vec<ResponsesInputItem>,
    max_output_tokens: Option<i64>,
    max_tokens: Option<i64>,
    temperature: Option<f64>,
    stream: Option<bool>,
    tools: Vec<ResponsesTool>,
    tool_choice: Option<ToolChoice>,
    parallel_tool_calls: Option<bool>,
    text: Option<ResponsesText>,
}

impl ResponsesRequest {
    pub(crate) fn from_value(value: &Value) -> Result<Self, ConvertError> {
        let raw: RawResponsesRequest =
            serde_json::from_value(value.clone()).map_err(|err| ConvertError::InvalidRequest {
                detail: err.to_string(),
            })?;
        Ok(Self::from(raw))
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn stream_enabled(&self) -> bool {
        self.stream != Some(false)
    }
}

impl From<RawResponsesRequest> for ResponsesRequest {
    fn from(raw: RawResponsesRequest) -> Self {
        let input = raw
            .input
            .into_iter()
            .map(ResponsesInputItem::from_raw)
            .collect();
        let tools = raw
            .tools
            .unwrap_or_default()
            .into_iter()
            .map(ResponsesTool::from_raw)
            .collect();
        Self {
            model: raw.model.into_inner(),
            instructions: raw.instructions,
            input,
            max_output_tokens: raw.max_output_tokens,
            max_tokens: raw.max_tokens,
            temperature: raw.temperature,
            stream: raw.stream,
            tools,
            tool_choice: raw.tool_choice,
            parallel_tool_calls: raw.parallel_tool_calls,
            text: raw.text,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawResponsesRequest {
    model: NonEmptyString,
    instructions: Option<String>,
    input: Vec<RawResponsesInputItem>,
    max_output_tokens: Option<i64>,
    max_tokens: Option<i64>,
    temperature: Option<f64>,
    stream: Option<bool>,
    tools: Option<Vec<RawResponsesTool>>,
    tool_choice: Option<ToolChoice>,
    parallel_tool_calls: Option<bool>,
    text: Option<ResponsesText>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum ToolChoice {
    Mode(ToolChoiceMode),
    Function(Value),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ToolChoiceMode {
    Auto,
    None,
    Required,
}

#[derive(Debug, Clone)]
enum RawResponsesInputItem {
    Text(String),
    Typed(TypedResponsesInputItem),
    UntypedMessage(UntypedResponsesMessage),
}

impl<'de> Deserialize<'de> for RawResponsesInputItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::String(text) => Ok(Self::Text(text)),
            Value::Object(object) => {
                let value = Value::Object(object);
                if value.get("type").is_some() {
                    serde_json::from_value(value)
                        .map(Self::Typed)
                        .map_err(D::Error::custom)
                } else {
                    serde_json::from_value(value)
                        .map(Self::UntypedMessage)
                        .map_err(D::Error::custom)
                }
            }
            Value::Null => Err(D::Error::custom(
                "Responses input item must be a string or object, got null",
            )),
            Value::Bool(_) => Err(D::Error::custom(
                "Responses input item must be a string or object, got boolean",
            )),
            Value::Number(_) => Err(D::Error::custom(
                "Responses input item must be a string or object, got number",
            )),
            Value::Array(_) => Err(D::Error::custom(
                "Responses input item must be a string or object, got array",
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TypedResponsesInputItem {
    Message {
        role: ResponsesRole,
        content: ResponsesContent,
    },
    FunctionCall {
        #[serde(alias = "id")]
        call_id: NonEmptyString,
        name: NonEmptyString,
        arguments: NonEmptyString,
    },
    FunctionCallOutput {
        call_id: NonEmptyString,
        output: Value,
    },
    Reasoning,
    ReasoningSummary,
    ComputerCall,
    ComputerCallOutput,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UntypedResponsesMessage {
    role: ResponsesRole,
    content: ResponsesContent,
}

#[derive(Debug, Clone)]
enum ResponsesInputItem {
    Text(String),
    Message {
        role: ResponsesRole,
        content: ResponsesContent,
    },
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        call_id: String,
        output: Value,
    },
    ReasoningOrTrace,
}

impl ResponsesInputItem {
    fn from_raw(raw: RawResponsesInputItem) -> Self {
        match raw {
            RawResponsesInputItem::Text(text) => Self::Text(text),
            RawResponsesInputItem::UntypedMessage(message) => Self::Message {
                role: message.role,
                content: message.content,
            },
            RawResponsesInputItem::Typed(TypedResponsesInputItem::Message { role, content }) => {
                Self::Message { role, content }
            }
            RawResponsesInputItem::Typed(TypedResponsesInputItem::FunctionCall {
                call_id,
                name,
                arguments,
            }) => Self::FunctionCall {
                call_id: call_id.into_inner(),
                name: name.into_inner(),
                arguments: arguments.into_inner(),
            },
            RawResponsesInputItem::Typed(TypedResponsesInputItem::FunctionCallOutput {
                call_id,
                output,
            }) => Self::FunctionCallOutput {
                call_id: call_id.into_inner(),
                output,
            },
            RawResponsesInputItem::Typed(
                TypedResponsesInputItem::Reasoning
                | TypedResponsesInputItem::ReasoningSummary
                | TypedResponsesInputItem::ComputerCall
                | TypedResponsesInputItem::ComputerCallOutput,
            ) => Self::ReasoningOrTrace,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponsesRole {
    Developer,
    System,
    User,
    Assistant,
}

impl ResponsesRole {
    fn chat_role(self) -> ChatRole {
        match self {
            Self::Developer | Self::System => ChatRole::System,
            Self::User => ChatRole::User,
            Self::Assistant => ChatRole::Assistant,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesContentPart>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ResponsesContentPart {
    Text(String),
    Object(ResponsesTextContentPart),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponsesTextContentPart {
    InputText { text: String },
    OutputText { text: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RawResponsesTool {
    Function {
        name: NonEmptyString,
        description: Option<Value>,
        parameters: Value,
        strict: Option<Value>,
    },
    Custom {
        name: NonEmptyString,
        #[serde(rename = "format")]
        _format: Option<Value>,
    },
    Namespace {
        name: NonEmptyString,
        #[serde(rename = "description")]
        _description: Option<Value>,
        #[serde(rename = "tools")]
        _tools: Option<Vec<RawResponsesTool>>,
    },
    WebSearch,
    FileSearch,
    ImageGeneration,
    CodeInterpreter,
    Shell,
    ApplyPatch,
    Skill,
    ComputerUsePreview,
    Mcp,
    ToolSearch,
}

#[derive(Debug, Clone)]
enum ResponsesTool {
    Function {
        name: String,
        description: Option<Value>,
        parameters: Value,
        strict: Option<Value>,
    },
    Custom {
        name: String,
    },
    Namespace {
        name: String,
    },
    Builtin {
        kind: ResponsesBuiltinToolKind,
    },
}

#[derive(Debug, Clone, Copy)]
enum ResponsesBuiltinToolKind {
    WebSearch,
    FileSearch,
    ImageGeneration,
    CodeInterpreter,
    Shell,
    ApplyPatch,
    Skill,
    ComputerUsePreview,
    Mcp,
    ToolSearch,
}

impl ResponsesBuiltinToolKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::FileSearch => "file_search",
            Self::ImageGeneration => "image_generation",
            Self::CodeInterpreter => "code_interpreter",
            Self::Shell => "shell",
            Self::ApplyPatch => "apply_patch",
            Self::Skill => "skill",
            Self::ComputerUsePreview => "computer_use_preview",
            Self::Mcp => "mcp",
            Self::ToolSearch => "tool_search",
        }
    }
}

impl ResponsesTool {
    fn from_raw(raw: RawResponsesTool) -> Self {
        match raw {
            RawResponsesTool::Function {
                name,
                description,
                parameters,
                strict,
            } => Self::Function {
                name: name.into_inner(),
                description,
                parameters,
                strict,
            },
            RawResponsesTool::Custom { name, _format: _ } => Self::Custom {
                name: name.into_inner(),
            },
            RawResponsesTool::Namespace {
                name,
                _description: _,
                _tools: _,
            } => Self::Namespace {
                name: name.into_inner(),
            },
            RawResponsesTool::WebSearch => Self::builtin(ResponsesBuiltinToolKind::WebSearch),
            RawResponsesTool::FileSearch => Self::builtin(ResponsesBuiltinToolKind::FileSearch),
            RawResponsesTool::ImageGeneration => {
                Self::builtin(ResponsesBuiltinToolKind::ImageGeneration)
            }
            RawResponsesTool::CodeInterpreter => {
                Self::builtin(ResponsesBuiltinToolKind::CodeInterpreter)
            }
            RawResponsesTool::Shell => Self::builtin(ResponsesBuiltinToolKind::Shell),
            RawResponsesTool::ApplyPatch => Self::builtin(ResponsesBuiltinToolKind::ApplyPatch),
            RawResponsesTool::Skill => Self::builtin(ResponsesBuiltinToolKind::Skill),
            RawResponsesTool::ComputerUsePreview => {
                Self::builtin(ResponsesBuiltinToolKind::ComputerUsePreview)
            }
            RawResponsesTool::Mcp => Self::builtin(ResponsesBuiltinToolKind::Mcp),
            RawResponsesTool::ToolSearch => Self::builtin(ResponsesBuiltinToolKind::ToolSearch),
        }
    }

    fn builtin(kind: ResponsesBuiltinToolKind) -> Self {
        Self::Builtin { kind }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponsesTextFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: Value,
        strict: Option<bool>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct ResponsesText {
    format: Option<ResponsesTextFormat>,
}

#[derive(Debug, Clone, Serialize)]
struct ChatPayload {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<Value>,
}

impl ChatPayload {
    fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            messages: Vec::new(),
            stream: false,
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
        }
    }

    fn into_value(self) -> Result<Value, ConvertError> {
        serde_json::to_value(self).map_err(|err| ConvertError::Serialization {
            target: "chat payload",
            detail: err.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct ChatMessage {
    role: ChatRole,
    content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl ChatMessage {
    fn system(content: impl Into<String>) -> Self {
        Self::text(ChatRole::System, content)
    }

    fn text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Value::String(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant_tool_call(call_id: String, name: String, arguments: String) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: Value::Null,
            tool_calls: Some(vec![ChatToolCall {
                id: call_id,
                kind: "function".to_string(),
                function: ChatToolFunctionCall { name, arguments },
            }]),
            tool_call_id: None,
        }
    }

    fn tool_result(call_id: String, content: String) -> Self {
        Self {
            role: ChatRole::Tool,
            content: Value::String(content),
            tool_calls: None,
            tool_call_id: Some(call_id),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize)]
struct ChatToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ChatToolFunctionCall,
}

#[derive(Debug, Clone, Serialize)]
struct ChatToolFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Serialize)]
struct ChatTool {
    #[serde(rename = "type")]
    kind: String,
    function: ChatToolFunction,
}

#[derive(Debug, Clone, Serialize)]
struct ChatToolFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Value>,
    parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<Value>,
}

/// Flatten a Responses content value (str, or list of typed parts) to text.
fn text_from_content(content: &ResponsesContent) -> Result<String, ConvertError> {
    match content {
        ResponsesContent::Text(s) => Ok(s.clone()),
        ResponsesContent::Parts(parts) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    ResponsesContentPart::Text(s) => out.push_str(s),
                    ResponsesContentPart::Object(ResponsesTextContentPart::InputText { text })
                    | ResponsesContentPart::Object(ResponsesTextContentPart::OutputText { text }) => {
                        out.push_str(text)
                    }
                }
            }
            Ok(out)
        }
    }
}

/// Translate a codex Responses request body into an OpenAI chat/completions
/// payload. `upstream_model` is the model id sent upstream (may differ from the
/// codex-facing route id in the request body).
#[cfg(test)]
fn responses_request_to_chat(
    body: &Value,
    upstream_model: &str,
    supports_json_schema: bool,
) -> Result<Value, ConvertError> {
    let request = ResponsesRequest::from_value(body)?;
    responses_request_to_chat_payload(&request, upstream_model, supports_json_schema)
}

pub(crate) fn responses_request_to_chat_payload(
    request: &ResponsesRequest,
    upstream_model: &str,
    supports_json_schema: bool,
) -> Result<Value, ConvertError> {
    let mut chat = ChatPayload::new(upstream_model);

    if let Some(instructions) = &request.instructions {
        if !instructions.trim().is_empty() {
            chat.messages.push(ChatMessage::system(instructions));
        }
    }

    for item in &request.input {
        match item {
            ResponsesInputItem::Text(s) => {
                chat.messages.push(ChatMessage::text(ChatRole::User, s));
            }
            // A plain message (the common case + the system/user turns).
            ResponsesInputItem::Message { role, content } => {
                let text = text_from_content(content)?;
                chat.messages
                    .push(ChatMessage::text(role.chat_role(), text));
            }
            // The assistant's prior tool call → a chat assistant message carrying one
            // `tool_calls` entry. One message per call keeps the tool-message pairing
            // valid for every provider.
            ResponsesInputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => {
                chat.messages.push(ChatMessage::assistant_tool_call(
                    call_id.clone(),
                    name.clone(),
                    arguments.clone(),
                ));
            }
            // The tool's result codex feeds back → a chat `tool` message.
            ResponsesInputItem::FunctionCallOutput { call_id, output } => {
                let content = match output {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                chat.messages
                    .push(ChatMessage::tool_result(call_id.clone(), content));
            }
            // Reasoning traces and any other item kinds carry no chat equivalent.
            // Reasoning is explicitly skipped so an agentic transcript can round-trip
            // without pretending it was chat.
            ResponsesInputItem::ReasoningOrTrace => {}
        }
    }

    // max_output_tokens (or max_tokens) -> chat max_tokens, if a positive int.
    let max_out = request.max_output_tokens.or(request.max_tokens);
    if let Some(n) = max_out {
        if n > 0 {
            chat.max_tokens = Some(n);
        }
    }

    // temperature passthrough, if numeric.
    if let Some(t) = &request.temperature {
        chat.temperature = Some(*t);
    }

    // Tools: Responses `{type:function, name, description, parameters, strict}` ->
    // chat `{type:function, function:{...}}`. This is what makes an AGENTIC codex
    // loop work through the proxy. `strict` is kept only for providers that accept
    // strict schemas (else it can be rejected). Non-function tool types (built-in
    // Responses tools) have no chat equivalent and are rejected at ingestion.
    let translated_tools = chat_tools(&request.tools, supports_json_schema);
    if !translated_tools.omitted_custom_names.is_empty() {
        chat.messages.push(ChatMessage::system(format!(
            "These Responses custom tools are unavailable through the chat/completions provider route: {}.",
            translated_tools.omitted_custom_names.join(", ")
        )));
    }
    let has_tools = !translated_tools.tools.is_empty();
    if has_tools {
        chat.tools = Some(translated_tools.tools);
        if let Some(tool_choice) = &request.tool_choice {
            chat.tool_choice = Some(tool_choice.clone());
        }
        if let Some(parallel_tool_calls) = &request.parallel_tool_calls {
            chat.parallel_tool_calls = Some(parallel_tool_calls.clone());
        }
    }

    // Structured output: Responses `text.format` json_schema -> chat response_format.
    // Providers that accept strict json_schema get it verbatim; those that only
    // accept json_object (DeepSeek) get downgraded — and the schema is injected as
    // a system message so the model still produces the right shape.
    //
    // SKIP entirely when tools are present: an agentic turn must be free to emit a
    // tool call, and forcing `json_object` (esp. the DeepSeek downgrade) would coerce
    // prose-JSON instead. The final answer is validated host-side by `ModelActor`.
    if has_tools {
        return chat.into_value();
    }
    if let Some(format) = request.text.as_ref().and_then(|text| text.format.as_ref()) {
        match format {
            ResponsesTextFormat::Text => {}
            ResponsesTextFormat::JsonObject => {
                chat.response_format = Some(json!({"type": "json_object"}));
            }
            ResponsesTextFormat::JsonSchema {
                name,
                schema,
                strict,
            } => {
                if supports_json_schema {
                    let mut response_format = json!({
                        "type": "json_schema",
                        "json_schema": {
                            "name": name,
                            "schema": schema,
                        }
                    });
                    if let Some(strict) = strict {
                        response_format["json_schema"]["strict"] = json!(strict);
                    }
                    chat.response_format = Some(response_format);
                } else {
                    chat.response_format = Some(json!({"type": "json_object"}));
                    // Inject the schema so the model knows the exact shape json_object
                    // alone does not constrain. Prepend so later turns can't bury it.
                    let schema_text = serde_json::to_string(schema).map_err(|err| {
                        ConvertError::Serialization {
                            target: "response schema",
                            detail: err.to_string(),
                        }
                    })?;
                    chat.messages.insert(
                    0,
                    ChatMessage::system(format!(
                        "You MUST reply with a single JSON object conforming exactly to this JSON Schema (no prose, no markdown):\n{schema_text}",
                    )),
                );
                }
            }
        }
    }

    chat.into_value()
}

struct ToolTranslation {
    tools: Vec<ChatTool>,
    omitted_custom_names: Vec<String>,
}

fn chat_tools(tools: &[ResponsesTool], supports_json_schema: bool) -> ToolTranslation {
    let mut out = Vec::new();
    let mut omitted_custom_names = Vec::new();
    for tool in tools {
        match tool {
            ResponsesTool::Function {
                name,
                description,
                parameters,
                strict,
            } => out.push(ChatTool {
                kind: "function".to_string(),
                function: ChatToolFunction {
                    name: name.clone(),
                    description: description.clone(),
                    parameters: parameters.clone(),
                    strict: supports_json_schema.then(|| strict.clone()).flatten(),
                },
            }),
            ResponsesTool::Custom { name } => omitted_custom_names.push(name.clone()),
            ResponsesTool::Namespace { name } => {
                omitted_custom_names.push(format!("namespace:{name}"));
            }
            ResponsesTool::Builtin { kind } => {
                omitted_custom_names.push(kind.as_str().to_string());
            }
        }
    }
    ToolTranslation {
        tools: out,
        omitted_custom_names,
    }
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
        assert_eq!(
            msgs[2]["tool_calls"][0]["function"]["name"],
            json!("exec_command")
        );
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
        assert!(
            msgs[0]["content"]
                .as_str()
                .unwrap()
                .contains("\"required\"")
        );
    }
}
