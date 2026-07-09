//! Response translation: a completed chat/completions reply -> the exact
//! official Responses streaming (SSE) event sequence codex reconciles.
//!
//! codex tracks an "active item": an `output_text.delta` with no prior
//! `output_item.added` is dropped. So we open the message item + content part
//! before the delta and close both before `response.completed`.

use serde::Deserialize;
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// One framed Responses SSE event.
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event_type: String,
    pub data: Value,
}

impl SseEvent {
    /// Frame as `event: <type>\ndata: <json>\n\n`.
    pub fn frame(&self) -> String {
        format!(
            "event: {}\ndata: {}\n\n",
            self.event_type,
            serde_json::to_string(&self.data)
                .expect("serde_json::Value event data is serializable")
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SseError {
    #[error("provider chat response schema mismatch: {0}")]
    Schema(#[from] serde_json::Error),
    #[error("provider chat response contained no choices")]
    NoChoices,
    #[error("Responses SSE builder emitted no completed response")]
    MissingCompletedResponse,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: ChatUsage,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCall {
    id: String,
    function: ChatFunctionCall,
}

#[derive(Debug, Deserialize)]
struct ChatFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
}

impl ChatResponse {
    pub(crate) fn from_value(value: Value) -> Result<Self, SseError> {
        let parsed: Self = serde_json::from_value(value)?;
        if parsed.choices.is_empty() {
            return Err(SseError::NoChoices);
        }
        Ok(parsed)
    }

    fn primary_message(&self) -> &ChatMessage {
        &self.choices[0].message
    }

    fn usage_json(&self) -> Value {
        json!({
            "input_tokens": self.usage.prompt_tokens,
            "output_tokens": self.usage.completion_tokens,
            "total_tokens": self.usage.total_tokens,
        })
    }
}

/// Deterministic-ish 24 hex chars derived from a per-request counter + port +
/// nanotime. Unique-ish per response, not cryptographic (std has no rand and we
/// add no rand crate).
pub fn hex24(counter: u64, port: u16) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h1 = std::collections::hash_map::DefaultHasher::new();
    (counter, port, nanos).hash(&mut h1);
    let a = h1.finish();
    let mut h2 = std::collections::hash_map::DefaultHasher::new();
    (a, counter, port).hash(&mut h2);
    let b = h2.finish();
    let s = format!("{:016x}{:016x}", a, b);
    s[..24].to_string()
}

/// Build the FULL official Responses streaming sequence for a completed chat
/// reply: 9 events (8 when the assistant text is empty — no delta).
///
/// `rid` / `msg_id` are the (already-generated) response and message ids;
/// `model` is the codex-facing model id echoed back in the response object.
pub(crate) fn build_events_validated(
    chat_response: &ChatResponse,
    model: &str,
    rid: &str,
    msg_id: &str,
) -> Result<Vec<SseEvent>, SseError> {
    let message = chat_response.primary_message();
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let text = message.content.clone().unwrap_or_default();
    let usage = chat_response.usage_json();

    let resp_obj = |status: &str, output: Value| -> Value {
        json!({
            "id": rid,
            "object": "response",
            "created_at": created,
            "model": model,
            "status": status,
            "output": output,
            "usage": if status == "completed" { usage.clone() } else { Value::Null },
            "metadata": {},
        })
    };

    // The model's tool calls, if any (chat: choices[0].message.tool_calls). An
    // agentic codex turn expects these re-emitted as Responses `function_call`
    // items so it can run the tool and feed the result back.
    let tool_calls = &message.tool_calls;

    let mut seq: u64 = 0;
    let mut nxt = || {
        let cur = seq;
        seq += 1;
        cur
    };

    let mut events = Vec::new();
    let mut push = |event_type: &str, data: Value| {
        events.push(SseEvent {
            event_type: event_type.to_string(),
            data: merge_type(event_type, data),
        });
    };

    push(
        "response.created",
        json!({"sequence_number": nxt(), "response": resp_obj("in_progress", json!([]))}),
    );
    push(
        "response.in_progress",
        json!({"sequence_number": nxt(), "response": resp_obj("in_progress", json!([]))}),
    );

    // Assemble output items in order: a text message item (when there is text or
    // no tool calls), then one function_call item per tool call. `output_index`
    // advances per item; each item id must be stable across its added/delta/done.
    let mut output: Vec<Value> = Vec::new();
    let mut output_index: u64 = 0;

    let emit_message = tool_calls.is_empty() || !text.is_empty();
    if emit_message {
        let item_done = json!({
            "id": msg_id,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        });
        push(
            "response.output_item.added",
            json!({"sequence_number": nxt(), "output_index": output_index,
                "item": {"id": msg_id, "type": "message", "status": "in_progress", "role": "assistant", "content": []}}),
        );
        push(
            "response.content_part.added",
            json!({"sequence_number": nxt(), "item_id": msg_id, "output_index": output_index,
                "content_index": 0, "part": {"type": "output_text", "text": "", "annotations": []}}),
        );
        if !text.is_empty() {
            push(
                "response.output_text.delta",
                json!({"sequence_number": nxt(), "item_id": msg_id, "output_index": output_index,
                    "content_index": 0, "delta": text}),
            );
        }
        push(
            "response.output_text.done",
            json!({"sequence_number": nxt(), "item_id": msg_id, "output_index": output_index,
                "content_index": 0, "text": text}),
        );
        push(
            "response.content_part.done",
            json!({"sequence_number": nxt(), "item_id": msg_id, "output_index": output_index,
                "content_index": 0, "part": {"type": "output_text", "text": text, "annotations": []}}),
        );
        push(
            "response.output_item.done",
            json!({"sequence_number": nxt(), "output_index": output_index, "item": item_done.clone()}),
        );
        output.push(item_done);
        output_index += 1;
    }

    for (i, call) in tool_calls.iter().enumerate() {
        let name = call.function.name.as_str();
        let arguments = call.function.arguments.as_str();
        // call_id ties the function_call to the function_call_output codex sends
        // back; item id just keys this item's own added/delta/done stream.
        let call_id = call.id.as_str();
        let fc_id = format!("{msg_id}_fc{i}");
        let item_done = json!({
            "id": fc_id, "type": "function_call", "status": "completed",
            "call_id": call_id, "name": name, "arguments": arguments,
        });
        push(
            "response.output_item.added",
            json!({"sequence_number": nxt(), "output_index": output_index,
                "item": {"id": fc_id, "type": "function_call", "status": "in_progress",
                    "call_id": call_id, "name": name, "arguments": ""}}),
        );
        push(
            "response.function_call_arguments.delta",
            json!({"sequence_number": nxt(), "item_id": fc_id, "output_index": output_index,
                "delta": arguments}),
        );
        push(
            "response.function_call_arguments.done",
            json!({"sequence_number": nxt(), "item_id": fc_id, "output_index": output_index,
                "arguments": arguments}),
        );
        push(
            "response.output_item.done",
            json!({"sequence_number": nxt(), "output_index": output_index, "item": item_done.clone()}),
        );
        output.push(item_done);
        output_index += 1;
    }

    push(
        "response.completed",
        json!({"sequence_number": nxt(), "response": resp_obj("completed", Value::Array(output))}),
    );

    Ok(events)
}

/// The final `response` object (used for a non-streaming `stream: false` reply).
pub fn final_response_object(
    chat_response: &ChatResponse,
    model: &str,
    rid: &str,
    msg_id: &str,
) -> Result<Value, SseError> {
    let events = build_events_validated(chat_response, model, rid, msg_id)?;
    let response = events
        .last()
        .and_then(|e| e.data.get("response").cloned())
        .ok_or(SseError::MissingCompletedResponse)?;
    Ok(response)
}

/// Inject `"type": <event_type>` as the leading field of the data object.
fn merge_type(event_type: &str, data: Value) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("type".to_string(), json!(event_type));
    if let Value::Object(obj) = data {
        for (k, v) in obj {
            map.insert(k, v);
        }
    }
    Value::Object(map)
}

#[cfg(test)]
fn build_events(
    chat_response: &Value,
    model: &str,
    rid: &str,
    msg_id: &str,
) -> Result<Vec<SseEvent>, SseError> {
    let validated = ChatResponse::from_value(chat_response.clone())?;
    build_events_validated(&validated, model, rid, msg_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: [&str; 9] = [
        "response.created",
        "response.in_progress",
        "response.output_item.added",
        "response.content_part.added",
        "response.output_text.delta",
        "response.output_text.done",
        "response.content_part.done",
        "response.output_item.done",
        "response.completed",
    ];

    fn chat_reply(text: &str) -> Value {
        json!({
            "choices": [{"message": {"role": "assistant", "content": text}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8},
        })
    }

    #[test]
    fn emits_nine_events_in_order_with_contiguous_sequence() {
        let events =
            build_events(&chat_reply("hi there"), "deepseek/x", "resp_abc", "msg_def").unwrap();
        assert_eq!(events.len(), 9);
        for (i, ev) in events.iter().enumerate() {
            assert_eq!(ev.event_type, EXPECTED[i]);
            // data.type mirrors the event line.
            assert_eq!(ev.data["type"], json!(EXPECTED[i]));
            // sequence numbers are 0..N contiguous.
            assert_eq!(ev.data["sequence_number"], json!(i as u64));
        }

        // Each frame parses as event:/data: with a matching type.
        for (i, ev) in events.iter().enumerate() {
            let framed = ev.frame();
            let mut lines = framed.trim_end().split('\n');
            let event_line = lines.next().unwrap();
            let data_line = lines.next().unwrap();
            assert_eq!(event_line, format!("event: {}", EXPECTED[i]));
            let data_json: Value =
                serde_json::from_str(data_line.strip_prefix("data: ").unwrap()).unwrap();
            assert_eq!(data_json["type"], json!(EXPECTED[i]));
        }

        // Final response.completed carries item_done with the text.
        let completed = events.last().unwrap();
        assert_eq!(completed.event_type, "response.completed");
        let item = &completed.data["response"]["output"][0];
        assert_eq!(item["type"], json!("message"));
        assert_eq!(item["status"], json!("completed"));
        assert_eq!(item["content"][0]["text"], json!("hi there"));
        assert_eq!(
            completed.data["response"]["usage"],
            json!({"input_tokens": 3, "output_tokens": 5, "total_tokens": 8})
        );
    }

    #[test]
    fn empty_text_drops_the_delta_event() {
        let events = build_events(&chat_reply(""), "m", "resp_1", "msg_1").unwrap();
        assert_eq!(events.len(), 8);
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        assert!(!types.contains(&"response.output_text.delta"));
        // still contiguous 0..8
        for (i, ev) in events.iter().enumerate() {
            assert_eq!(ev.data["sequence_number"], json!(i as u64));
        }
    }

    #[test]
    fn tool_calls_emit_function_call_items() {
        // A chat reply with a tool call (no text) → function_call item events, no
        // message item.
        let reply = json!({
            "choices": [{"message": {"role": "assistant", "content": null,
                "tool_calls": [{"id": "call_9", "type": "function",
                    "function": {"name": "exec_command", "arguments": "{\"cmd\":\"ls\"}"}}]},
                "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3},
        });
        let events = build_events(&reply, "m", "resp_1", "msg_1").unwrap();
        let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
        // No message/text events; a full function_call item stream instead.
        assert!(!types.contains(&"response.output_text.delta"));
        assert!(types.contains(&"response.function_call_arguments.delta"));
        assert!(types.contains(&"response.function_call_arguments.done"));
        // sequence numbers contiguous.
        for (i, ev) in events.iter().enumerate() {
            assert_eq!(ev.data["sequence_number"], json!(i as u64));
        }
        // completed carries the function_call in output, with call_id + arguments.
        let out = &events.last().unwrap().data["response"]["output"][0];
        assert_eq!(out["type"], json!("function_call"));
        assert_eq!(out["call_id"], json!("call_9"));
        assert_eq!(out["name"], json!("exec_command"));
        assert_eq!(out["arguments"], json!("{\"cmd\":\"ls\"}"));
    }
}
