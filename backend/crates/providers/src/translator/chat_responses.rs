use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Map, Value};

pub fn chat_request_to_responses(model: &str, body: &Value) -> Value {
    let Some(obj) = body.as_object() else {
        return body.clone();
    };
    if obj.contains_key("input") {
        let mut out = obj.clone();
        out.insert("model".into(), Value::String(model.to_string()));
        out.insert("stream".into(), Value::Bool(true));
        return Value::Object(out);
    }

    let mut result = Map::new();
    result.insert("model".into(), Value::String(model.to_string()));
    result.insert("stream".into(), Value::Bool(true));
    result.insert("store".into(), Value::Bool(false));

    let mut instructions_parts: Vec<String> = Vec::new();
    let mut input_items: Vec<Value> = Vec::new();

    if let Some(messages) = obj.get("messages").and_then(Value::as_array) {
        for msg in messages {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
            match role {
                "system" | "developer" => {
                    let txt = extract_text_content(msg.get("content"));
                    if !txt.is_empty() {
                        instructions_parts.push(txt);
                    }
                }
                "user" | "assistant" => {
                    let content_type = if role == "user" {
                        "input_text"
                    } else {
                        "output_text"
                    };
                    let mut parts = Vec::new();
                    if let Some(s) = msg.get("content").and_then(Value::as_str) {
                        if !s.is_empty() {
                            parts.push(json!({ "type": content_type, "text": s }));
                        }
                    } else if let Some(arr) = msg.get("content").and_then(Value::as_array) {
                        for item in arr {
                            match item.get("type").and_then(Value::as_str) {
                                Some("text") | Some("input_text") | Some("output_text") => {
                                    if let Some(t) = item.get("text").and_then(Value::as_str) {
                                        parts.push(json!({ "type": content_type, "text": t }));
                                    }
                                }
                                Some("image_url") => {
                                    let url = item
                                        .get("image_url")
                                        .and_then(|u| u.as_str().or_else(|| u.get("url").and_then(Value::as_str)))
                                        .unwrap_or("");
                                    let detail = item
                                        .get("image_url")
                                        .and_then(|u| u.get("detail").and_then(Value::as_str))
                                        .unwrap_or("auto");
                                    parts.push(json!({
                                        "type": "input_image",
                                        "image_url": url,
                                        "detail": detail
                                    }));
                                }
                                _ => {
                                    parts.push(json!({
                                        "type": content_type,
                                        "text": item.to_string()
                                    }));
                                }
                            }
                        }
                    }
                    if !parts.is_empty() {
                        input_items.push(json!({
                            "type": "message",
                            "role": role,
                            "content": parts
                        }));
                    }
                    if role == "assistant"
                        && let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array)
                    {
                        for tc in tool_calls {
                            let fn_obj = tc.get("function").and_then(Value::as_object);
                            let name = fn_obj
                                .and_then(|f| f.get("name"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .trim();
                            if name.is_empty() {
                                continue;
                            }
                            let call_id = tc
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("call_0")
                                .chars()
                                .take(64)
                                .collect::<String>();
                            let args = fn_obj
                                .and_then(|f| f.get("arguments"))
                                .map(|a| {
                                    if let Some(s) = a.as_str() {
                                        s.to_string()
                                    } else {
                                        a.to_string()
                                    }
                                })
                                .unwrap_or_else(|| "{}".to_string());
                            input_items.push(json!({
                                "type": "function_call",
                                "call_id": call_id,
                                "name": name,
                                "arguments": args
                            }));
                        }
                    }
                }
                "tool" => {
                    let call_id = msg
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_0")
                        .chars()
                        .take(64)
                        .collect::<String>();
                    let output = extract_text_content(msg.get("content"));
                    input_items.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output
                    }));
                }
                _ => {}
            }
        }
    }

    result.insert(
        "instructions".into(),
        Value::String(instructions_parts.join("\n\n")),
    );
    result.insert("input".into(), Value::Array(input_items));

    if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                if t.get("function").is_some() {
                    let fn_obj = t.get("function")?.as_object()?;
                    let name = fn_obj.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    let desc = fn_obj
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let mut params = fn_obj
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                    ensure_object_properties(&mut params);
                    let mut out = json!({
                        "type": "function",
                        "name": name,
                        "description": desc,
                        "parameters": params
                    });
                    if let Some(strict) = fn_obj.get("strict") {
                        out["strict"] = strict.clone();
                    }
                    Some(out)
                } else {
                    Some(t.clone())
                }
            })
            .collect();
        if !converted.is_empty() {
            result.insert("tools".into(), Value::Array(converted));
        }
    }

    if let Some(tc) = obj.get("tool_choice") {
        result.insert("tool_choice".into(), tc.clone());
    }
    if let Some(temp) = obj.get("temperature") {
        result.insert("temperature".into(), temp.clone());
    }
    if let Some(top_p) = obj.get("top_p") {
        result.insert("top_p".into(), top_p.clone());
    }
    if let Some(max_tok) = obj
        .get("max_output_tokens")
        .or_else(|| obj.get("max_completion_tokens"))
        .or_else(|| obj.get("max_tokens"))
    {
        result.insert("max_output_tokens".into(), max_tok.clone());
    }
    if let Some(r) = obj.get("reasoning") {
        result.insert("reasoning".into(), r.clone());
    } else if let Some(effort) = obj.get("reasoning_effort").and_then(Value::as_str) {
        result.insert(
            "reasoning".into(),
            json!({ "effort": effort, "summary": "auto" }),
        );
    }

    Value::Object(result)
}

