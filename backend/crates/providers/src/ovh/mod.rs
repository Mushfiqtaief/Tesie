use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures::{Stream, StreamExt};
use rand::seq::SliceRandom;
use serde_json::{json, Value};
use tesie_core::{
    display_name_for_model, normalize_routing_strategy, ParsedModel,
    ProviderKind, Result, TesRouteError, UsageLogInput, WireFormat,
};
use tesie_proxy::{MihomoController, RelayRequest};
use tesie_store::{now_ms, Store};
use tracing::warn;

use crate::opencode::catalog::infer_model_capabilities;
use crate::opencode::executor::{
    ActiveRequestInfo, AttemptLeg, ProviderRequest, ProviderStreamResponse,
};
use crate::opencode::fingerprint::{
    apply_forced_thinking_level, apply_thinking_level, encode_opencode_body, ToolRenameMap,
};
use crate::translator::{translate_request, StreamTranslator};

pub const OVH_CHAT_URL: &str = "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/chat/completions";

pub struct OvhEngine {
    store: Store,
    mihomo: Arc<MihomoController>,
    rr_cursor: AtomicUsize,
    active_requests: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
    last_signal: Arc<Mutex<Option<(String, i64)>>>,
    last_error: Arc<Mutex<Option<(String, i64)>>>,
}

struct OvhStreamGuard {
    key: String,
    active: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
}

impl Drop for OvhStreamGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.active.lock() {
            g.remove(&self.key);
        }
    }
}

impl OvhEngine {
    pub fn new(store: Store, mihomo: Arc<MihomoController>) -> Self {
        Self {
            store,
            mihomo,
            rr_cursor: AtomicUsize::new(0),
            active_requests: Arc::new(Mutex::new(HashMap::new())),
            last_signal: Arc::new(Mutex::new(None)),
            last_error: Arc::new(Mutex::new(None)),
        }
    }

    pub fn active_requests_snapshot(&self) -> Vec<ActiveRequestInfo> {
        let now = now_ms();
        let mut g = self.active_requests.lock().unwrap();
        g.retain(|_, v| now.saturating_sub(v.started_at) < 120_000);
        g.values().cloned().collect()
    }

    pub fn recent_signals(&self) -> (Option<(String, i64)>, Option<(String, i64)>) {
        (
            self.last_signal.lock().unwrap().clone(),
            self.last_error.lock().unwrap().clone(),
        )
    }

