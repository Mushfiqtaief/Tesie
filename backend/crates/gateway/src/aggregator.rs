use std::collections::BTreeMap;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde_json::{json, Map, Value};
use tesie_core::{Result, TesRouteError, WireFormat};
use tesie_providers::translator::sse::SseBuffer;

pub async fn drain_and_aggregate<S>(
    source_format: WireFormat,
    fallback_model: &str,
    mut stream: S,
) -> Result<Value>
where
    S: Stream<Item = Result<Bytes>> + Unpin,
{
    let mut sse = SseBuffer::default();
    let mut events = Vec::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        events.extend(sse.push(&bytes));
    }
    events.extend(sse.finish());

    let payloads: Vec<(Option<String>, Value)> = events
        .into_iter()
        .filter(|ev| ev.data.trim() != "[DONE]" && !ev.data.trim().is_empty())
        .filter_map(|ev| {
            serde_json::from_str::<Value>(ev.data.trim())
                .ok()
                .map(|val| (ev.event, val))
        })
        .collect();

    Ok(match source_format {
        WireFormat::OpenAiChat => aggregate_openai_chat(&payloads, fallback_model),
        WireFormat::OpenAiResponses => aggregate_openai_responses(&payloads, fallback_model),
        WireFormat::AnthropicMessages => aggregate_anthropic_messages(&payloads, fallback_model),
    })
}

#[derive(Default)]
struct ChatToolCallAgg {
    id: String,
    kind: String,
    name: String,
    arguments: String,
}

fn aggregate_openai_chat(payloads: &[(Option<String>, Value)], fallback_model: &str) -> Value {
    let mut id = format!("chatcmpl-{}", chrono_sec());
    let mut created = chrono_sec();
    let mut model = fallback_model.to_string();
    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut tool_calls: BTreeMap<usize, ChatToolCallAgg> = BTreeMap::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<Value> = None;

    for (_, chunk) in payloads {
        if let Some(cid) = chunk.get("id").and_then(Value::as_str)
            && !cid.is_empty()
        {
            id = cid.to_string();
        }
        if let Some(cr) = chunk.get("created").and_then(Value::as_i64) {
            created = cr;
        }
        if let Some(m) = chunk.get("model").and_then(Value::as_str)
            && !m.is_empty()
        {
            model = m.to_string();
        }
        if let Some(u) = chunk.get("usage")
            && !u.is_null()
        {
            usage = Some(u.clone());
        }

        if let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        {
            if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                finish_reason = Some(fr.to_string());
            }
            if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
                if let Some(c) = delta.get("content").and_then(Value::as_str) {
                    content.push_str(c);
                }
                if let Some(rc) = delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
                    .and_then(Value::as_str)
                {
                    reasoning_content.push_str(rc);
                }
                if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tc in calls {
                        let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let entry = tool_calls.entry(idx).or_insert_with(|| ChatToolCallAgg {
                            kind: "function".to_string(),
                            ..Default::default()
                        });
                        if let Some(tid) = tc.get("id").and_then(Value::as_str)
                            && !tid.is_empty()
                        {
                            entry.id = tid.to_string();
                        }
                        if let Some(tp) = tc.get("type").and_then(Value::as_str)
                            && !tp.is_empty()
                        {
                            entry.kind = tp.to_string();
                        }
                        if let Some(fn_obj) = tc.get("function").and_then(Value::as_object) {
                            if let Some(name) = fn_obj.get("name").and_then(Value::as_str)
                                && !name.is_empty()
                            {
                                entry.name.push_str(name);
                            }
                            if let Some(args) = fn_obj.get("arguments").and_then(Value::as_str) {
                                entry.arguments.push_str(args);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut message = Map::new();
    message.insert("role".into(), Value::String("assistant".into()));
    if content.is_empty() && !tool_calls.is_empty() {
        message.insert("content".into(), Value::Null);
    } else {
        message.insert("content".into(), Value::String(content));
    }
    if !reasoning_content.is_empty() {
        message.insert("reasoning_content".into(), Value::String(reasoning_content));
    }
    if !tool_calls.is_empty() {
        let calls: Vec<Value> = tool_calls
            .into_values()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": tc.kind,
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments
                    }
                })
            })
            .collect();
        message.insert("tool_calls".into(), Value::Array(calls));
        if finish_reason.is_none() {
            finish_reason = Some("tool_calls".to_string());
        }
    }

    let mut out = json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": Value::Object(message),
            "finish_reason": finish_reason.unwrap_or_else(|| "stop".to_string())
        }]
    });
    if let Some(u) = usage {
        out["usage"] = u;
    }
    out
}