pub fn responses_request_to_chat(model: &str, body: &Value) -> Value {
    let Some(obj) = body.as_object() else {
        return body.clone();
    };
    if !obj.contains_key("input") && obj.contains_key("messages") {
        let mut out = obj.clone();
        out.insert("model".into(), Value::String(model.to_string()));
        return Value::Object(out);
    }

    let mut result = obj.clone();
    result.insert("model".into(), Value::String(model.to_string()));
    let mut messages: Vec<Value> = Vec::new();

    if let Some(instr) = obj.get("instructions").and_then(Value::as_str)
        && !instr.trim().is_empty()
    {
        messages.push(json!({ "role": "system", "content": instr }));
    }

    let mut current_assistant: Option<Map<String, Value>> = None;
    let flush_assistant = |msgs: &mut Vec<Value>, cur: &mut Option<Map<String, Value>>| {
        if let Some(a) = cur.take() {
            msgs.push(Value::Object(a));
        }
    };

    if let Some(Value::String(s)) = obj.get("input") {
        messages.push(json!({ "role": "user", "content": s }));
    } else if let Some(items) = obj.get("input").and_then(Value::as_array) {
        for item in items {
            let item_type = item
                .get("type")
                .and_then(Value::as_str)
                .or_else(|| item.get("role").map(|_| "message"))
                .unwrap_or("");
            match item_type {
                "message" => {
                    flush_assistant(&mut messages, &mut current_assistant);
                    let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                    let content = convert_responses_message_content(item.get("content"));
                    messages.push(json!({ "role": role, "content": content }));
                }
                "function_call" => {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if name.is_empty() {
                        continue;
                    }
                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_0");
                    let args = item
                        .get("arguments")
                        .map(|a| {
                            if let Some(s) = a.as_str() {
                                s.to_string()
                            } else {
                                a.to_string()
                            }
                        })
                        .unwrap_or_else(|| "{}".to_string());
                    let assistant = current_assistant.get_or_insert_with(|| {
                        let mut m = Map::new();
                        m.insert("role".into(), Value::String("assistant".into()));
                        m.insert("content".into(), Value::Null);
                        m.insert("tool_calls".into(), Value::Array(Vec::new()));
                        m
                    });
                    if let Some(arr) = assistant
                        .get_mut("tool_calls")
                        .and_then(Value::as_array_mut)
                    {
                        arr.push(json!({
                            "id": call_id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args
                            }
                        }));
                    }
                }
                "function_call_output" => {
                    flush_assistant(&mut messages, &mut current_assistant);
                    let call_id = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_0");
                    let output = extract_text_content(item.get("output"));
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": output
                    }));
                }
                _ => {}
            }
        }
    }
    flush_assistant(&mut messages, &mut current_assistant);
    result.insert("messages".into(), Value::Array(messages));

    if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                if t.get("function").is_some() {
                    return Some(t.clone());
                }
                let name = t.get("name")?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                let desc = t.get("description").and_then(Value::as_str).unwrap_or("");
                let mut params = t
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                ensure_object_properties(&mut params);
                Some(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": desc,
                        "parameters": params
                    }
                }))
            })
            .collect();
        result.insert("tools".into(), Value::Array(converted));
    }

    if let Some(max_out) = result.remove("max_output_tokens") {
        result.entry("max_tokens").or_insert(max_out);
    }
    if let Some(reasoning) = result.remove("reasoning")
        && let Some(effort) = reasoning.get("effort").and_then(Value::as_str)
    {
        result.insert("reasoning_effort".into(), Value::String(effort.to_string()));
    }
    result.remove("input");
    result.remove("instructions");
    result.remove("include");
    result.remove("store");

    Value::Object(result)
}

