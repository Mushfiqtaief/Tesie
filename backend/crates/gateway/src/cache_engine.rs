use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use futures::{stream, Stream};
use hex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tesie_core::{GatewaySettings, ResponseCacheRecord, WireFormat};

/// In-Memory L1 LRU Cache for sub-millisecond hot hits
#[derive(Clone)]
pub struct MemoryCache {
    inner: Arc<RwLock<MemoryCacheInner>>,
}

struct MemoryCacheInner {
    entries: HashMap<String, ResponseCacheRecord>,
    order: VecDeque<String>,
    max_entries: usize,
}

impl Default for MemoryCache {
    fn default() -> Self {
        Self::new(500)
    }
}

impl MemoryCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryCacheInner {
                entries: HashMap::new(),
                order: VecDeque::new(),
                max_entries,
            })),
        }
    }

    pub fn get(&self, key: &str, now_ms: i64) -> Option<ResponseCacheRecord> {
        let mut guard = self.inner.write().ok()?;
        if let Some(record) = guard.entries.get(key) {
            if record.expires_at <= now_ms {
                guard.entries.remove(key);
                return None;
            }
            let rec = record.clone();
            // Move to back of LRU queue
            if let Some(pos) = guard.order.iter().position(|k| k == key) {
                guard.order.remove(pos);
            }
            guard.order.push_back(key.to_string());
            Some(rec)
        } else {
            None
        }
    }

    pub fn put(&self, record: ResponseCacheRecord) {
        if let Ok(mut guard) = self.inner.write() {
            let key = record.cache_key.clone();
            if guard.entries.contains_key(&key) {
                guard.entries.insert(key.clone(), record);
                if let Some(pos) = guard.order.iter().position(|k| k == &key) {
                    guard.order.remove(pos);
                }
                guard.order.push_back(key);
            } else {
                if guard.entries.len() >= guard.max_entries {
                    if let Some(oldest) = guard.order.pop_front() {
                        guard.entries.remove(&oldest);
                    }
                }
                guard.order.push_back(key.clone());
                guard.entries.insert(key, record);
            }
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            guard.entries.clear();
            guard.order.clear();
        }
    }

    pub fn count(&self) -> usize {
        self.inner.read().map(|g| g.entries.len()).unwrap_or(0)
    }
}

/// Checks if a client request is eligible for caching based on headers, temperature, and settings.
pub fn is_request_cacheable(body: &Value, headers: &axum::http::HeaderMap, settings: &GatewaySettings) -> bool {
    if !settings.response_cache_enabled {
        return false;
    }

    // Check client bypass headers
    if let Some(cc) = headers.get("cache-control").and_then(|v| v.to_str().ok()) {
        if cc.contains("no-cache") || cc.contains("no-store") {
            return false;
        }
    }
    if let Some(bypass) = headers.get("x-tesie-no-cache").and_then(|v| v.to_str().ok()) {
        if bypass.eq_ignore_ascii_case("true") || bypass == "1" {
            return false;
        }
    }

    // Check temperature: default allows temperature <= 0.0 or unspecified
    if let Some(temp) = body.get("temperature").and_then(Value::as_f64) {
        if temp > 0.0 {
            return false;
        }
    }

    true
}

/// Generates a canonical SHA-256 cache key from request parameters
pub fn compute_cache_key(model: &str, body: &Value, source_format: WireFormat) -> Option<String> {
    let clean_model = model.trim().to_ascii_lowercase();
    let messages = body.get("messages")?;

    // Optional canonical fields
    let tools = body.get("tools").unwrap_or(&Value::Null);
    let tool_choice = body.get("tool_choice").unwrap_or(&Value::Null);
    let system = body.get("system").unwrap_or(&Value::Null);

    let canonical = json!({
        "fmt": format!("{:?}", source_format),
        "model": clean_model,
        "messages": messages,
        "system": system,
        "tools": tools,
        "tool_choice": tool_choice,
    });

    let canonical_str = serde_json::to_string(&canonical).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(canonical_str.as_bytes());
    Some(hex::encode(hasher.finalize()))
}