fn aggregate_openai_responses(payloads: &[(Option<String>, Value)], fallback_model: &str) -> Value {
    for (ev_name, val) in payloads.iter().rev() {
        let ev = val
            .get("type")
            .and_then(Value::as_str)
            .or(ev_name.as_deref())
            .unwrap_or("");
        if (ev == "response.completed" || ev == "response.done")
            && let Some(resp) = val.get("response")
            && resp.is_object()
        {
            return resp.clone();
        }
    }

    let mut text = String::new();
    let mut output_items: Vec<Value> = Vec::new();
    let mut usage: Option<Value> = None;

    for (ev_name, val) in payloads {
        let ev = val
            .get("type")
            .and_then(Value::as_str)
            .or(ev_name.as_deref())
            .unwrap_or("");
        match ev {
            "response.output_text.delta" => {
                if let Some(d) = val.get("delta").and_then(Value::as_str) {
                    text.push_str(d);
                }
            }
            "response.output_item.done" => {
                if let Some(item) = val.get("item") {
                    output_items.push(item.clone());
                }
            }
            "response.completed" | "response.done" => {
                if let Some(u) = val.get("response").and_then(|r| r.get("usage")) {
                    usage = Some(u.clone());
                }
            }
            _ => {}
        }
    }

    if output_items.is_empty() && !text.is_empty() {
        output_items.push(json!({
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }]
        }));
    }

    let mut out = json!({
        "id": format!("resp_{}", chrono_sec()),
        "object": "response",
        "created_at": chrono_sec(),
        "model": fallback_model,
        "status": "completed",
        "output": output_items
    });
    if let Some(u) = usage {
        out["usage"] = u;
    }
    out
}

#[derive(Default)]
struct ClaudeBlockAgg {
    block_type: String,
    text: String,
    thinking: String,
    tool_id: String,
    tool_name: String,
    input_json: String,
    initial_input: Option<Value>,
}