fn convert_responses_message_content(val: Option<&Value>) -> Value {
    match val {
        Some(Value::String(s)) => Value::String(s.clone()),
        Some(Value::Array(arr)) => {
            let mapped: Vec<Value> = arr
                .iter()
                .map(|c| match c.get("type").and_then(Value::as_str) {
                    Some("input_text") | Some("output_text") | Some("text") => json!({
                        "type": "text",
                        "text": c.get("text").and_then(Value::as_str).unwrap_or("")
                    }),
                    Some("input_image") => {
                        let url = c
                            .get("image_url")
                            .or_else(|| c.get("file_id"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let detail = c.get("detail").and_then(Value::as_str).unwrap_or("auto");
                        json!({
                            "type": "image_url",
                            "image_url": { "url": url, "detail": detail }
                        })
                    }
                    _ => c.clone(),
                })
                .collect();
            Value::Array(mapped)
        }
        _ => Value::String(String::new()),
    }
}

fn extract_text_content(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|c| {
                c.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| c.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    }
}

fn ensure_object_properties(params: &mut Value) {
    if let Some(obj) = params.as_object_mut()
        && obj.get("type").and_then(Value::as_str) == Some("object")
        && !obj.contains_key("properties")
    {
        obj.insert("properties".into(), json!({}));
    }
}

#[derive(Debug, Default)]
pub struct ResponsesToChatStreamState {
    started: bool,
    chat_id: String,
    created: i64,
    model: String,
    next_tool_index: usize,
    item_to_tool_idx: HashMap<String, usize>,
    item_args_emitted: HashMap<usize, bool>,
    has_tool_calls: bool,
    finish_sent: bool,
    usage: Option<Value>,
}

pub fn translate_responses_event_to_chat(
    event_name: Option<&str>,
    data: &Value,
    state: &mut ResponsesToChatStreamState,
    fallback_model: &str,
) -> Vec<Value> {
    let ev_type = data
        .get("type")
        .and_then(Value::as_str)
        .or(event_name)
        .unwrap_or("");

    if !state.started {
        state.started = true;
        let now_sec = chrono::Utc::now().timestamp();
        state.chat_id = format!("chatcmpl-{now_sec}");
        state.created = now_sec;
        state.model = data
            .get("response")
            .and_then(|r| r.get("model"))
            .and_then(Value::as_str)
            .unwrap_or(fallback_model)
            .to_string();
    }

    match ev_type {
        "response.output_text.delta" => {
            let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
            if delta.is_empty() {
                return Vec::new();
            }
            vec![build_chat_chunk(
                &state.chat_id,
                state.created,
                &state.model,
                json!({ "content": delta }),
                None,
                None,
            )]
        }
        "response.reasoning_summary_text.delta" => {
            let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
            if delta.is_empty() {
                return Vec::new();
            }
            vec![build_chat_chunk(
                &state.chat_id,
                state.created,
                &state.model,
                json!({ "reasoning_content": delta }),
                None,
                None,
            )]
        }
        "response.output_item.added" => {
            let Some(item) = data.get("item").and_then(Value::as_object) else {
                return Vec::new();
            };
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            if item_type != "function_call" && item_type != "custom_tool_call" {
                return Vec::new();
            }
            state.has_tool_calls = true;
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or("call_0")
                .to_string();
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| call_id.clone());
            let idx = *state
                .item_to_tool_idx
                .entry(item_id)
                .or_insert_with(|| {
                    let i = state.next_tool_index;
                    state.next_tool_index += 1;
                    i
                });
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            vec![build_chat_chunk(
                &state.chat_id,
                state.created,
                &state.model,
                json!({
                    "tool_calls": [{
                        "index": idx,
                        "id": call_id,
                        "type": "function",
                        "function": { "name": name, "arguments": "" }
                    }]
                }),
                None,
                None,
            )]
        }
        "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
            let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
            if delta.is_empty() {
                return Vec::new();
            }
            let item_id = data.get("item_id").and_then(Value::as_str);
            let idx = item_id
                .and_then(|id| state.item_to_tool_idx.get(id).copied())
                .unwrap_or_else(|| state.next_tool_index.saturating_sub(1));
            state.item_args_emitted.insert(idx, true);
            vec![build_chat_chunk(
                &state.chat_id,
                state.created,
                &state.model,
                json!({
                    "tool_calls": [{
                        "index": idx,
                        "function": { "arguments": delta }
                    }]
                }),
                None,
                None,
            )]
        }
        "response.output_item.done" => {
            let Some(item) = data.get("item").and_then(Value::as_object) else {
                return Vec::new();
            };
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            if item_type != "function_call" && item_type != "custom_tool_call" {
                return Vec::new();
            }
            let key = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| data.get("item_id").and_then(Value::as_str));
            let idx = key
                .and_then(|k| state.item_to_tool_idx.get(k).copied())
                .unwrap_or_else(|| state.next_tool_index.saturating_sub(1));
            if !state.item_args_emitted.get(&idx).copied().unwrap_or(false)
                && let Some(full_args) = item.get("arguments").and_then(Value::as_str)
                && !full_args.is_empty()
            {
                state.item_args_emitted.insert(idx, true);
                return vec![build_chat_chunk(
                    &state.chat_id,
                    state.created,
                    &state.model,
                    json!({
                        "tool_calls": [{
                            "index": idx,
                            "function": { "arguments": full_args }
                        }]
                    }),
                    None,
                    None,
                )];
            }
            Vec::new()
        }
        "response.completed" | "response.done" => {
            if let Some(ru) = data
                .get("response")
                .and_then(|r| r.get("usage"))
                .and_then(Value::as_object)
            {
                let in_tok = ru.get("input_tokens").and_then(Value::as_i64).unwrap_or(0);
                let out_tok = ru.get("output_tokens").and_then(Value::as_i64).unwrap_or(0);
                let tot = ru
                    .get("total_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(in_tok + out_tok);
                state.usage = Some(json!({
                    "prompt_tokens": in_tok,
                    "completion_tokens": out_tok,
                    "total_tokens": tot
                }));
            }
            if state.finish_sent {
                return Vec::new();
            }
            state.finish_sent = true;
            let finish = if state.has_tool_calls {
                "tool_calls"
            } else {
                "stop"
            };
            vec![build_chat_chunk(
                &state.chat_id,
                state.created,
                &state.model,
                json!({}),
                Some(finish),
                state.usage.clone(),
            )]
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, Default)]
pub struct ChatToResponsesStreamState {
    started: bool,
    response_id: String,
    created: i64,
    model: String,
    seq: u64,
    next_output_index: usize,
    reasoning_open: bool,
    reasoning_id: String,
    reasoning_index: usize,
    reasoning_buf: String,
    msg_open: bool,
    msg_id: String,
    msg_index: usize,
    msg_buf: String,
    tool_calls: BTreeMap<usize, ToolCallTrack>,
    output_items: Vec<Value>,
    usage: Option<Value>,
    completed_sent: bool,
}

#[derive(Debug, Default)]
struct ToolCallTrack {
    output_index: usize,
    item_id: String,
    call_id: String,
    name: String,
    args: String,
    added_sent: bool,
    done_sent: bool,
}

impl ChatToResponsesStreamState {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn emit(&mut self, event: &str, mut data: Value) -> (String, Value) {
        let seq = self.next_seq();
        if let Some(obj) = data.as_object_mut() {
            obj.insert("sequence_number".into(), json!(seq));
        }
        (event.to_string(), data)
    }

    fn close_reasoning(&mut self, out: &mut Vec<(String, Value)>) {
        if !self.reasoning_open {
            return;
        }
        self.reasoning_open = false;
        let rid = self.reasoning_id.clone();
        let ridx = self.reasoning_index;
        let text = self.reasoning_buf.clone();
        out.push(self.emit(
            "response.reasoning_summary_text.done",
            json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": rid,
                "output_index": ridx,
                "summary_index": 0,
                "text": text
            }),
        ));
        let item = json!({
            "id": rid,
            "type": "reasoning",
            "summary": [{ "type": "summary_text", "text": text }]
        });
        out.push(self.emit(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": ridx,
                "item": item
            }),
        ));
        self.output_items.push(item);
    }

    fn close_message(&mut self, out: &mut Vec<(String, Value)>) {
        if !self.msg_open {
            return;
        }
        self.msg_open = false;
        let mid = self.msg_id.clone();
        let midx = self.msg_index;
        let text = self.msg_buf.clone();
        out.push(self.emit(
            "response.output_text.done",
            json!({
                "type": "response.output_text.done",
                "item_id": mid,
                "output_index": midx,
                "content_index": 0,
                "text": text
            }),
        ));
        let item = json!({
            "id": mid,
            "type": "message",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }]
        });
        out.push(self.emit(
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": midx,
                "item": item
            }),
        ));
        self.output_items.push(item);
    }

    fn close_tools(&mut self, out: &mut Vec<(String, Value)>) {
        let keys: Vec<usize> = self.tool_calls.keys().copied().collect();
        for k in keys {
            let (done_sent, item_id, call_id, name, args, oidx) = {
                let tc = self.tool_calls.get_mut(&k).unwrap();
                if tc.done_sent || !tc.added_sent {
                    continue;
                }
                tc.done_sent = true;
                (
                    tc.done_sent,
                    tc.item_id.clone(),
                    tc.call_id.clone(),
                    tc.name.clone(),
                    tc.args.clone(),
                    tc.output_index,
                )
            };
            let _ = done_sent;
            out.push(self.emit(
                "response.function_call_arguments.done",
                json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": item_id,
                    "output_index": oidx,
                    "arguments": args
                }),
            ));
            let item = json!({
                "id": item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": args
            });
            out.push(self.emit(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": oidx,
                    "item": item
                }),
            ));
            self.output_items.push(item);
        }
    }
}

