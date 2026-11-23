use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use rand::seq::SliceRandom;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tesie_core::catalog::strip_thinking_suffix;
use tesie_core::{
    normalize_routing_strategy, CustomProvider, ProviderConfig, ProxyNode, Result, TesRouteError,
    UsageLogInput, WireFormat,
};
use tesie_proxy::{MihomoController, RelayRequest};
use tesie_store::{now_ms, Store};
use tracing::debug;
use url::Url;

use crate::opencode::executor::{ActiveRequestInfo, ProviderRequest, ProviderStreamResponse};
use crate::opencode::fingerprint::{
    apply_forced_thinking_level, apply_thinking_level, encode_opencode_body, ToolRenameMap,
};
use crate::translator::sse::{pre_commit_sse_response, SsePreCommit};
use crate::translator::{translate_request, StreamTranslator};

const MAX_HTML_BYTES: usize = 128 * 1024;
const MAX_ICON_BYTES: usize = 256 * 1024;

static LINK_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<link\b[^>]*>").unwrap());
static REL_ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\brel\s*=\s*["']([^"']+)["']|\brel\s*=\s*([^\s>]+)"#).unwrap());
static HREF_ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\bhref\s*=\s*["']([^"']+)["']|\bhref\s*=\s*([^\s>]+)"#).unwrap());
static TYPE_ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\btype\s*=\s*["']([^"']+)["']"#).unwrap());

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomAttemptLeg {
    attempt: usize,
    proxy: String,
    egress_ip: Option<String>,
    key_hint: Option<String>,
    status: u16,
    latency_ms: i64,
    phase: String,
    detail: Option<String>,
}

struct CustomStreamGuard {
    key: String,
    active: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
}

impl Drop for CustomStreamGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.active.lock() {
            g.remove(&self.key);
        }
    }
}

pub struct CustomProviderEngine {
    store: Store,
    mihomo: Arc<MihomoController>,
    rr_cursor: AtomicUsize,
    active_requests: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
    last_signal: Arc<Mutex<Option<(String, i64)>>>,
    last_error: Arc<Mutex<Option<(String, i64)>>>,
}

