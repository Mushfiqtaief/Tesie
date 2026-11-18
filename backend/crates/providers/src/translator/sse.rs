use std::pin::Pin;

use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        self.drain_events(false)
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        self.drain_events(true)
    }

    fn drain_events(&mut self, eof: bool) -> Vec<SseEvent> {
        let mut events = Vec::new();
        loop {
            if let Some((idx, sep_len)) = find_event_boundary(&self.buf) {
                let raw = self.buf[..idx].to_vec();
                self.buf.drain(..idx + sep_len);
                if let Some(ev) = parse_sse_block(&raw) {
                    events.push(ev);
                }
            } else if eof && !self.buf.is_empty() {
                let raw = std::mem::take(&mut self.buf);
                if let Some(ev) = parse_sse_block(&raw) {
                    events.push(ev);
                }
                break;
            } else {
                break;
            }
        }
        events
    }
}

fn find_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some((i, 4));
        }
        i += 1;
    }
    None
}

fn parse_sse_block(raw: &[u8]) -> Option<SseEvent> {
    let text = String::from_utf8_lossy(raw);
    let mut event_name: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();

    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim_start().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    if data_lines.is_empty() {
        return None;
    }

    Some(SseEvent {
        event: event_name,
        data: data_lines.join("\n"),
    })
}

pub fn format_sse_frame(event: Option<&str>, data: &Value) -> Bytes {
    let mut s = String::new();
    if let Some(ev) = event
        && !ev.is_empty()
    {
        s.push_str("event: ");
        s.push_str(ev);
        s.push('\n');
    }
    s.push_str("data: ");
    s.push_str(&data.to_string());
    s.push_str("\n\n");
    Bytes::from(s)
}

