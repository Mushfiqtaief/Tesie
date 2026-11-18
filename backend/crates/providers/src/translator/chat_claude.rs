use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Map, Value};

use crate::translator::chat_responses::build_chat_chunk;

pub fn chat_request_to_claude(model: &str, body: &Value) -> Value {
    let Some(obj) = body.as_object() else {
        return body.clone();
    };
    let mut result = Map::new();
    result.insert("model".into(), Value::String(model.to_string()));
    let max_tokens = obj
        .get("max_tokens")
        .or_else(|| obj.get("max_completion_tokens"))
        .and_then(Value::as_i64)
        .unwrap_or(16_384);
    result.insert("max_tokens".into(), json!(max_tokens));
    result.insert(
        "stream".into(),
        obj.get("stream").cloned().unwrap_or(Value::Bool(true)),
    );
    if let Some(temp) = obj.get("temperature") {
        result.insert("temperature".into(), temp.clone());
    }
    if let Some(top_p) = obj.get("top_p") {
        result.insert("top_p".into(), top_p.clone());
    }

    let mut system_parts: Vec<String> = Vec::new();
    let mut claude_messages: Vec<Value> = Vec::new();

    if let Some(messages) = obj.get("messages").and_then(Value::as_array) {
        let mut current_role: Option<&'static str> = None;
        let mut current_blocks: Vec<Value> = Vec::new();

        let flush = |role: &mut Option<&'static str>, blocks: &mut Vec<Value>, out: &mut Vec<Value>| {
            if let Some(r) = role.take()
                && !blocks.is_empty()
            {
                out.push(json!({ "role": r, "content": std::mem::take(blocks) }));
            }
        };

        for msg in messages {
            let raw_role = msg.get("role").and_then(Value::as_str).unwrap_or("");
            if raw_role == "system" || raw_role == "developer" {
                let txt = extract_text(msg.get("content"));
                if !txt.is_empty() {
                    system_parts.push(txt);
                }
                continue;
            }

            if raw_role == "tool" {
                let tool_use_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("call_0");
                let content = extract_text(msg.get("content"));
                if current_role != Some("user") {
                    flush(&mut current_role, &mut current_blocks, &mut claude_messages);
                    current_role = Some("user");
                }
                current_blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content
                }));
                continue;
            }

            let target_role = if raw_role == "user" {
                "user"
            } else {
                "assistant"
            };
            if current_role != Some(target_role) {
                flush(&mut current_role, &mut current_blocks, &mut claude_messages);
                current_role = Some(target_role);
            }

            if let Some(s) = msg.get("content").and_then(Value::as_str) {
                if !s.is_empty() {
                    current_blocks.push(json!({ "type": "text", "text": s }));
                }
            } else if let Some(arr) = msg.get("content").and_then(Value::as_array) {
                for part in arr {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = part.get("text").and_then(Value::as_str)
                                && !t.is_empty()
                            {
                                current_blocks.push(json!({ "type": "text", "text": t }));
                            }
                        }
                        Some("tool_use") | Some("tool_result") | Some("thinking") => {
                            current_blocks.push(part.clone());
                        }
                        _ => {}
                    }
                }
            }

            if target_role == "assistant"
                && let Some(calls) = msg.get("tool_calls").and_then(Value::as_array)
            {
                for tc in calls {
                    let fn_obj = tc.get("function").and_then(Value::as_object);
                    let name = fn_obj
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if name.is_empty() {
                        continue;
                    }
                    let id = tc.get("id").and_then(Value::as_str).unwrap_or("call_0");
                    let input_val = fn_obj
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| {
                            a.as_str()
                                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                                .or_else(|| if a.is_object() { Some(a.clone()) } else { None })
                        })
                        .unwrap_or_else(|| json!({}));
                    current_blocks.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input_val
                    }));
                }
            }
        }
        flush(&mut current_role, &mut current_blocks, &mut claude_messages);
    }

    if !system_parts.is_empty() {
        result.insert(
            "system".into(),
            json!([{ "type": "text", "text": system_parts.join("\n\n") }]),
        );
    }
    result.insert("messages".into(), Value::Array(claude_messages));

    if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
        let mapped: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                if let Some(fn_obj) = t.get("function").and_then(Value::as_object) {
                    let name = fn_obj.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    let desc = fn_obj
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let schema = fn_obj
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                    Some(json!({
                        "name": name,
                        "description": desc,
                        "input_schema": schema
                    }))
                } else if t.get("input_schema").is_some() {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect();
        if !mapped.is_empty() {
            result.insert("tools".into(), Value::Array(mapped));
        }
    }

    if let Some(tc) = obj.get("tool_choice") {
        let converted = match tc {
            Value::String(s) if s == "required" => json!({ "type": "any" }),
            Value::String(s) if s == "none" => json!({ "type": "none" }),
            Value::Object(m) => {
                if let Some(name) = m
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                {
                    json!({ "type": "tool", "name": name })
                } else {
                    json!({ "type": "auto" })
                }
            }
            _ => json!({ "type": "auto" }),
        };
        result.insert("tool_choice".into(), converted);
    }

    Value::Object(result)
}