    pub async fn execute_chat(
        &self,
        req: ProviderRequest,
        parsed: ParsedModel,
    ) -> Result<ProviderStreamResponse> {
        let now = now_ms();
        let cfg = self
            .store
            .provider_configs()
            .get_or_default("ovh")
            .await?;
        if !cfg.enabled {
            return Err(TesRouteError::BadRequest(
                "provider 'ovh' is currently disabled in Provider Settings".into(),
            ));
        }

        // Clean up model id: strip "ovh/" prefix if present
        let upstream_model_id = if let Some(stripped) = parsed.model_id.strip_prefix("ovh/") {
            stripped.to_string()
        } else {
            parsed.model_id.clone()
        };

        let target_format = WireFormat::OpenAiChat;
        let mut upstream_body = translate_request(
            req.source_format,
            target_format,
            &upstream_model_id,
            &req.body,
        );

        if let Some(obj) = upstream_body.as_object_mut() {
            obj.insert("stream".into(), Value::Bool(true));
            if !obj.contains_key("stream_options") {
                obj.insert("stream_options".into(), json!({ "include_usage": true }));
            }
        }

        let (_, model_has_reasoning, _, _) =
            infer_model_capabilities(&parsed.model_id, None, None, None, None);
        let has_reasoning = model_has_reasoning == Some(true);

        let reasoning_level = if let Some(ref eff) = parsed.reasoning_effort {
            apply_thinking_level(target_format, &mut upstream_body, eff, true);
            Some(eff.clone())
        } else if has_reasoning {
            apply_forced_thinking_level(target_format, &mut upstream_body, &cfg.thinking_level);
            (!cfg.thinking_level.eq_ignore_ascii_case("auto"))
                .then(|| cfg.thinking_level.clone())
        } else {
            None
        };

        let body_bytes = encode_opencode_body(&upstream_body, target_format)?;

        let mut candidates = self
            .store
            .quotas()
            .select_healthy_candidates_for_config(now, &cfg, |p| p.alive)
            .await?;

        let model_label = display_name_for_model(ProviderKind::Ovh, &parsed.model_id);
        if candidates.is_empty() {
            return Err(TesRouteError::ModelQuotaExhausted(format!(
                "No healthy proxy or available quota for {model_label} (OVHcloud limit: 2 requests/min per IP). All proxies currently cooling down.",
            )));
        }

        if candidates.len() > 1 {
            match normalize_routing_strategy(&cfg.routing_strategy, "round-robin").as_str() {
                "round-robin" => {
                    let off = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % candidates.len();
                    candidates.rotate_left(off);
                }
                "random" => {
                    candidates.shuffle(&mut rand::thread_rng());
                }
                _ => {}
            }
        }

        let wire_label = target_format.as_str().to_string();
        let mut attempt_legs: Vec<AttemptLeg> = Vec::new();
        let mut last_error: Option<String> = None;

        for candidate in candidates {
            let attempt_start = now_ms();
            let attempt_num = attempt_legs.len() + 1;
            let ip_key = format!("ovh:{}", candidate.proxy.ip_key());
            let egress_display = candidate
                .proxy
                .egress_ip
                .clone()
                .or_else(|| candidate.proxy.ip.clone())
                .or_else(|| Some(candidate.proxy.ip_key()));

            let headers = vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "text/event-stream".into()),
            ];

            let relay_resp = self
                .mihomo
                .relay(RelayRequest {
                    target_url: OVH_CHAT_URL.to_string(),
                    proxy_name: candidate.proxy.name.clone(),
                    method: "POST".to_string(),
                    fingerprint: "opencode".to_string(),
                    timeout_ms: 120_000,
                    upstream_authorization: None,
                    headers,
                    body: body_bytes.clone(),
                })
                .await;

            let resp = match relay_resp {
                Ok(r) => r,
                Err(err) => {
                    let msg = err.to_string();
                    let lat = (now_ms() - attempt_start).max(1);
                    warn!(
                        proxy = %candidate.proxy.name,
                        error = %msg,
                        "OVH relay transport error; trying next candidate"
                    );
                    attempt_legs.push(AttemptLeg {
                        attempt: attempt_num,
                        proxy: candidate.proxy.name.clone(),
                        egress_ip: egress_display.clone(),
                        status: 502,
                        latency_ms: lat,
                        phase: "relay_transport".into(),
                        model: Some(parsed.model_id.clone()),
                        wire_model: Some(upstream_model_id.clone()),
                        reasoning_level: reasoning_level.clone(),
                        cooldown_applied: Some("relay error".into()),
                        session_id: None,
                        detail: Some(msg.clone()),
                    });
                    last_error = Some(msg);
                    continue;
                }
            };

            let status_code = resp.status().as_u16();