fn aggregate_anthropic_messages(payloads: &[(Option<String>, Value)], fallback_model: &str) -> Value {
    let mut msg_id = format!("msg_{}", chrono_sec());
    let mut model = fallback_model.to_string();
    let mut blocks: BTreeMap<usize, ClaudeBlockAgg> = BTreeMap::new();
    let mut stop_reason: Option<String> = None;
    let mut input_tokens = 0_i64;
    let mut output_tokens = 0_i64;

    for (ev_name, val) in payloads {
        let ev = val
            .get("type")
            .and_then(Value::as_str)
            .or(ev_name.as_deref())
            .unwrap_or("");
        match ev {
            "message_start" => {
                if let Some(m) = val.get("message") {
                    if let Some(id) = m.get("id").and_then(Value::as_str) {
                        msg_id = id.to_string();
                    }
                    if let Some(md) = m.get("model").and_then(Value::as_str) {
                        model = md.to_string();
                    }
                    if let Some(u) = m.get("usage") {
                        input_tokens = u
                            .get("input_tokens")
                            .and_then(Value::as_i64)
                            .unwrap_or(input_tokens);
                    }
                }
            }
            "content_block_start" => {
                let idx = val.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if let Some(cb) = val.get("content_block").and_then(Value::as_object) {
                    let btype = cb
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("text")
                        .to_string();
                    let agg = ClaudeBlockAgg {
                        block_type: btype,
                        text: cb
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        thinking: cb
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        tool_id: cb
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        tool_name: cb
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        input_json: String::new(),
                        initial_input: cb.get("input").cloned(),
                    };
                    blocks.insert(idx, agg);
                }
            }
            "content_block_delta" => {
                let idx = val.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let entry = blocks.entry(idx).or_insert_with(|| ClaudeBlockAgg {
                    block_type: "text".to_string(),
                    ..Default::default()
                });
                if let Some(d) = val.get("delta").and_then(Value::as_object) {
                    match d.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            if let Some(t) = d.get("text").and_then(Value::as_str) {
                                entry.text.push_str(t);
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(t) = d.get("thinking").and_then(Value::as_str) {
                                entry.thinking.push_str(t);
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(p) = d.get("partial_json").and_then(Value::as_str) {
                                entry.input_json.push_str(p);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                if let Some(sr) = val
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    stop_reason = Some(sr.to_string());
                }
                if let Some(u) = val.get("usage") {
                    if let Some(it) = u.get("input_tokens").and_then(Value::as_i64) {
                        input_tokens = it;
                    }
                    if let Some(ot) = u.get("output_tokens").and_then(Value::as_i64) {
                        output_tokens = ot;
                    }
                }
            }
            _ => {}
        }
    }

    let content: Vec<Value> = blocks
        .into_values()
        .map(|b| match b.block_type.as_str() {
            "thinking" => json!({ "type": "thinking", "thinking": b.thinking }),
            "tool_use" => {
                let parsed_input = if !b.input_json.trim().is_empty() {
                    serde_json::from_str::<Value>(&b.input_json).unwrap_or_else(|_| json!({}))
                } else {
                    b.initial_input.unwrap_or_else(|| json!({}))
                };
                json!({
                    "type": "tool_use",
                    "id": b.tool_id,
                    "name": b.tool_name,
                    "input": parsed_input
                })
            }
            _ => json!({ "type": "text", "text": b.text }),
        })
        .collect();

    json!({
        "id": msg_id,
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": content,
        "stop_reason": stop_reason.unwrap_or_else(|| "end_turn".to_string()),
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    })
}

fn chrono_sec() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn sse_error_payload(err: &TesRouteError) -> (axum::http::StatusCode, Option<u64>, Value) {
    use axum::http::StatusCode;
    match err {
        TesRouteError::BadRequest(msg) => (
            StatusCode::BAD_REQUEST,
            None,
            json!({ "error": { "message": msg, "type": "invalid_request_error" } }),
        ),
        TesRouteError::ContextLengthExceeded(msg) => {
            let clean_msg = if msg.trim().is_empty() {
                "This model's maximum context length was exceeded. Please compact or shorten the prompt."
            } else {
                msg.as_str()
            };
            (
                StatusCode::BAD_REQUEST,
                None,
                json!({
                    "error": {
                        "message": clean_msg,
                        "type": "invalid_request_error",
                        "param": "messages",
                        "code": "context_length_exceeded"
                    }
                }),
            )
        }
        TesRouteError::Unauthorized(msg) => (
            StatusCode::UNAUTHORIZED,
            None,
            json!({ "error": { "message": msg, "type": "authentication_error" } }),
        ),
        TesRouteError::NotFound(msg) => (
            StatusCode::NOT_FOUND,
            None,
            json!({ "error": { "message": msg, "type": "not_found_error" } }),
        ),
        TesRouteError::Conflict(msg) => (
            StatusCode::CONFLICT,
            None,
            json!({ "error": { "message": msg, "type": "conflict_error" } }),
        ),
        TesRouteError::AllIpsRateLimited { .. } => (
            StatusCode::FORBIDDEN,
            None,
            json!({
                "error": {
                    "message": "No quota available for this model.",
                    "type": "insufficient_quota",
                    "code": "insufficient_quota"
                }
            }),
        ),
        TesRouteError::ModelQuotaExhausted(msg) => (
            StatusCode::FORBIDDEN,
            None,
            json!({
                "error": {
                    "message": msg,
                    "type": "insufficient_quota",
                    "code": "insufficient_quota"
                }
            }),
        ),
        TesRouteError::ModelCoolingDown { message, .. } => (
            StatusCode::FORBIDDEN,
            None,
            json!({
                "error": {
                    "message": message,
                    "type": "model_cooldown",
                    "code": "model_cooldown"
                }
            }),
        ),
        TesRouteError::ApiKeyQuotaExceeded(msg) => (
            StatusCode::TOO_MANY_REQUESTS,
            None,
            json!({ "error": { "message": msg, "type": "insufficient_quota" } }),
        ),
        TesRouteError::NoCredentials(msg) => (
            StatusCode::SERVICE_UNAVAILABLE,
            None,
            json!({ "error": { "message": format!("no credentials configured for {msg}"), "type": "service_unavailable" } }),
        ),
        TesRouteError::Upstream { status, message } => {
            if tesie_core::is_context_length_error(*status, message) {
                let clean_msg = tesie_core::extract_clean_context_message(message);
                return (
                    StatusCode::BAD_REQUEST,
                    None,
                    json!({
                        "error": {
                            "message": clean_msg,
                            "type": "invalid_request_error",
                            "param": "messages",
                            "code": "context_length_exceeded"
                        }
                    }),
                );
            }
            let lower = message.to_ascii_lowercase();
            let is_quota_leak = lower.contains("freeusagelimiterror")
                || lower.contains("freetiererror")
                || lower.contains("free usage")
                || lower.contains("usage exceeded")
                || lower.contains("usage limit")
                || lower.contains("subscribe to go")
                || lower.contains("subscribe to opencode")
                || lower.contains("opencode.ai/go")
                || lower.contains("insufficient_quota")
                || lower.contains("resource_exhausted")
                || lower.contains("quota_exhausted")
                || lower.contains("quota exceeded")
                || lower.contains("rate limit")
                || lower.contains("rate_limit")
                || lower.contains("too many requests");
            if is_quota_leak {
                (
                    StatusCode::FORBIDDEN,
                    None,
                    json!({
                        "error": {
                            "message": "No quota available for this model.",
                            "type": "insufficient_quota",
                            "code": "insufficient_quota"
                        }
                    }),
                )
            } else {
                let code = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
                let body = serde_json::from_str::<Value>(message)
                    .unwrap_or_else(|_| json!({ "error": { "message": message, "type": "upstream_error" } }));
                (code, None, body)
            }
        }
        TesRouteError::ProxyRelay(msg) => {
            let lower = msg.to_ascii_lowercase();
            let is_quota_leak = lower.contains("freeusagelimiterror")
                || lower.contains("freetiererror")
                || lower.contains("free usage")
                || lower.contains("usage exceeded")
                || lower.contains("usage limit")
                || lower.contains("subscribe to go")
                || lower.contains("subscribe to opencode")
                || lower.contains("opencode.ai/go")
                || lower.contains("insufficient_quota")
                || lower.contains("resource_exhausted")
                || lower.contains("quota_exhausted")
                || lower.contains("quota exceeded");
            if is_quota_leak {
                (
                    StatusCode::FORBIDDEN,
                    None,
                    json!({
                        "error": {
                            "message": "No quota available for this model.",
                            "type": "insufficient_quota",
                            "code": "insufficient_quota"
                        }
                    }),
                )
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    None,
                    json!({ "error": { "message": msg, "type": "proxy_relay_error" } }),
                )
            }
        }
        TesRouteError::Store(msg) | TesRouteError::Internal(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            None,
            json!({ "error": { "message": msg, "type": "server_error" } }),
        ),
    }
}