pub fn format_sse_done() -> Bytes {
    Bytes::from_static(b"data: [DONE]\n\n")
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct UsageAccumulator {
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub emitted_chars: usize,
}

impl UsageAccumulator {
    pub fn observe_payload(&mut self, payload: &Value) {
        // Count streamed text/reasoning/tool-arg characters as fallback when upstream omits usage
        if let Some(choices) = payload.get("choices").and_then(Value::as_array) {
            for choice in choices {
                if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
                    if let Some(c) = delta.get("content").and_then(Value::as_str) {
                        self.emitted_chars += c.chars().count();
                    }
                    if let Some(rc) = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning"))
                        .and_then(Value::as_str)
                    {
                        self.emitted_chars += rc.chars().count();
                    }
                    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                        for tc in calls {
                            if let Some(args) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(Value::as_str)
                            {
                                self.emitted_chars += args.chars().count();
                            }
                        }
                    }
                }
            }
        }
        // OpenAI Responses delta
        if let Some(ev_type) = payload.get("type").and_then(Value::as_str)
            && matches!(
                ev_type,
                "response.output_text.delta"
                    | "response.function_call_arguments.delta"
                    | "response.reasoning_summary_text.delta"
            )
            && let Some(d) = payload.get("delta").and_then(Value::as_str)
        {
            self.emitted_chars += d.chars().count();
        }
        // Anthropic delta
        if let Some(delta) = payload.get("delta").and_then(Value::as_object) {
            for k in ["text", "thinking", "partial_json"] {
                if let Some(s) = delta.get(k).and_then(Value::as_str) {
                    self.emitted_chars += s.chars().count();
                }
            }
        }

        // 1. OpenAI Chat Completions usage
        if let Some(u) = payload.get("usage").and_then(Value::as_object) {
            self.observe_usage_obj(u);
        }
        // 2. OpenAI Responses API response.completed / response.done
        if let Some(u) = payload
            .get("response")
            .and_then(|r| r.get("usage"))
            .and_then(Value::as_object)
        {
            self.observe_usage_obj(u);
        }
        // 3. Anthropic message_start message.usage
        if let Some(u) = payload
            .get("message")
            .and_then(|m| m.get("usage"))
            .and_then(Value::as_object)
        {
            self.observe_usage_obj(u);
        }
    }

    pub fn finalize_fallback(&mut self) {
        if self.completion_tokens == 0 && self.emitted_chars > 0 {
            self.completion_tokens = ((self.emitted_chars + 3) / 4).max(1) as i64;
            self.total_tokens = self
                .total_tokens
                .max(self.prompt_tokens + self.completion_tokens);
        }
    }

    fn observe_usage_obj(&mut self, u: &serde_json::Map<String, Value>) {
        let cached = u
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(Value::as_i64)
            .or_else(|| {
                u.get("input_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(Value::as_i64)
            })
            .or_else(|| u.get("cache_read_input_tokens").and_then(Value::as_i64))
            .or_else(|| {
                u.get("opencode")
                    .and_then(|o| o.get("tokens"))
                    .and_then(|t| t.get("cache_read"))
                    .and_then(Value::as_i64)
            });

        // Normalize `prompt_tokens` to NON-CACHED input tokens (matching 9router), while tracking
        // `cached_tokens` separately:
        // - OpenAI Chat (`prompt_tokens`) and Responses (`input_tokens` without `cache_read_input_tokens`)
        //   report gross input tokens (uncached + cached), so subtract `cached`.
        // - Anthropic (`cache_read_input_tokens` / `cache_creation_input_tokens`) reports non-cached
        //   `input_tokens` separately from `cache_read_input_tokens`, so do NOT add `cache_read`.
        let is_anthropic_shape = u.contains_key("cache_read_input_tokens")
            || u.contains_key("cache_creation_input_tokens");

        let uncached_p = if let Some(gross) = u.get("prompt_tokens").and_then(Value::as_i64) {
            Some((gross - cached.unwrap_or(0)).max(0))
        } else if let Some(base) = u.get("input_tokens").and_then(Value::as_i64) {
            if is_anthropic_shape {
                let cache_create = u
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                Some((base + cache_create).max(0))
            } else {
                Some((base - cached.unwrap_or(0)).max(0))
            }
        } else {
            None
        };

        let c = u
            .get("completion_tokens")
            .and_then(Value::as_i64)
            .or_else(|| u.get("output_tokens").and_then(Value::as_i64));

        if let Some(pv) = uncached_p
            && pv > self.prompt_tokens
        {
            self.prompt_tokens = pv;
        }
        if let Some(cv_cached) = cached
            && cv_cached > self.cached_tokens
        {
            self.cached_tokens = cv_cached;
        }
        if let Some(cv) = c
            && cv > self.completion_tokens
        {
            self.completion_tokens = cv;
        }
        self.total_tokens = self.prompt_tokens + self.completion_tokens;
    }
}

/// Returns true if the message or error indicates upstream quota exhaustion or free tier limits
pub fn is_upstream_quota_exhausted(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("freeusagelimiterror")
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
        || lower.contains("too many requests")
}

/// Inspects a raw SSE `data:` JSON string for an upstream error payload emitted on HTTP `200 OK`
/// (before any valid text/tool delta has been streamed). Returns `Some((is_quota_exhausted, detail))`.
pub fn classify_sse_error_event(raw_data: &str) -> Option<(bool, String)> {
    let trimmed = raw_data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return None;
    }
    let val = serde_json::from_str::<Value>(trimmed).ok()?;
    let ev_type = val.get("type").and_then(Value::as_str).unwrap_or("");
    let err_val = val.get("error");
    let msg_val = val.get("message").and_then(Value::as_str).unwrap_or("");
    let is_quota = is_upstream_quota_exhausted(trimmed) || is_upstream_quota_exhausted(msg_val);
    let has_error = ev_type == "error" || err_val.is_some() || is_quota;
    if !has_error {
        return None;
    }
    Some((is_quota, trimmed.chars().take(1024).collect()))
}

pub enum SsePreCommit {
    Ok {
        stream: Pin<Box<dyn Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send>>,
    },
    UpstreamError {
        is_quota: bool,
        raw_detail: String,
    },
    TransportError(String),
}