impl CustomProviderEngine {
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
        let now = now_ms();
        let last = self.last_signal.lock().unwrap().clone();
        let err = self
            .last_error
            .lock()
            .unwrap()
            .clone()
            .filter(|(_, ts)| now.saturating_sub(*ts) < 60_000);
        (last, err)
    }

    pub async fn resolve_target(
        &self,
        raw_model: &str,
    ) -> Result<Option<(CustomProvider, String, Option<String>)>> {
        let (without_effort, effort) = strip_thinking_suffix(raw_model);
        if let Some((prefix, rest)) = without_effort.split_once('/') {
            let model_id = rest.trim();
            if !model_id.is_empty()
                && let Some(cp) = self.store.custom_providers().get_by_prefix(prefix).await?
            {
                return Ok(Some((cp, model_id.to_string(), effort)));
            }
        }

        let needle = without_effort.trim();
        for cp in self.store.custom_providers().list_enabled().await? {
            if cp
                .models
                .iter()
                .any(|m| m.eq_ignore_ascii_case(needle))
            {
                return Ok(Some((cp, needle.to_string(), effort)));
            }
        }
        Ok(None)
    }

    pub async fn execute_chat(
        &self,
        req: ProviderRequest,
        cp: CustomProvider,
        model_id: String,
        effort: Option<String>,
    ) -> Result<ProviderStreamResponse> {
        if !cp.enabled {
            return Err(TesRouteError::BadRequest(format!(
                "custom provider '{}' is currently disabled",
                cp.name
            )));
        }

        let cfg = self
            .store
            .provider_configs()
            .get_or_default(&cp.id)
            .await?;
        if !cfg.enabled {
            return Err(TesRouteError::BadRequest(format!(
                "provider '{}' is currently disabled in Provider Settings",
                cp.name
            )));
        }

        let target_format = if cp.protocol.eq_ignore_ascii_case("anthropic") {
            WireFormat::AnthropicMessages
        } else if cp.api_type.eq_ignore_ascii_case("responses") {
            WireFormat::OpenAiResponses
        } else {
            WireFormat::OpenAiChat
        };

        let mut upstream_body = translate_request(req.source_format, target_format, &model_id, &req.body);
        if let Some(obj) = upstream_body.as_object_mut() {
            obj.insert("stream".into(), Value::Bool(true));
            if target_format == WireFormat::OpenAiChat && !obj.contains_key("stream_options") {
                obj.insert("stream_options".into(), json!({ "include_usage": true }));
            }
        }

        let reasoning_level = if let Some(ref eff) = effort {
            apply_thinking_level(target_format, &mut upstream_body, eff, true);
            Some(eff.clone())
        } else {
            apply_forced_thinking_level(target_format, &mut upstream_body, &cfg.thinking_level);
            (!cfg.thinking_level.eq_ignore_ascii_case("auto"))
                .then(|| cfg.thinking_level.clone())
        };

        let target_url = build_custom_endpoint_url(&cp.base_url, target_format)?;
        let body_bytes = encode_opencode_body(&upstream_body, target_format)?;

        let proxies = self
            .store
            .proxies()
            .list_for_provider_config(&cfg)
            .await?;
        let mihomo_states = self.mihomo.fetch_proxy_states().await.unwrap_or_default();
        let healthy_proxies: Vec<ProxyNode> = proxies
            .into_iter()
            .filter(|p| {
                mihomo_states
                    .get(&p.name)
                    .map(|s| s.alive)
                    .unwrap_or(p.is_direct)
            })
            .collect();

        if healthy_proxies.is_empty() {
            return Err(TesRouteError::ProxyRelay(format!(
                "No healthy proxy available for custom provider '{}'",
                cp.name
            )));
        }

        let keys: Vec<Option<String>> = if cp.api_keys.is_empty() {
            vec![None]
        } else {
            cp.api_keys.iter().cloned().map(Some).collect()
        };

        let mut legs_pool: Vec<(Option<String>, ProxyNode)> = Vec::with_capacity(keys.len() * healthy_proxies.len());
        for px in &healthy_proxies {
            for k in &keys {
                legs_pool.push((k.clone(), px.clone()));
            }
        }
        if legs_pool.len() > 1 {
            match normalize_routing_strategy(&cfg.routing_strategy, "fallback").as_str() {
                "round-robin" => {
                    let off = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % legs_pool.len();
                    legs_pool.rotate_left(off);
                }
                "random" => {
                    legs_pool.shuffle(&mut rand::thread_rng());
                }
                _ => {}
            }
        }

        let wire_label = target_format.as_str().to_string();
        let mut attempt_legs: Vec<CustomAttemptLeg> = Vec::new();
        let mut last_error: Option<String> = None;

        for (key_opt, proxy) in legs_pool {
            let attempt_start = now_ms();
            let attempt_num = attempt_legs.len() + 1;
            let key_hint = key_opt.as_deref().map(|k| {
                let prefix: String = k.chars().take(6).collect();
                format!("{prefix}***")
            });

            let mut headers = Vec::new();
            headers.push(("Content-Type".into(), "application/json".into()));
            if target_format == WireFormat::AnthropicMessages {
                headers.push(("anthropic-version".into(), "2023-06-01".into()));
                if let Some(k) = key_opt.as_deref() {
                    headers.push(("x-api-key".into(), k.to_string()));
                }
            }
            for (hk, hv) in &cp.custom_headers {
                if hk.eq_ignore_ascii_case("authorization")
                    || hk.eq_ignore_ascii_case("content-type")
                    || hk.eq_ignore_ascii_case("x-api-key")
                {
                    continue;
                }
                headers.push((hk.clone(), hv.clone()));
            }
            let upstream_auth = if let Some(k) = key_opt.as_deref()
                && target_format != WireFormat::AnthropicMessages
            {
                Some(format!("Bearer {k}"))
            } else {
                cp.custom_headers
                    .iter()
                    .find(|(hk, _)| hk.eq_ignore_ascii_case("authorization"))
                    .map(|(_, hv)| hv.clone())
            };

            let relay_resp = self
                .mihomo
                .relay(RelayRequest {
                    target_url: target_url.clone(),
                    proxy_name: proxy.name.clone(),
                    method: "POST".to_string(),
                    fingerprint: "standard".to_string(),
                    // Fast failover timeout: 15s instead of 120s so dead/stalled proxy nodes fail over quickly
                    timeout_ms: 15_000,
                    upstream_authorization: upstream_auth,
                    headers,
                    body: body_bytes.clone(),
                })
                .await;

            let resp = match relay_resp {
                Ok(r) => r,
                Err(err) => {
                    let msg = err.to_string();
                    attempt_legs.push(CustomAttemptLeg {
                        attempt: attempt_num,
                        proxy: proxy.name.clone(),
                        egress_ip: proxy.ip.clone(),
                        key_hint,
                        status: 502,
                        latency_ms: (now_ms() - attempt_start).max(0),
                        phase: "relay_transport".into(),
                        detail: Some(msg.chars().take(220).collect()),
                    });
                    last_error = Some(msg);
                    continue;
                }
            };

            let status = resp.status().as_u16();
            if status == 502
                && resp.headers().get("x-tesie-proxy-used").is_none()
                && resp.headers().get("x-tesroute-proxy-used").is_none()
            {
                let err_txt = resp.text().await.unwrap_or_default();
                attempt_legs.push(CustomAttemptLeg {
                    attempt: attempt_num,
                    proxy: proxy.name.clone(),
                    egress_ip: proxy.ip.clone(),
                    key_hint,
                    status: 502,
                    latency_ms: (now_ms() - attempt_start).max(0),
                    phase: "upstream_dial_failed".into(),
                    detail: Some(err_txt.chars().take(220).collect()),
                });
                last_error = Some(err_txt);
                continue;
            }

            if (200..300).contains(&status) {
                match pre_commit_sse_response(resp).await {
                    SsePreCommit::Ok { stream } => {
                        return Ok(self.wrap_success_stream(
                            stream,
                            req,
                            cp,
                            model_id,
                            target_format,
                            target_url,
                            wire_label,
                            reasoning_level,
                            proxy,
                            key_hint,
                            attempt_legs,
                            attempt_start,
                        ));
                    }
                    SsePreCommit::UpstreamError { raw_detail, .. } => {
                        attempt_legs.push(CustomAttemptLeg {
                            attempt: attempt_num,
                            proxy: proxy.name.clone(),
                            egress_ip: proxy.ip.clone(),
                            key_hint,
                            status: 502,
                            latency_ms: (now_ms() - attempt_start).max(0),
                            phase: "sse_upstream_error".into(),
                            detail: Some(raw_detail.chars().take(220).collect()),
                        });
                        last_error = Some(raw_detail);
                        continue;
                    }
                    SsePreCommit::TransportError(msg) => {
                        attempt_legs.push(CustomAttemptLeg {
                            attempt: attempt_num,
                            proxy: proxy.name.clone(),
                            egress_ip: proxy.ip.clone(),
                            key_hint,
                            status: 502,
                            latency_ms: (now_ms() - attempt_start).max(0),
                            phase: "sse_stream_transport".into(),
                            detail: Some(msg.chars().take(220).collect()),
                        });
                        last_error = Some(msg);
                        continue;
                    }
                }
            }

            let err_txt = resp.text().await.unwrap_or_default();
            let lat = (now_ms() - attempt_start).max(0);
            attempt_legs.push(CustomAttemptLeg {
                attempt: attempt_num,
                proxy: proxy.name.clone(),
                egress_ip: proxy.ip.clone(),
                key_hint,
                status,
                latency_ms: lat,
                phase: format!("http_{status}"),
                detail: Some(err_txt.chars().take(220).collect()),
            });
            let _ = self
                .store
                .usage()
                .insert_log(UsageLogInput {
                    request_id: req.request_id.clone(),
                    api_key_id: req.api_key_id.clone(),
                    provider_id: cp.id.clone(),
                    model: model_id.clone(),
                    proxy_name: proxy.name.clone(),
                    egress_ip: proxy.ip.clone(),
                    status_code: status as i64,
                    prompt_tokens: 0,
                    cached_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    latency_ms: lat,
                    ttft_ms: None,
                    tokens_per_sec: None,
                    error_kind: Some(format!("http_{status}")),
                    error_detail: Some(err_txt.chars().take(1500).collect()),
                    upstream_url: Some(target_url.clone()),
                    attempt_count: attempt_legs.len() as i64,
                    attempt_trace: serde_json::to_string(&attempt_legs).ok(),
                    session_id: None,
                    wire_format: Some(wire_label.clone()),
                })
                .await;
            *self.last_error.lock().unwrap() = Some((cp.id.clone(), now_ms()));
            last_error = Some(format!("HTTP {status}: {}", err_txt.chars().take(400).collect::<String>()));

            if tesie_core::is_context_length_error(status, &err_txt) {
                return Err(TesRouteError::ContextLengthExceeded(
                    tesie_core::extract_clean_context_message(&err_txt),
                ));
            }

            if matches!(status, 400 | 422) {
                return Err(TesRouteError::BadRequest(
                    tesie_core::extract_clean_context_message(&err_txt),
                ));
            }
        }

        Err(TesRouteError::ProxyRelay(last_error.unwrap_or_else(|| {
            format!("all keys/routes failed for custom provider '{}'", cp.name)
        })))
    }

    #[allow(clippy::too_many_arguments)]
    fn wrap_success_stream<S>(
        &self,
        upstream: S,
        req: ProviderRequest,
        cp: CustomProvider,
        model_id: String,
        target_format: WireFormat,
        target_url: String,
        wire_label: String,
        _reasoning_level: Option<String>,
        proxy: ProxyNode,
        key_hint: Option<String>,
        prior_legs: Vec<CustomAttemptLeg>,
        attempt_start: i64,
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
        let provider_id = cp.id.clone();
        let proxy_name = proxy.name.clone();
        let egress_ip = proxy.egress_ip.clone().or(proxy.ip.clone());

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
        let guard = CustomStreamGuard {
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
            store: Store,
            provider_id: String,
            model_id: String,
            proxy_name: String,
            egress_ip: Option<String>,
            request_id: String,
            api_key_id: Option<String>,
            target_url: String,
            wire_label: String,
            key_hint: Option<String>,
            prior_legs: Vec<CustomAttemptLeg>,
            attempt_start: i64,
            first_token_at: Option<i64>,
            _guard: CustomStreamGuard,
            last_signal: Arc<Mutex<Option<(String, i64)>>>,
        }

        let initial = StreamState {
            upstream,
            translator,
            pending: std::collections::VecDeque::new(),
            finished: false,
            committed: false,
            store,
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
            proxy_name: proxy_name.clone(),
            egress_ip: egress_ip.clone(),
            request_id: req.request_id,
            api_key_id: req.api_key_id,
            target_url,
            wire_label,
            key_hint,
            prior_legs,
            attempt_start,
            first_token_at: None,
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
                            Some(((usage.completion_tokens as f64 / (win as f64 / 1000.0)) * 100.0).round() / 100.0)
                        } else {
                            None
                        };

                        let mut legs = st.prior_legs.clone();
                        legs.push(CustomAttemptLeg {
                            attempt: legs.len() + 1,
                            proxy: st.proxy_name.clone(),
                            egress_ip: st.egress_ip.clone(),
                            key_hint: st.key_hint.clone(),
                            status: 200,
                            latency_ms,
                            phase: "ok".into(),
                            detail: None,
                        });
                        if let Some(kid) = &st.api_key_id
                            && usage.total_tokens > 0
                        {
                            let _ = st.store.keys().add_usage(kid, usage.total_tokens, now).await;
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
                                status_code: 200,
                                prompt_tokens: usage.prompt_tokens,
                                cached_tokens: usage.cached_tokens,
                                completion_tokens: usage.completion_tokens,
                                total_tokens: usage.total_tokens,
                                latency_ms,
                                ttft_ms,
                                tokens_per_sec: tps,
                                error_kind: None,
                                error_detail: None,
                                upstream_url: Some(st.target_url.clone()),
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
                        st.finished = true;
                        return Some((
                            Err(TesRouteError::ProxyRelay(format!(
                                "custom provider upstream stream error: {err}"
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

pub fn build_custom_endpoint_url(base_url: &str, target: WireFormat) -> Result<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed)
        .map_err(|e| TesRouteError::BadRequest(format!("invalid custom base_url: {e}")))?;
    let path = parsed.path().trim_end_matches('/');

    let url = match target {
        WireFormat::AnthropicMessages => {
            if path.ends_with("/messages") {
                trimmed.to_string()
            } else if path.is_empty() {
                format!("{trimmed}/v1/messages")
            } else {
                format!("{trimmed}/messages")
            }
        }
        WireFormat::OpenAiResponses => {
            if path.ends_with("/responses") {
                trimmed.to_string()
            } else if path.is_empty() {
                format!("{trimmed}/v1/responses")
            } else {
                format!("{trimmed}/responses")
            }
        }
        WireFormat::OpenAiChat => {
            if path.ends_with("/chat/completions") {
                trimmed.to_string()
            } else if path.ends_with("/completions") {
                trimmed.to_string()
            } else if path.is_empty() {
                format!("{trimmed}/v1/chat/completions")
            } else {
                format!("{trimmed}/chat/completions")
            }
        }
    };
    Ok(url)
}

pub fn build_custom_models_url(base_url: &str) -> Result<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed = Url::parse(trimmed)
        .map_err(|e| TesRouteError::BadRequest(format!("invalid custom base_url: {e}")))?;
    let mut base = trimmed.to_string();
    for suffix in ["/chat/completions", "/responses", "/messages"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.trim_end_matches('/').to_string();
            break;
        }
    }
    let path = parsed.path().trim_end_matches('/');
    if path.ends_with("/models") {
        Ok(base)
    } else if path.is_empty() {
        Ok(format!("{base}/v1/models"))
    } else {
        Ok(format!("{base}/models"))
    }
}

pub async fn fetch_custom_provider_models(
    cp: &CustomProvider,
    mihomo: &MihomoController,
    cfg: &ProviderConfig,
    store: &Store,
) -> Result<Vec<String>> {
    let url = build_custom_models_url(&cp.base_url)?;
    let proxies = store.proxies().list_for_provider_config(cfg).await?;
    let mihomo_states = mihomo.fetch_proxy_states().await.unwrap_or_default();
    let proxy_name = proxies
        .into_iter()
        .find(|p| mihomo_states.get(&p.name).map(|s| s.alive).unwrap_or(p.is_direct))
        .map(|p| p.name)
        .unwrap_or_else(|| "DIRECT".to_string());

    let key = cp.api_keys.first().map(String::as_str);
    let mut headers = vec![("Accept".into(), "application/json".into())];
    if cp.protocol.eq_ignore_ascii_case("anthropic") {
        headers.push(("anthropic-version".into(), "2023-06-01".into()));
        if let Some(k) = key {
            headers.push(("x-api-key".into(), k.to_string()));
        }
    }
    for (hk, hv) in &cp.custom_headers {
        if !hk.eq_ignore_ascii_case("authorization") && !hk.eq_ignore_ascii_case("x-api-key") {
            headers.push((hk.clone(), hv.clone()));
        }
    }
    let auth = key
        .filter(|_| !cp.protocol.eq_ignore_ascii_case("anthropic"))
        .map(|k| format!("Bearer {k}"));

    let resp = mihomo
        .relay(RelayRequest {
            target_url: url,
            proxy_name,
            method: "GET".to_string(),
            fingerprint: "standard".to_string(),
            timeout_ms: 12_000,
            upstream_authorization: auth,
            headers,
            body: Bytes::new(),
        })
        .await?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(TesRouteError::ProxyRelay(format!(
            "upstream /models returned HTTP {status}: {}",
            text.chars().take(280).collect::<String>()
        )));
    }

    let val: Value = serde_json::from_str(&text)
        .map_err(|e| TesRouteError::ProxyRelay(format!("invalid /models JSON response: {e}")))?;

    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for arr_key in ["data", "models"] {
        if let Some(arr) = val.get(arr_key).and_then(Value::as_array) {
            for item in arr {
                let raw_id = item
                    .get("id")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("");
                let clean = raw_id.strip_prefix("models/").unwrap_or(raw_id);
                if !clean.is_empty() && seen.insert(clean.to_string()) {
                    models.push(clean.to_string());
                }
            }
        }
    }

    Ok(models)
}

pub fn extract_favicon_urls_from_html(html: &str, base: &Url) -> Vec<String> {
    let mut scored: Vec<(u8, String)> = Vec::new();
    for mat in LINK_TAG_RE.find_iter(html) {
        let tag = mat.as_str();
        let rel = REL_ATTR_RE
            .captures(tag)
            .and_then(|c| c.get(1).or_else(|| c.get(2)))
            .map(|m| m.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        if !rel.contains("icon") {
            continue;
        }
        let href = HREF_ATTR_RE
            .captures(tag)
            .and_then(|c| c.get(1).or_else(|| c.get(2)))
            .map(|m| m.as_str().trim())
            .unwrap_or("");
        if href.is_empty() {
            continue;
        }
        let type_attr = TYPE_ATTR_RE
            .captures(tag)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        let href_lower = href.to_ascii_lowercase();
        let rank = if type_attr.contains("svg") || href_lower.contains(".svg") {
            0
        } else if rel == "icon" && (type_attr.contains("png") || href_lower.contains(".png")) {
            1
        } else if rel.contains("apple-touch-icon") {
            2
        } else if type_attr.contains("png") || href_lower.contains(".png") {
            3
        } else {
            4
        };

        if href_lower.starts_with("data:image/") {
            scored.push((rank, href.to_string()));
        } else if let Ok(joined) = base.join(href)
            && matches!(joined.scheme(), "http" | "https")
        {
            scored.push((rank, joined.to_string()));
        }
    }

    scored.sort_by_key(|(rank, _)| *rank);
    let mut seen = HashSet::new();
    scored
        .into_iter()
        .filter_map(|(_, u)| seen.insert(u.clone()).then_some(u))
        .collect()
}

fn is_disallowed_favicon_host(host: &str) -> bool {
    let h = host.trim().trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    if h.is_empty() || h == "localhost" || h.ends_with(".local") || h.ends_with(".internal") {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    || v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 64
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
            }
        };
    }
    false
}

fn detect_image_mime(bytes: &[u8], content_type: Option<&str>) -> Option<&'static str> {
    if bytes.is_empty() || bytes.len() > MAX_ICON_BYTES {
        return None;
    }
    let trimmed_start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &bytes[i..])
        .unwrap_or(bytes);

    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) || bytes.starts_with(&[0x00, 0x00, 0x02, 0x00]) {
        return Some("image/x-icon");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if (trimmed_start.starts_with(b"<svg") || trimmed_start.starts_with(b"<?xml"))
        && trimmed_start
            .windows(4)
            .any(|w| w.eq_ignore_ascii_case(b"<svg"))
    {
        return Some("image/svg+xml");
    }
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    if ct.starts_with("image/svg") {
        return Some("image/svg+xml");
    }
    if ct.starts_with("image/png") {
        return Some("image/png");
    }
    None
}