pub fn translate_chat_chunk_to_responses(
    chunk: &Value,
    state: &mut ChatToResponsesStreamState,
    fallback_model: &str,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(u) = chunk.get("usage").and_then(Value::as_object) {
        let p = u.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0);
        let c = u
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if p + c > 0 {
            state.usage = Some(json!({
                "input_tokens": p,
                "output_tokens": c,
                "total_tokens": p + c
            }));
        }
    }

    if !state.started {
        state.started = true;
        state.created = chrono::Utc::now().timestamp();
        let raw_id = chunk
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("resp_0");
        state.response_id = format!("resp_{}", raw_id.trim_start_matches("chatcmpl-"));
        state.model = chunk
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(fallback_model)
            .to_string();
        let resp_id = state.response_id.clone();
        let created = state.created;
        let model = state.model.clone();
        out.push(state.emit(
            "response.created",
            json!({
                "type": "response.created",
                "response": {
                    "id": resp_id,
                    "object": "response",
                    "created_at": created,
                    "model": model,
                    "status": "in_progress",
                    "output": []
                }
            }),
        ));
    }

    let Some(choice) = chunk
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    else {
        return out;
    };
    let delta = choice.get("delta").and_then(Value::as_object);

    if let Some(d) = delta {
        if let Some(rc) = d
            .get("reasoning_content")
            .or_else(|| d.get("reasoning"))
            .and_then(Value::as_str)
            && !rc.is_empty()
        {
            if !state.reasoning_open {
                state.reasoning_open = true;
                let oidx = state.next_output_index;
                state.next_output_index += 1;
                state.reasoning_index = oidx;
                state.reasoning_id = format!("rs_{}_{}", state.response_id, oidx);
                let rid = state.reasoning_id.clone();
                out.push(state.emit(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": oidx,
                        "item": { "id": rid, "type": "reasoning", "summary": [] }
                    }),
                ));
            }
            state.reasoning_buf.push_str(rc);
            let rid = state.reasoning_id.clone();
            let ridx = state.reasoning_index;
            out.push(state.emit(
                "response.reasoning_summary_text.delta",
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": rid,
                    "output_index": ridx,
                    "summary_index": 0,
                    "delta": rc
                }),
            ));
        }

        if let Some(content) = d.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            state.close_reasoning(&mut out);
            if !state.msg_open {
                state.msg_open = true;
                let oidx = state.next_output_index;
                state.next_output_index += 1;
                state.msg_index = oidx;
                state.msg_id = format!("msg_{}_{}", state.response_id, oidx);
                let mid = state.msg_id.clone();
                out.push(state.emit(
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": oidx,
                        "item": { "id": mid, "type": "message", "role": "assistant", "content": [] }
                    }),
                ));
                out.push(state.emit(
                    "response.content_part.added",
                    json!({
                        "type": "response.content_part.added",
                        "item_id": mid,
                        "output_index": oidx,
                        "content_index": 0,
                        "part": { "type": "output_text", "text": "", "annotations": [] }
                    }),
                ));
            }
            state.msg_buf.push_str(content);
            let mid = state.msg_id.clone();
            let midx = state.msg_index;
            out.push(state.emit(
                "response.output_text.delta",
                json!({
                    "type": "response.output_text.delta",
                    "item_id": mid,
                    "output_index": midx,
                    "content_index": 0,
                    "delta": content
                }),
            ));
        }

        if let Some(tool_calls) = d.get("tool_calls").and_then(Value::as_array) {
            state.close_reasoning(&mut out);
            state.close_message(&mut out);
            for tc in tool_calls {
                let tc_idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let next_oidx = state.next_output_index;
                let entry = state.tool_calls.entry(tc_idx).or_insert_with(|| {
                    state.next_output_index += 1;
                    ToolCallTrack {
                        output_index: next_oidx,
                        call_id: format!("call_{tc_idx}"),
                        item_id: format!("fc_call_{tc_idx}"),
                        ..Default::default()
                    }
                });
                if let Some(cid) = tc.get("id").and_then(Value::as_str)
                    && !cid.is_empty()
                {
                    entry.call_id = cid.to_string();
                    entry.item_id = format!("fc_{cid}");
                }
                if let Some(name) = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    && !name.is_empty()
                {
                    entry.name = name.to_string();
                }
                let need_add = !entry.added_sent && !entry.name.is_empty();
                let item_id = entry.item_id.clone();
                let call_id = entry.call_id.clone();
                let name = entry.name.clone();
                let oidx = entry.output_index;
                let args_delta = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !args_delta.is_empty() {
                    entry.args.push_str(&args_delta);
                }
                if need_add {
                    entry.added_sent = true;
                    out.push(state.emit(
                        "response.output_item.added",
                        json!({
                            "type": "response.output_item.added",
                            "output_index": oidx,
                            "item": {
                                "id": item_id,
                                "type": "function_call",
                                "call_id": call_id,
                                "name": name,
                                "arguments": ""
                            }
                        }),
                    ));
                }
                if !args_delta.is_empty() {
                    out.push(state.emit(
                        "response.function_call_arguments.delta",
                        json!({
                            "type": "response.function_call_arguments.delta",
                            "item_id": item_id,
                            "output_index": oidx,
                            "delta": args_delta
                        }),
                    ));
                }
            }
        }
    }

    if choice.get("finish_reason").is_some_and(|v| !v.is_null()) {
        state.close_reasoning(&mut out);
        state.close_message(&mut out);
        state.close_tools(&mut out);
    }

    out
}

pub fn flush_chat_to_responses(state: &mut ChatToResponsesStreamState) -> Vec<(String, Value)> {
    if !state.started || state.completed_sent {
        return Vec::new();
    }
    let mut out = Vec::new();
    state.close_reasoning(&mut out);
    state.close_message(&mut out);
    state.close_tools(&mut out);
    state.completed_sent = true;

    let mut resp = json!({
        "id": state.response_id,
        "object": "response",
        "created_at": state.created,
        "model": state.model,
        "status": "completed",
        "output": state.output_items
    });
    if let Some(u) = &state.usage {
        resp["usage"] = u.clone();
    }
    out.push(state.emit(
        "response.completed",
        json!({
            "type": "response.completed",
            "response": resp
        }),
    ));
    out
}

pub(crate) fn build_chat_chunk(
    id: &str,
    created: i64,
    model: &str,
    delta: Value,
    finish_reason: Option<&str>,
    usage: Option<Value>,
) -> Value {
    let mut chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason
        }]
    });
    if let Some(u) = usage {
        chunk["usage"] = u;
    }
    chunk
}