pub fn claude_request_to_chat(model: &str, body: &Value) -> Value {
    let Some(obj) = body.as_object() else {
        return body.clone();
    };
    let mut result = Map::new();
    result.insert("model".into(), Value::String(model.to_string()));
    result.insert(
        "stream".into(),
        obj.get("stream").cloned().unwrap_or(Value::Bool(true)),
    );
    if let Some(mt) = obj.get("max_tokens") {
        result.insert("max_tokens".into(), mt.clone());
    }
    if let Some(temp) = obj.get("temperature") {
        result.insert("temperature".into(), temp.clone());
    }
    if let Some(top_p) = obj.get("top_p") {
        result.insert("top_p".into(), top_p.clone());
    }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = obj.get("system") {
        let sys_text = extract_text(Some(sys));
        if !sys_text.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": sys_text }));
        }
    }

    if let Some(in_msgs) = obj.get("messages").and_then(Value::as_array) {
        for msg in in_msgs {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
            if let Some(s) = msg.get("content").and_then(Value::as_str) {
                messages.push(json!({ "role": role, "content": s }));
                continue;
            }
            let Some(blocks) = msg.get("content").and_then(Value::as_array) else {
                continue;
            };
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            let mut tool_results = Vec::new();
            let mut reasoning = String::new();

            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            text_parts.push(t.to_string());
                        }
                    }
                    Some("thinking") => {
                        if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                            reasoning.push_str(t);
                        }
                    }
                    Some("tool_use") => {
                        let id = b.get("id").and_then(Value::as_str).unwrap_or("call_0");
                        let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                        let input = b.get("input").cloned().unwrap_or_else(|| json!({}));
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": input.to_string()
                            }
                        }));
                    }
                    Some("tool_result") => {
                        let tid = b
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or("call_0");
                        let content = extract_text(b.get("content"));
                        tool_results.push(json!({
                            "role": "tool",
                            "tool_call_id": tid,
                            "content": content
                        }));
                    }
                    _ => {}
                }
            }

            if !tool_results.is_empty() {
                messages.extend(tool_results);
            }
            if !text_parts.is_empty() || !tool_calls.is_empty() {
                let mut m = Map::new();
                m.insert("role".into(), Value::String(role.to_string()));
                if text_parts.is_empty() {
                    m.insert("content".into(), Value::Null);
                } else {
                    m.insert("content".into(), Value::String(text_parts.join("")));
                }
                if !tool_calls.is_empty() {
                    m.insert("tool_calls".into(), Value::Array(tool_calls));
                }
                if !reasoning.is_empty() {
                    m.insert("reasoning_content".into(), Value::String(reasoning));
                }
                messages.push(Value::Object(m));
            }
        }
    }
    result.insert("messages".into(), Value::Array(messages));

    if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
        let mapped: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                let desc = t.get("description").and_then(Value::as_str).unwrap_or("");
                let params = t
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
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
        if !mapped.is_empty() {
            result.insert("tools".into(), Value::Array(mapped));
        }
    }

    if let Some(tc) = obj.get("tool_choice").and_then(Value::as_object) {
        let mapped = match tc.get("type").and_then(Value::as_str) {
            Some("any") => json!("required"),
            Some("none") => json!("none"),
            Some("tool") => json!({
                "type": "function",
                "function": { "name": tc.get("name").and_then(Value::as_str).unwrap_or("") }
            }),
            _ => json!("auto"),
        };
        result.insert("tool_choice".into(), mapped);
    }

    Value::Object(result)
}

fn extract_text(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    }
}

#[derive(Debug, Default)]
pub struct ClaudeToChatStreamState {
    message_id: String,
    model: String,
    created: i64,
    next_tool_index: usize,
    block_to_tool_idx: HashMap<usize, usize>,
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_create_tokens: i64,
    output_tokens: i64,
    finish_sent: bool,
}