async fn fetch_image_as_data_uri(client: &reqwest::Client, target: &str) -> Option<String> {
    if target.starts_with("data:image/") {
        return Some(target.to_string());
    }
    let parsed = Url::parse(target).ok()?;
    let host = parsed.host_str()?;
    if is_disallowed_favicon_host(host) {
        return None;
    }
    let resp = client
        .get(target)
        .header("Accept", "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if resp.content_length().is_some_and(|l| l > MAX_ICON_BYTES as u64) {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    let mime = detect_image_mime(&bytes, ct.as_deref())?;
    let encoded = STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

fn candidate_origins_for(parsed: &Url) -> Vec<Url> {
    let mut origins = Vec::new();
    let Some(host) = parsed.host_str() else {
        return origins;
    };
    let port_part = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    if let Ok(u) = Url::parse(&format!("{}://{}{}/", parsed.scheme(), host, port_part)) {
        origins.push(u);
    }
    for sub in ["api.", "llm.", "gateway.", "open.", "proxy.", "chat.", "ai."] {
        if let Some(root_host) = host.strip_prefix(sub)
            && root_host.contains('.')
            && let Ok(u) = Url::parse(&format!("https://{root_host}/"))
        {
            origins.push(u);
        }
    }
    origins
}

/// Discovers a website's favicon directly from its homepage HTML (`<link rel="icon">`,
/// `/favicon.svg`, `/favicon.png`, `/favicon.ico`) and returns an inline `data:image/...` URI
/// so it works reliably offline and on desktop UIs without third-party runtime dependencies.
pub async fn discover_favicon_data_uri(base_url: &str) -> Option<String> {
    let parsed = Url::parse(base_url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    if is_disallowed_favicon_host(host) {
        return None;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .ok()?;

    let origins = candidate_origins_for(&parsed);
    let mut icon_candidates: Vec<String> = Vec::new();

    for origin in &origins {
        if let Ok(resp) = client
            .get(origin.as_str())
            .header("Accept", "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8")
            .send()
            .await
            && resp.status().is_success()
            && let Ok(bytes) = resp.bytes().await
        {
            let slice = &bytes[..bytes.len().min(MAX_HTML_BYTES)];
            let html = String::from_utf8_lossy(slice);
            for found in extract_favicon_urls_from_html(&html, origin) {
                if !icon_candidates.contains(&found) {
                    icon_candidates.push(found);
                }
            }
        }
        for path in ["favicon.svg", "favicon.png", "favicon.ico", "apple-touch-icon.png"] {
            if let Ok(u) = origin.join(path) {
                let s = u.to_string();
                if !icon_candidates.contains(&s) {
                    icon_candidates.push(s);
                }
            }
        }
    }

    for candidate in icon_candidates.into_iter().take(10) {
        debug!(favicon_candidate = %candidate, "probing site favicon");
        if let Some(data_uri) = fetch_image_as_data_uri(&client, &candidate).await {
            return Some(data_uri);
        }
    }

    // Fallback: Google S2 favicon resolver (covers Cloudflare-protected origins)
    let root_host = origins
        .last()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| host.to_string());
    let s2_url = format!("https://www.google.com/s2/favicons?domain={root_host}&sz=64");
    fetch_image_as_data_uri(&client, &s2_url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_openai_and_anthropic_endpoints_cleanly() {
        assert_eq!(
            build_custom_endpoint_url("https://logfare.ai/v1", WireFormat::OpenAiChat).unwrap(),
            "https://logfare.ai/v1/chat/completions"
        );
        assert_eq!(
            build_custom_endpoint_url("https://logfare.ai/v1/", WireFormat::AnthropicMessages).unwrap(),
            "https://logfare.ai/v1/messages"
        );
        assert_eq!(
            build_custom_endpoint_url("https://example.com", WireFormat::OpenAiResponses).unwrap(),
            "https://example.com/v1/responses"
        );
        assert_eq!(
            build_custom_models_url("https://logfare.ai/v1").unwrap(),
            "https://logfare.ai/v1/models"
        );
    }

    #[test]
    fn ranks_svg_and_png_favicons_over_ico_from_html() {
        let base = Url::parse("https://logfare.ai/").unwrap();
        let html = r#"
            <html><head>
              <link rel="shortcut icon" href="/favicon.ico">
              <link rel="apple-touch-icon" href="/apple-touch-icon.png">
              <link rel="icon" type="image/svg+xml" href="/brand/mark.svg">
              <link rel="icon" type="image/png" href="https://cdn.logfare.ai/icon-64.png">
            </head></html>
        "#;
        let urls = extract_favicon_urls_from_html(html, &base);
        assert_eq!(urls.len(), 4);
        assert_eq!(urls[0], "https://logfare.ai/brand/mark.svg");
        assert_eq!(urls[1], "https://cdn.logfare.ai/icon-64.png");
    }
}