            // Handle 429 Too Many Requests
            if status_code == 429 {
                let lat = (now_ms() - attempt_start).max(1);
                let retry_after_sec = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<i64>().ok())
                    .or_else(|| {
                        resp.headers()
                            .get("ratelimit-reset")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.trim().parse::<i64>().ok())
                    })
                    .unwrap_or(60)
                    .max(1);

                let cooldown_until = now_ms() + retry_after_sec * 1000;
                let _ = self
                    .store
                    .quotas()
                    .record_ovh_rate_limit(
                        &ip_key,
                        cooldown_until,
                        "Rate limit exceeded (429)",
                        now_ms(),
                    )
                    .await;

                warn!(
                    proxy = %candidate.proxy.name,
                    egress = ?egress_display,
                    retry_after_sec,
                    "OVH rate limit (429); cooling proxy and failing over"
                );
                attempt_legs.push(AttemptLeg {
                    attempt: attempt_num,
                    proxy: candidate.proxy.name.clone(),
                    egress_ip: egress_display.clone(),
                    status: 429,
                    latency_ms: lat,
                    phase: "rate_limit".into(),
                    model: Some(parsed.model_id.clone()),
                    wire_model: Some(upstream_model_id.clone()),
                    reasoning_level: reasoning_level.clone(),
                    cooldown_applied: Some(format!("cooldown {retry_after_sec}s")),
                    session_id: None,
                    detail: Some("Rate limit exceeded (429)".into()),
                });
                last_error = Some("Rate limit exceeded (429)".into());
                continue;
            }

            // Handle other HTTP errors (4xx / 5xx)
            if !resp.status().is_success() {
                let lat = (now_ms() - attempt_start).max(1);
                let err_bytes = resp.bytes().await.unwrap_or_default();
                let err_text = String::from_utf8_lossy(&err_bytes).to_string();
                warn!(
                    proxy = %candidate.proxy.name,
                    status = status_code,
                    body = %err_text,
                    "OVH upstream error response; failing over"
                );
                attempt_legs.push(AttemptLeg {
                    attempt: attempt_num,
                    proxy: candidate.proxy.name.clone(),
                    egress_ip: egress_display.clone(),
                    status: status_code,
                    latency_ms: lat,
                    phase: "upstream_error".into(),
                    model: Some(parsed.model_id.clone()),
                    wire_model: Some(upstream_model_id.clone()),
                    reasoning_level: reasoning_level.clone(),
                    cooldown_applied: None,
                    session_id: None,
                    detail: Some(err_text.chars().take(256).collect()),
                });
                last_error = Some(format!("HTTP {status_code}: {err_text}"));
                continue;
            }

            // Success (200 OK)
            // Parse rate limit headers
            let remaining_reqs = resp
                .headers()
                .get("ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());
            let reset_sec = resp
                .headers()
                .get("ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<i64>().ok());

            return Ok(self.wrap_success_stream(
                Box::pin(resp.bytes_stream()),
                req,
                parsed.model_id,
                target_format,
                wire_label,
                reasoning_level,
                candidate.proxy.name,
                egress_display,
                ip_key,
                attempt_legs,
                attempt_start,
                remaining_reqs,
                reset_sec,
            ));
        }

        // All candidates failed
        let err_summary = last_error.unwrap_or_else(|| "All candidate proxies exhausted".into());
        {
            if let Ok(mut g) = self.last_error.lock() {
                *g = Some(("ovh".into(), now_ms()));
            }
        }
        let _ = self
            .store
            .usage()
            .insert_log(UsageLogInput {
                request_id: req.request_id,
                api_key_id: req.api_key_id,
                provider_id: "ovh".into(),
                model: parsed.model_id,
                proxy_name: "none".into(),
                egress_ip: None,
                status_code: 502,
                prompt_tokens: 0,
                cached_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                latency_ms: (now_ms() - now).max(1),
                ttft_ms: None,
                tokens_per_sec: None,
                error_kind: Some("exhausted".into()),
                error_detail: Some(err_summary.clone()),
                upstream_url: Some(OVH_CHAT_URL.to_string()),
                attempt_count: attempt_legs.len() as i64,
                attempt_trace: serde_json::to_string(&attempt_legs).ok(),
                session_id: None,
                wire_format: Some(wire_label),
            })
            .await;

        Err(TesRouteError::ProxyRelay(format!(
            "OVH request failed after {} attempts: {err_summary}",
            attempt_legs.len()
        )))
    }

    #[allow(clippy::too_many_arguments)]
    fn wrap_success_stream<S>(
        &self,
        upstream: S,
        req: ProviderRequest,
        model_id: String,
        target_format: WireFormat,
        wire_label: String,
        reasoning_level: Option<String>,
        proxy_name: String,
        egress_ip: Option<String>,
        ip_key: String,
        prior_legs: Vec<AttemptLeg>,
        attempt_start: i64,
        remaining_reqs: Option<i64>,
        reset_sec: Option<i64>,
    ) -> ProviderStreamResponse
    where
        S: Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    {
        let store = self.store.clone();
        let source_format = req.source_format;
        let translator = StreamTranslator::new(
            source_format,
            target_format,
            model_id.clone(),
            ToolRenameMap::default(),
        );
        let provider_id = "ovh".to_string();

        {
            let mut active = self.active_requests.lock().unwrap();
            active.insert(
                req.request_id.clone(),
                ActiveRequestInfo {
                    request_id: req.request_id.clone(),
                    provider_id: provider_id.clone(),
                    model: model_id.clone(),
                    proxy_name: proxy_name.clone(),
                    egress_ip: egress_ip.clone(),
                    started_at: attempt_start,
                },
            );
        }
        let guard = OvhStreamGuard {
            key: req.request_id.clone(),
            active: self.active_requests.clone(),
        };
        let last_signal = self.last_signal.clone();

        struct StreamState<S> {
            upstream: S,
            translator: StreamTranslator,
            pending: std::collections::VecDeque<Bytes>,
            finished: bool,
            committed: bool,
            stream_error: Option<String>,
            store: Store,
            provider_id: String,
            model_id: String,
            proxy_name: String,
            egress_ip: Option<String>,
            ip_key: String,
            request_id: String,
            api_key_id: Option<String>,
            prior_legs: Vec<AttemptLeg>,
            reasoning_level: Option<String>,
            attempt_start: i64,
            first_token_at: Option<i64>,
            remaining_reqs: Option<i64>,
            reset_sec: Option<i64>,
            wire_label: String,
            _guard: OvhStreamGuard,
            last_signal: Arc<Mutex<Option<(String, i64)>>>,
        }

        let initial = StreamState {
            upstream,
            translator,
            pending: std::collections::VecDeque::new(),
            finished: false,
            committed: false,
            stream_error: None,
            store,
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
            proxy_name: proxy_name.clone(),
            egress_ip: egress_ip.clone(),
            ip_key,
            request_id: req.request_id,
            api_key_id: req.api_key_id,
            prior_legs,
            reasoning_level,
            attempt_start,
            first_token_at: None,
            remaining_reqs,
            reset_sec,
            wire_label,
            _guard: guard,
            last_signal,
        };

        let out_stream = futures::stream::unfold(initial, |mut st| async move {
            loop {
                if let Some(chunk) = st.pending.pop_front() {
                    return Some((Ok(chunk), st));
                }
                if st.finished {
                    if !st.committed {
                        st.committed = true;
                        st.translator.usage.finalize_fallback();
                        let usage = st.translator.usage;
                        let now = now_ms();
                        {
                            if let Ok(mut g) = st.last_signal.lock() {
                                *g = Some((st.provider_id.clone(), now));
                            }
                        }
                        let latency_ms = (now - st.attempt_start).max(1);
                        let ttft_ms = st.first_token_at.map(|t| (t - st.attempt_start).max(1));
                        let tps = if usage.completion_tokens > 0 {
                            let win = match st.first_token_at {
                                Some(t) if now - t >= 100 => now - t,
                                _ => latency_ms.max(50),
                            };
                            Some(
                                ((usage.completion_tokens as f64 / (win as f64 / 1000.0))
                                    * 100.0)
                                    .round()
                                    / 100.0,
                            )
                        } else {
                            None
                        };

                        let is_err = st.stream_error.is_some() || (usage.total_tokens == 0 && usage.emitted_chars == 0);
                        let final_status = if is_err { 502 } else { 200 };
                        let err_kind = if st.stream_error.is_some() {
                            Some("upstream_stream_drop".to_string())
                        } else if usage.total_tokens == 0 && usage.emitted_chars == 0 {
                            Some("empty_upstream_stream".to_string())
                        } else {
                            None
                        };
                        let err_detail = st.stream_error.clone().or_else(|| {
                            (usage.total_tokens == 0 && usage.emitted_chars == 0)
                                .then(|| "Upstream closed connection without returning tokens".to_string())
                        });

                        let mut legs = st.prior_legs.clone();
                        legs.push(AttemptLeg {
                            attempt: legs.len() + 1,
                            proxy: st.proxy_name.clone(),
                            egress_ip: st.egress_ip.clone(),
                            status: final_status,
                            latency_ms,
                            phase: if is_err { "stream_error".into() } else { "ok".into() },
                            model: Some(st.model_id.clone()),
                            wire_model: Some(st.model_id.clone()),
                            reasoning_level: st.reasoning_level.clone(),
                            cooldown_applied: if st.remaining_reqs == Some(0) {
                                Some(format!(
                                    "exhausted 60s window (reset {}s)",
                                    st.reset_sec.unwrap_or(60)
                                ))
                            } else {
                                None
                            },
                            session_id: None,
                            detail: err_detail.clone(),
                        });

                        if !is_err {
                            // Record success and remaining rate limits into database
                            let _ = st
                                .store
                                .quotas()
                                .record_ovh_success(
                                    &st.ip_key,
                                    usage.prompt_tokens,
                                    usage.completion_tokens,
                                    st.remaining_reqs,
                                    st.reset_sec,
                                    now,
                                )
                                .await;

                            if let Some(kid) = &st.api_key_id
                                && usage.total_tokens > 0
                            {
                                let _ = st.store.keys().add_usage(kid, usage.total_tokens, now).await;
                            }
                        }

                        let _ = st
                            .store
                            .usage()
                            .insert_log(UsageLogInput {
                                request_id: st.request_id.clone(),
                                api_key_id: st.api_key_id.clone(),
                                provider_id: st.provider_id.clone(),
                                model: st.model_id.clone(),
                                proxy_name: st.proxy_name.clone(),
                                egress_ip: st.egress_ip.clone(),
                                status_code: final_status as i64,
                                prompt_tokens: usage.prompt_tokens,
                                cached_tokens: usage.cached_tokens,
                                completion_tokens: usage.completion_tokens,
                                total_tokens: usage.total_tokens,
                                latency_ms,
                                ttft_ms,
                                tokens_per_sec: tps,
                                error_kind: err_kind,
                                error_detail: err_detail,
                                upstream_url: Some(OVH_CHAT_URL.to_string()),
                                attempt_count: legs.len() as i64,
                                attempt_trace: serde_json::to_string(&legs).ok(),
                                session_id: None,
                                wire_format: Some(st.wire_label.clone()),
                            })
                            .await;
                    }
                    return None;
                }

                match st.upstream.next().await {
                    Some(Ok(bytes)) => {
                        let frames = st.translator.push_bytes(&bytes);
                        if st.first_token_at.is_none()
                            && frames.iter().any(|f| f.as_ref() != b"data: [DONE]\n\n")
                        {
                            st.first_token_at = Some(now_ms());
                        }
                        for f in frames {
                            st.pending.push_back(f);
                        }
                    }
                    Some(Err(err)) => {
                        let msg = err.to_string();
                        st.finished = true;
                        st.stream_error = Some(msg.clone());
                        return Some((
                            Err(TesRouteError::ProxyRelay(format!(
                                "OVH upstream stream error: {msg}"
                            ))),
                            st,
                        ));
                    }
                    None => {
                        for f in st.translator.finish() {
                            st.pending.push_back(f);
                        }
                        st.finished = true;
                    }
                }
            }
        });

        ProviderStreamResponse {
            provider_id,
            model: model_id,
            proxy_name,
            egress_ip,
            source_format,
            stream: Box::pin(out_stream),
        }
    }
}