pub fn translate_claude_event_to_chat(
    event_name: Option<&str>,
    data: &Value,
    state: &mut ClaudeToChatStreamState,
    fallback_model: &str,
) -> Vec<Value> {
    let ev = data
        .get("type")
        .and_then(Value::as_str)
        .or(event_name)
        .unwrap_or("");
    let mut out = Vec::new();

    match ev {
        "message_start" => {
            state.created = chrono::Utc::now().timestamp();
            let msg = data.get("message");
            let raw_id = msg
                .and_then(|m| m.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("msg_0");
            state.message_id = format!("chatcmpl-{raw_id}");
            state.model = msg
                .and_then(|m| m.get("model"))
                .and_then(Value::as_str)
                .unwrap_or(fallback_model)
                .to_string();
            if let Some(u) = msg.and_then(|m| m.get("usage")).and_then(Value::as_object) {
                state.input_tokens = u.get("input_tokens").and_then(Value::as_i64).unwrap_or(0);
                state.cache_read_tokens = u
                    .get("cache_read_input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                state.cache_create_tokens = u
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
            }
            out.push(build_chat_chunk(
                &state.message_id,
                state.created,
                &state.model,
                json!({ "role": "assistant" }),
                None,
                None,
            ));
        }
        "content_block_start" => {
            let block_idx = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(cb) = data.get("content_block").and_then(Value::as_object)
                && cb.get("type").and_then(Value::as_str) == Some("tool_use")
            {
                let tc_idx = state.next_tool_index;
                state.next_tool_index += 1;
                state.block_to_tool_idx.insert(block_idx, tc_idx);
                let id = cb.get("id").and_then(Value::as_str).unwrap_or("call_0");
                let name = cb.get("name").and_then(Value::as_str).unwrap_or("");
                out.push(build_chat_chunk(
                    &state.message_id,
                    state.created,
                    &state.model,
                    json!({
                        "tool_calls": [{
                            "index": tc_idx,
                            "id": id,
                            "type": "function",
                            "function": { "name": name, "arguments": "" }
                        }]
                    }),
                    None,
                    None,
                ));
            }
        }
        "content_block_delta" => {
            let block_idx = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(delta) = data.get("delta").and_then(Value::as_object) {
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            out.push(build_chat_chunk(
                                &state.message_id,
                                state.created,
                                &state.model,
                                json!({ "content": text }),
                                None,
                                None,
                            ));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(thinking) = delta.get("thinking").and_then(Value::as_str)
                            && !thinking.is_empty()
                        {
                            out.push(build_chat_chunk(
                                &state.message_id,
                                state.created,
                                &state.model,
                                json!({ "reasoning_content": thinking }),
                                None,
                                None,
                            ));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                            && !partial.is_empty()
                            && let Some(&tc_idx) = state.block_to_tool_idx.get(&block_idx)
                        {
                            out.push(build_chat_chunk(
                                &state.message_id,
                                state.created,
                                &state.model,
                                json!({
                                    "tool_calls": [{
                                        "index": tc_idx,
                                        "function": { "arguments": partial }
                                    }]
                                }),
                                None,
                                None,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        "message_delta" => {
            if let Some(u) = data.get("usage").and_then(Value::as_object) {
                if let Some(it) = u.get("input_tokens").and_then(Value::as_i64) {
                    state.input_tokens = it;
                }
                if let Some(ot) = u.get("output_tokens").and_then(Value::as_i64) {
                    state.output_tokens = ot;
                }
            }
            if let Some(stop_reason) = data
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(Value::as_str)
                && !state.finish_sent
            {
                state.finish_sent = true;
                let finish = match stop_reason {
                    "tool_use" => "tool_calls",
                    "max_tokens" => "length",
                    _ => "stop",
                };
                let prompt_tokens =
                    state.input_tokens + state.cache_read_tokens + state.cache_create_tokens;
                let usage = json!({
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": state.output_tokens,
                    "total_tokens": prompt_tokens + state.output_tokens
                });
                out.push(build_chat_chunk(
                    &state.message_id,
                    state.created,
                    &state.model,
                    json!({}),
                    Some(finish),
                    Some(usage),
                ));
            }
        }
        _ => {}
    }

    out
}

#[derive(Debug, Default)]
pub struct ChatToClaudeStreamState {
    started: bool,
    message_id: String,
    model: String,
    next_block_index: usize,
    thinking_open: bool,
    thinking_index: usize,
    text_open: bool,
    text_index: usize,
    tool_blocks: BTreeMap<usize, usize>,
    finish_reason: Option<String>,
    usage: Option<Value>,
    stopped: bool,
}

impl ChatToClaudeStreamState {
    fn close_thinking(&mut self, out: &mut Vec<(String, Value)>) {
        if self.thinking_open {
            self.thinking_open = false;
            out.push((
                "content_block_stop".into(),
                json!({ "type": "content_block_stop", "index": self.thinking_index }),
            ));
        }
    }

    fn close_text(&mut self, out: &mut Vec<(String, Value)>) {
        if self.text_open {
            self.text_open = false;
            out.push((
                "content_block_stop".into(),
                json!({ "type": "content_block_stop", "index": self.text_index }),
            ));
        }
    }
}

pub fn translate_chat_chunk_to_claude(
    chunk: &Value,
    state: &mut ChatToClaudeStreamState,
    fallback_model: &str,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(u) = chunk.get("usage").and_then(Value::as_object) {
        let p = u.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0);
        let c = u
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        state.usage = Some(json!({ "input_tokens": p, "output_tokens": c }));
    }

    if !state.started {
        state.started = true;
        let raw_id = chunk
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("msg_0")
            .trim_start_matches("chatcmpl-");
        state.message_id = raw_id.to_string();
        state.model = chunk
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(fallback_model)
            .to_string();
        out.push((
            "message_start".into(),
            json!({
                "type": "message_start",
                "message": {
                    "id": state.message_id,
                    "type": "message",
                    "role": "assistant",
                    "model": state.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
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

    if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
        if let Some(rc) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(Value::as_str)
            && !rc.is_empty()
        {
            state.close_text(&mut out);
            if !state.thinking_open {
                state.thinking_open = true;
                state.thinking_index = state.next_block_index;
                state.next_block_index += 1;
                out.push((
                    "content_block_start".into(),
                    json!({
                        "type": "content_block_start",
                        "index": state.thinking_index,
                        "content_block": { "type": "thinking", "thinking": "" }
                    }),
                ));
            }
            out.push((
                "content_block_delta".into(),
                json!({
                    "type": "content_block_delta",
                    "index": state.thinking_index,
                    "delta": { "type": "thinking_delta", "thinking": rc }
                }),
            ));
        }

        if let Some(content) = delta.get("content").and_then(Value::as_str)
            && !content.is_empty()
        {
            state.close_thinking(&mut out);
            if !state.text_open {
                state.text_open = true;
                state.text_index = state.next_block_index;
                state.next_block_index += 1;
                out.push((
                    "content_block_start".into(),
                    json!({
                        "type": "content_block_start",
                        "index": state.text_index,
                        "content_block": { "type": "text", "text": "" }
                    }),
                ));
            }
            out.push((
                "content_block_delta".into(),
                json!({
                    "type": "content_block_delta",
                    "index": state.text_index,
                    "delta": { "type": "text_delta", "text": content }
                }),
            ));
        }

        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            state.close_thinking(&mut out);
            state.close_text(&mut out);
            for tc in calls {
                let tc_idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                if !state.tool_blocks.contains_key(&tc_idx) {
                    let bidx = state.next_block_index;
                    state.next_block_index += 1;
                    state.tool_blocks.insert(tc_idx, bidx);
                    let id = tc.get("id").and_then(Value::as_str).unwrap_or("call_0");
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    out.push((
                        "content_block_start".into(),
                        json!({
                            "type": "content_block_start",
                            "index": bidx,
                            "content_block": {
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": {}
                            }
                        }),
                    ));
                }
                if let Some(partial) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    && !partial.is_empty()
                {
                    let bidx = state.tool_blocks[&tc_idx];
                    out.push((
                        "content_block_delta".into(),
                        json!({
                            "type": "content_block_delta",
                            "index": bidx,
                            "delta": { "type": "input_json_delta", "partial_json": partial }
                        }),
                    ));
                }
            }
        }
    }

    if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
        state.finish_reason = Some(fr.to_string());
    }

    out
}

pub fn flush_chat_to_claude(state: &mut ChatToClaudeStreamState) -> Vec<(String, Value)> {
    if !state.started || state.stopped {
        return Vec::new();
    }
    state.stopped = true;
    let mut out = Vec::new();
    state.close_thinking(&mut out);
    state.close_text(&mut out);
    let tool_indices: Vec<usize> = state.tool_blocks.values().copied().collect();
    for bidx in tool_indices {
        out.push((
            "content_block_stop".into(),
            json!({ "type": "content_block_stop", "index": bidx }),
        ));
    }

    let stop_reason = match state.finish_reason.as_deref() {
        Some("tool_calls") => "tool_use",
        Some("length") => "max_tokens",
        _ => {
            if state.tool_blocks.is_empty() {
                "end_turn"
            } else {
                "tool_use"
            }
        }
    };
    let usage = state
        .usage
        .clone()
        .unwrap_or_else(|| json!({ "input_tokens": 0, "output_tokens": 0 }));
    out.push((
        "message_delta".into(),
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": stop_reason, "stop_sequence": null },
            "usage": usage
        }),
    ));
    out.push(("message_stop".into(), json!({ "type": "message_stop" })));
    out
}