/// Extracts prompt, completion, and total tokens from an aggregated completion payload
pub fn extract_usage_tokens(payload: &Value) -> (i64, i64, i64) {
    if let Some(usage) = payload.get("usage") {
        let prompt = usage.get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let completion = usage.get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let total = usage.get("total_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(prompt + completion);
        (prompt, completion, total)
    } else {
        (0, 0, 0)
    }
}

/// Replays a cached response as an SSE stream for clients requesting `stream: true`
pub fn replay_cached_as_sse_stream(
    source_format: WireFormat,
    model: &str,
    payload: &Value,
) -> impl Stream<Item = std::io::Result<Bytes>> + Send + 'static {
    let mut chunks: Vec<Bytes> = Vec::new();

    match source_format {
        WireFormat::OpenAiChat | WireFormat::OpenAiResponses => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("chatcmpl-cached");
            let created = payload.get("created").and_then(Value::as_i64).unwrap_or(1700000000);

            // Extract content and reasoning
            let (content, reasoning, tool_calls) = if let Some(choice) = payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
            {
                let msg = choice.get("message").unwrap_or(&Value::Null);
                let c = msg.get("content").and_then(Value::as_str).unwrap_or("").to_string();
                let r = msg.get("reasoning_content")
                    .or_else(|| msg.get("reasoning"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let tc = msg.get("tool_calls").cloned();
                (c, r, tc)
            } else {
                (String::new(), None, None)
            };

            // 1. Initial role chunk
            let initial_chunk = json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant" },
                    "finish_reason": null
                }]
            });
            chunks.push(Bytes::from(format!("data: {}\n\n", initial_chunk)));

            // 2. Reasoning delta if present
            if let Some(r) = reasoning {
                if !r.is_empty() {
                    let r_chunk = json!({
                        "id": id,
                        "object": "chat.completion.chunk",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "delta": { "reasoning_content": r },
                            "finish_reason": null
                        }]
                    });
                    chunks.push(Bytes::from(format!("data: {}\n\n", r_chunk)));
                }
            }

            // 3. Content delta if present
            if !content.is_empty() {
                let c_chunk = json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": { "content": content },
                        "finish_reason": null
                    }]
                });
                chunks.push(Bytes::from(format!("data: {}\n\n", c_chunk)));
            }

            // 4. Tool calls if present
            if let Some(tc) = tool_calls {
                let tc_chunk = json!({
                    "id": id,
                    "object": "chat.completion.chunk",
                    "created": created,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "delta": { "tool_calls": tc },
                        "finish_reason": null
                    }]
                });
                chunks.push(Bytes::from(format!("data: {}\n\n", tc_chunk)));
            }

            // 5. Final finish chunk
            let finish_chunk = json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]
            });
            chunks.push(Bytes::from(format!("data: {}\n\n", finish_chunk)));
            chunks.push(Bytes::from("data: [DONE]\n\n"));
        }
        WireFormat::AnthropicMessages => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("msg_cached");
            let content = if let Some(arr) = payload.get("content").and_then(Value::as_array) {
                arr.iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                String::new()
            };

            let usage = payload.get("usage").cloned().unwrap_or(json!({
                "input_tokens": 10,
                "output_tokens": 10
            }));

            // message_start
            let msg_start = json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": model,
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": usage
                }
            });
            chunks.push(Bytes::from(format!("event: message_start\ndata: {}\n\n", msg_start)));

            // content_block_start
            let block_start = json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            });
            chunks.push(Bytes::from(format!("event: content_block_start\ndata: {}\n\n", block_start)));

            // content_block_delta
            if !content.is_empty() {
                let block_delta = json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "text_delta", "text": content }
                });
                chunks.push(Bytes::from(format!("event: content_block_delta\ndata: {}\n\n", block_delta)));
            }

            // content_block_stop
            let block_stop = json!({
                "type": "content_block_stop",
                "index": 0
            });
            chunks.push(Bytes::from(format!("event: content_block_stop\ndata: {}\n\n", block_stop)));

            // message_delta
            let msg_delta = json!({
                "type": "message_delta",
                "delta": { "stop_reason": "end_turn", "stop_sequence": null },
                "usage": usage
            });
            chunks.push(Bytes::from(format!("event: message_delta\ndata: {}\n\n", msg_delta)));

            // message_stop
            chunks.push(Bytes::from("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"));
        }
    }

    stream::iter(chunks.into_iter().map(Ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use futures::StreamExt;

    #[test]
    fn test_compute_cache_key_deterministic_and_variant() {
        let body1 = json!({
            "model": "deepseek-v4.1-flash",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.0
        });
        let body2 = json!({
            "model": "deepseek-v4.1-flash",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.0
        });
        let body3 = json!({
            "model": "deepseek-v4.1-flash",
            "messages": [{"role": "user", "content": "different"}],
            "temperature": 0.0
        });

        let key1 = compute_cache_key("deepseek-v4.1-flash", &body1, WireFormat::OpenAiChat).unwrap();
        let key2 = compute_cache_key("deepseek-v4.1-flash", &body2, WireFormat::OpenAiChat).unwrap();
        let key3 = compute_cache_key("deepseek-v4.1-flash", &body3, WireFormat::OpenAiChat).unwrap();

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_is_request_cacheable_rules() {
        let settings = GatewaySettings::default();
        let mut headers = HeaderMap::new();

        let deterministic_body = json!({
            "messages": [{"role": "user", "content": "test"}],
            "temperature": 0.0
        });
        assert!(is_request_cacheable(&deterministic_body, &headers, &settings));

        let random_body = json!({
            "messages": [{"role": "user", "content": "test"}],
            "temperature": 0.7
        });
        assert!(!is_request_cacheable(&random_body, &headers, &settings));

        headers.insert("cache-control", "no-cache".parse().unwrap());
        assert!(!is_request_cacheable(&deterministic_body, &headers, &settings));
    }

    #[test]
    fn test_memory_cache_lru_and_expiration() {
        let cache = MemoryCache::new(2);
        let now = 100000;

        let r1 = ResponseCacheRecord {
            cache_key: "k1".into(),
            model: "m1".into(),
            source_format: "openai-chat".into(),
            response_payload: "{}".into(),
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
            created_at: now,
            expires_at: now + 5000,
            hit_count: 0,
            last_hit_at: None,
        };
        let r2 = ResponseCacheRecord {
            cache_key: "k2".into(),
            model: "m1".into(),
            source_format: "openai-chat".into(),
            response_payload: "{}".into(),
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
            created_at: now,
            expires_at: now + 5000,
            hit_count: 0,
            last_hit_at: None,
        };
        let r3 = ResponseCacheRecord {
            cache_key: "k3".into(),
            model: "m1".into(),
            source_format: "openai-chat".into(),
            response_payload: "{}".into(),
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
            created_at: now,
            expires_at: now + 5000,
            hit_count: 0,
            last_hit_at: None,
        };

        cache.put(r1);
        cache.put(r2);
        assert_eq!(cache.count(), 2);

        // Access k1 to make k2 older
        assert!(cache.get("k1", now).is_some());

        // Put k3 -> k2 should be evicted
        cache.put(r3);
        assert_eq!(cache.count(), 2);
        assert!(cache.get("k2", now).is_none());
        assert!(cache.get("k1", now).is_some());
        assert!(cache.get("k3", now).is_some());

        // Check expiration
        assert!(cache.get("k1", now + 6000).is_none());
    }

    #[tokio::test]
    async fn test_replay_cached_as_sse_stream() {
        let payload = json!({
            "id": "chatcmpl-test",
            "created": 1234567,
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello world from cache!",
                    "reasoning_content": "Thinking about it"
                }
            }]
        });

        let mut stream = replay_cached_as_sse_stream(WireFormat::OpenAiChat, "test-model", &payload);
        let mut chunks = Vec::new();
        while let Some(item) = stream.next().await {
            let b = item.unwrap();
            chunks.push(String::from_utf8_lossy(&b).to_string());
        }

        let full_output = chunks.join("");
        assert!(full_output.contains("chatcmpl-test"));
        assert!(full_output.contains("Thinking about it"));
        assert!(full_output.contains("Hello world from cache!"));
        assert!(full_output.contains("[DONE]"));
    }
}