/// Buffers ONLY until the first complete SSE frame (or EOF / 16 KB) on an HTTP `200 OK`
/// stream so upstream errors emitted inside the first SSE chunk (e.g. `FreeUsageLimitError`
/// or `RESOURCE_EXHAUSTED`) are intercepted BEFORE any HTTP 200 status/chunk goes downstream.
pub async fn pre_commit_sse_response(resp: reqwest::Response) -> SsePreCommit {
    let mut upstream = Box::pin(resp.bytes_stream());
    let mut raw_buf: Vec<u8> = Vec::new();
    let mut eof = false;

    loop {
        if find_event_boundary(&raw_buf).is_some() || raw_buf.len() >= 16_384 {
            break;
        }
        match upstream.next().await {
            Some(Ok(chunk)) => {
                raw_buf.extend_from_slice(&chunk);
            }
            Some(Err(err)) => {
                if raw_buf.is_empty() {
                    return SsePreCommit::TransportError(err.to_string());
                }
                eof = true;
                break;
            }
            None => {
                eof = true;
                break;
            }
        }
    }

    if raw_buf.is_empty() && eof {
        return SsePreCommit::TransportError("upstream closed stream before first event".into());
    }

    let mut sse = SseBuffer::default();
    let mut events = sse.push(&raw_buf);
    if eof {
        events.extend(sse.finish());
    }

    for ev in &events {
        if let Some((is_quota, detail)) = classify_sse_error_event(&ev.data) {
            return SsePreCommit::UpstreamError {
                is_quota,
                raw_detail: detail,
            };
        }
    }

    if events.is_empty() && eof {
        let txt = String::from_utf8_lossy(&raw_buf);
        if let Some((is_quota, detail)) = classify_sse_error_event(txt.trim()) {
            return SsePreCommit::UpstreamError {
                is_quota,
                raw_detail: detail,
            };
        }
    }

    let head = Bytes::from(raw_buf);
    let chained = stream::once(async move { Ok(head) }).chain(upstream);
    SsePreCommit::Ok {
        stream: Box::pin(chained),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_split_sse_chunks_and_usage() {
        let mut buf = SseBuffer::default();
        assert!(buf.push(b"event: message_start\nda").is_empty());
        let evs = buf.push(b"ta: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].event.as_deref(), Some("message_start"));

        let mut acc = UsageAccumulator::default();
        let val: Value = serde_json::from_str(&evs[0].data).unwrap();
        acc.observe_payload(&val);
        acc.observe_payload(&json!({ "type": "message_delta", "usage": { "output_tokens": 8 } }));
        assert_eq!(acc.prompt_tokens, 12);
        assert_eq!(acc.completion_tokens, 8);
        assert_eq!(acc.total_tokens, 20);
    }

    #[test]
    fn separates_uncached_input_from_cached_tokens_like_9router() {
        let mut acc = UsageAccumulator::default();
        // OpenAI Chat / OpenCode gross prompt_tokens includes cached_tokens
        acc.observe_payload(&json!({
            "usage": {
                "prompt_tokens": 206_800,
                "completion_tokens": 1_137,
                "prompt_tokens_details": { "cached_tokens": 184_192 }
            }
        }));
        assert_eq!(acc.prompt_tokens, 22_608);
        assert_eq!(acc.cached_tokens, 184_192);
        assert_eq!(acc.completion_tokens, 1_137);
        assert_eq!(acc.total_tokens, 23_745);

        // Anthropic Messages already excludes cache_read_input_tokens from input_tokens
        let mut anth = UsageAccumulator::default();
        anth.observe_payload(&json!({
            "type": "message_start",
            "message": {
                "usage": {
                    "input_tokens": 4_200,
                    "cache_read_input_tokens": 180_000,
                    "cache_creation_input_tokens": 500
                }
            }
        }));
        anth.observe_payload(&json!({
            "type": "message_delta",
            "usage": { "output_tokens": 300 }
        }));
        assert_eq!(anth.prompt_tokens, 4_700);
        assert_eq!(anth.cached_tokens, 180_000);
        assert_eq!(anth.completion_tokens, 300);
        assert_eq!(anth.total_tokens, 5_000);

        let err = classify_sse_error_event(
            r#"{"type":"error","error":{"type":"FreeUsageLimitError","message":"Free usage limit reached"}}"#,
        );
        assert_eq!(err.as_ref().map(|e| e.0), Some(true));
    }
}
