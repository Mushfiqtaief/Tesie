use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::{stream, Stream, StreamExt};
use regex::Regex;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tesie_core::catalog::{
    display_name_for_model, parse_model_spec, resolve_provider_for_model, resolve_wire_target,
};
use tesie_core::{
    IpCandidate, IpQuota, IpQuotaStatus, ParsedModel, ProviderKind, ProxyNode, Result,
    TesRouteError, UsageLogInput, WireFormat,
};
use tesie_proxy::{IpPoolSelector, MihomoController, RelayRequest};
use tesie_store::{now_ms, Store};
use tracing::{debug, info, warn};

use crate::antigravity::AntigravityEngine;
use crate::custom::CustomProviderEngine;

use crate::opencode::fingerprint::{
    ToolRenameMap, apply_fingerprint_tools, apply_forced_thinking_level, apply_thinking_level,
    build_minimal_quota_probe_body, encode_opencode_body, is_effort_rejection,
    is_opencode_title_request, is_stale_reasoning_reference, normalize_opencode_request_for_lane,
    strip_forced_thinking_level, strip_stale_reasoning_inputs, synthesize_title_from_body,
};
use crate::opencode::session::{
    OPENCODE_CLIENT, OPENCODE_V2_UA, canonical_session_id, extract_child_session,
    extract_conversation_hint, extract_native_opencode_session, extract_parent_session,
    generate_session_id, has_valid_opencode_v2_ua, header_ci, is_valid_v2_project_id,
    observe_upstream_date_header, random_project_id, upstream_now_ms,
};
use crate::translator::sse::{
    is_upstream_quota_exhausted, pre_commit_sse_response, SsePreCommit,
};
use crate::translator::StreamTranslator;
use tokio::sync::Notify;

const MAX_CONV_SESSIONS: usize = 4096;
const MAX_OVERLOAD_RETRIES: usize = 3;
const OVERLOAD_BASE_MS: u64 = 800;
const NETWORK_ABUSE_COOLDOWN_MS: i64 = 30 * 60 * 1000;

/// Default automatic quota probe interval (30 seconds, configurable per provider).
pub const QUOTA_PROBE_INTERVAL_SEC: u64 = 30;
const QUOTA_PROBE_MODEL: &str = "deepseek-v4.1-flash-free";
const QUOTA_PROBE_MANUAL_CONCURRENCY: usize = 16;
const QUOTA_PROBE_AUTO_CONCURRENCY: usize = 3;
const QUOTA_PROBE_AUTO_BATCH_LIMIT: usize = 12;

static OVERLOAD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)overloaded").unwrap());
static RESET_AFTER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)reset(?:\s+after|[_-]after)?[:\s]+(\d+)\s*s").unwrap());

#[derive(Debug, Clone)]
pub struct OpenCodeConfig {
    pub free_base_url: String,
    pub zen_base_url: String,
    pub timeout_ms: u64,
}

impl Default for OpenCodeConfig {
    fn default() -> Self {
        let free_base = std::env::var("OPENCODE_FREE_BASE_URL")
            .unwrap_or_else(|_| "https://opencode.ai".to_string());
        let zen_base =
            std::env::var("OPENCODE_ZEN_BASE_URL").unwrap_or_else(|_| free_base.clone());
        Self {
            free_base_url: free_base,
            zen_base_url: zen_base,
            timeout_ms: 120_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub request_id: String,
    pub source_format: WireFormat,
    pub raw_model: String,
    pub body: Value,
    pub headers: Vec<(String, String)>,
    pub api_key_id: Option<String>,
}

pub struct ProviderStreamResponse {
    pub provider_id: String,
    pub model: String,
    pub proxy_name: String,
    pub egress_ip: Option<String>,
    pub source_format: WireFormat,
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveRequestInfo {
    pub request_id: String,
    pub provider_id: String,
    pub model: String,
    pub proxy_name: String,
    pub egress_ip: Option<String>,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptLeg {
    pub attempt: usize,
    pub proxy: String,
    pub egress_ip: Option<String>,
    pub status: u16,
    pub latency_ms: i64,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wire_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_applied: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaProbeStatus {
    pub enabled: bool,
    pub interval_sec: u64,
    pub probe_running: bool,
    pub last_probe_at: Option<i64>,
    pub last_checked: usize,
    pub last_ready: usize,
    pub last_cooling: usize,
    pub last_unhealthy: usize,
}

impl Default for QuotaProbeStatus {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_sec: QUOTA_PROBE_INTERVAL_SEC,
            probe_running: false,
            last_probe_at: None,
            last_checked: 0,
            last_ready: 0,
            last_cooling: 0,
            last_unhealthy: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaProbeSummary {
    pub checked: usize,
    pub ready: usize,
    pub cooling: usize,
    pub unhealthy: usize,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedQuotaResetResult {
    pub ip_key: String,
    pub reset: bool,
    pub available: bool,
    pub message: String,
    pub quota: Option<IpQuota>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedBatchResetResult {
    pub checked: usize,
    pub reset: usize,
    pub still_cooling: usize,
    pub unhealthy: usize,
}

struct ActiveStreamGuard {
    key: String,
    active_requests: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
}

impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.active_requests.lock() {
            guard.remove(&self.key);
        }
    }
}

pub struct OpenCodeEngine {
    store: Store,
    mihomo: Arc<MihomoController>,
    selector: Arc<IpPoolSelector>,
    antigravity: Arc<AntigravityEngine>,
    custom: Arc<CustomProviderEngine>,
    config: OpenCodeConfig,
    conv_sessions: Mutex<HashMap<String, (String, i64)>>,
    active_requests: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
    last_provider: Arc<Mutex<Option<(String, i64)>>>,
    last_error_provider: Arc<Mutex<Option<(String, i64)>>>,
    quota_probe_lock: tokio::sync::Mutex<()>,
    quota_probe_running: AtomicBool,
    quota_probe_meta: Mutex<QuotaProbeStatus>,
    probe_notify: Notify,
}

impl OpenCodeEngine {
    pub fn new(
        store: Store,
        mihomo: Arc<MihomoController>,
        selector: Arc<IpPoolSelector>,
        config: OpenCodeConfig,
    ) -> Self {
        let antigravity = Arc::new(AntigravityEngine::new(store.clone(), mihomo.clone()));
        let custom = Arc::new(CustomProviderEngine::new(store.clone(), mihomo.clone()));
        Self {
            store,
            mihomo,
            selector,
            antigravity,
            custom,
            config,
            conv_sessions: Mutex::new(HashMap::new()),
            active_requests: Arc::new(Mutex::new(HashMap::new())),
            last_provider: Arc::new(Mutex::new(None)),
            last_error_provider: Arc::new(Mutex::new(None)),
            quota_probe_lock: tokio::sync::Mutex::new(()),
            quota_probe_running: AtomicBool::new(false),
            quota_probe_meta: Mutex::new(QuotaProbeStatus::default()),
            probe_notify: Notify::new(),
        }
    }

    pub fn notify_probe_config_changed(&self) {
        self.probe_notify.notify_waiters();
        self.antigravity.notify_probe_config_changed();
    }

    pub fn antigravity(&self) -> &Arc<AntigravityEngine> {
        &self.antigravity
    }

    pub fn custom(&self) -> &Arc<CustomProviderEngine> {
        &self.custom
    }

    pub fn config(&self) -> &OpenCodeConfig {
        &self.config
    }

    pub fn selector(&self) -> &Arc<IpPoolSelector> {
        &self.selector
    }

    pub async fn quota_probe_status(&self) -> QuotaProbeStatus {
        let cfg = self
            .store
            .provider_configs()
            .get_or_default("opencode")
            .await
            .ok();
        let mut st = self
            .quota_probe_meta
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        st.enabled = cfg.as_ref().map(|c| c.probe_enabled).unwrap_or(true);
        st.interval_sec = cfg
            .as_ref()
            .map(|c| c.probe_interval_sec.clamp(5, 86_400) as u64)
            .unwrap_or(QUOTA_PROBE_INTERVAL_SEC);
        st.probe_running = self.quota_probe_running.load(Ordering::Relaxed);
        st
    }

    /// Starts the configurable background per-IP quota probe loop (`probe_enabled`, `probe_interval_sec`, default 30s)
    /// alongside Antigravity's configurable quota sync loop.
    pub fn start_auto_quota_check_loop(self: &Arc<Self>) {
        self.antigravity.start_auto_quota_sync_loop();
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            // Allow TesieCore's initial HTTPS pool latency check to settle first
            tokio::time::sleep(Duration::from_secs(4)).await;
            if let Ok(cfg) = engine
                .store
                .provider_configs()
                .get_or_default("opencode")
                .await
                && cfg.enabled
                && cfg.probe_enabled
                && let Err(err) = engine.probe_ip_quotas_live(false).await
            {
                warn!(error = %err, "initial OpenCode quota probe failed");
            }

            loop {
                let (enabled, interval_sec) = match engine
                    .store
                    .provider_configs()
                    .get_or_default("opencode")
                    .await
                {
                    Ok(cfg) => (
                        cfg.enabled && cfg.probe_enabled,
                        cfg.probe_interval_sec.clamp(5, 86_400) as u64,
                    ),
                    Err(_) => (true, QUOTA_PROBE_INTERVAL_SEC),
                };

                if !enabled {
                    engine.probe_notify.notified().await;
                    continue;
                }

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(interval_sec)) => {
                        if let Err(err) = engine.probe_ip_quotas_live(false).await {
                            warn!(error = %err, "scheduled OpenCode quota probe failed");
                        }
                    }
                    _ = engine.probe_notify.notified() => {
                        continue;
                    }
                }
            }
        });
    }

    /// Runs a live minimal-request OpenCode quota check across eligible healthy exit IPs.
    /// When `force_all` is `true` (manual "Check Quotas Now"), re-checks up to 48 healthy IPs;
    /// when `false` (automatic 30s tick), checks unchecked IPs, used-since-check IPs, and a
    /// rolling slice of oldest-checked active IPs.
    pub async fn probe_ip_quotas_live(&self, force_all: bool) -> Result<QuotaProbeSummary> {
        let _guard = self.quota_probe_lock.lock().await;
        self.quota_probe_running.store(true, Ordering::Relaxed);

        let result = self.probe_ip_quotas_inner(force_all).await;
        self.quota_probe_running.store(false, Ordering::Relaxed);

        if let Ok(summary) = &result
            && let Ok(mut meta) = self.quota_probe_meta.lock()
        {
            meta.last_probe_at = Some(summary.checked_at);
            meta.last_checked = summary.checked;
            meta.last_ready = summary.ready;
            meta.last_cooling = summary.cooling;
            meta.last_unhealthy = summary.unhealthy;
        }

        result
    }

    async fn probe_ip_quotas_inner(&self, force_all: bool) -> Result<QuotaProbeSummary> {
        let now = now_ms();
        self.store
            .quotas()
            .refresh_windows_and_cooldowns(now)
            .await?;

        let oc_cfg = self
            .store
            .provider_configs()
            .get_or_default("opencode")
            .await?;
        if !oc_cfg.enabled {
            return Ok(QuotaProbeSummary {
                checked: 0,
                ready: 0,
                cooling: 0,
                unhealthy: 0,
                checked_at: now,
            });
        }

        let settings = self.store.settings().get_settings().await?;
        let mihomo_states = self.mihomo.fetch_proxy_states().await?;
        let proxies = self
            .store
            .proxies()
            .list_for_provider_config(&oc_cfg)
            .await?;

        for node in &proxies {
            self.store
                .quotas()
                .ensure_ip_quota_row(&node.ip_key(), &settings, now)
                .await?;
        }

        let all_quotas = self.store.quotas().list_ip_quotas().await?;
        let quota_map: HashMap<String, _> = all_quotas
            .into_iter()
            .map(|q| (q.quota.ip_key.clone(), q.quota))
            .collect();

        let next_probe_ms = oc_cfg.probe_interval_sec.clamp(5, 86_400) * 1000;

        // Pick one healthy TesieCore proxy per unique exit IP
        let mut seen_ips = HashSet::new();
        let mut due_targets: Vec<(ProxyNode, String, bool, i64)> = Vec::new();

        for proxy in proxies {
            let is_alive = mihomo_states
                .get(&proxy.name)
                .map(|s| s.alive)
                .unwrap_or(proxy.is_direct);
            if !is_alive {
                continue;
            }
            let ip_key = proxy.ip_key();
            if !seen_ips.insert(ip_key.clone()) {
                continue;
            }
            let Some(q) = quota_map.get(&ip_key) else {
                continue;
            };
            let is_cooling = q.status == IpQuotaStatus::Cooldown;
            let cooldown_expired =
                is_cooling && q.cooldown_until.is_none_or(|until| until <= now);

            if force_all {
                let sort_ts = if is_cooling && !cooldown_expired {
                    q.last_quota_checked_at.unwrap_or(0) + 1_000_000_000
                } else {
                    q.last_quota_checked_at.unwrap_or(0)
                };
                due_targets.push((proxy, ip_key, is_cooling, sort_ts));
                continue;
            }

            // Background queue recovery: probe ONLY queued (`cooldown`) IPs whose N-second
            // re-check timer is due. Live (`active`) IPs are never touched so Ready quota is never burned.
            if is_cooling && cooldown_expired {
                due_targets.push((proxy, ip_key, true, q.last_quota_checked_at.unwrap_or(0)));
            }
        }

        due_targets.sort_by_key(|(_, _, _, ts)| *ts);

        let selected: Vec<(ProxyNode, String, bool)> = if force_all {
            due_targets
                .into_iter()
                .map(|(p, ip, queued, _)| (p, ip, queued))
                .collect()
        } else {
            due_targets
                .into_iter()
                .take(QUOTA_PROBE_AUTO_BATCH_LIMIT)
                .map(|(p, ip, queued, _)| (p, ip, queued))
                .collect()
        };

        if selected.is_empty() {
            return Ok(QuotaProbeSummary {
                checked: 0,
                ready: 0,
                cooling: 0,
                unhealthy: 0,
                checked_at: now,
            });
        }

        let probe_body = build_minimal_quota_probe_body(QUOTA_PROBE_MODEL)?;
        let target_url = self.build_upstream_url(WireFormat::OpenAiChat);
        let concurrency = if force_all {
            QUOTA_PROBE_MANUAL_CONCURRENCY
        } else {
            QUOTA_PROBE_AUTO_CONCURRENCY
        };

        let mut probe_stream = stream::iter(selected)
            .map(|(proxy, ip_key, is_already_queued)| {
                let body = probe_body.clone();
                let url = target_url.clone();
                async move {
                    self.probe_single_ip_quota(
                        proxy,
                        ip_key,
                        url,
                        body,
                        is_already_queued,
                        next_probe_ms,
                    )
                    .await
                }
            })
            .buffer_unordered(concurrency);

        let mut checked = 0usize;
        let mut ready = 0usize;
        let mut cooling = 0usize;
        let mut unhealthy = 0usize;

        while let Some(outcome) = probe_stream.next().await {
            checked += 1;
            match outcome {
                SingleProbeOutcome::Ready => ready += 1,
                SingleProbeOutcome::Cooling => cooling += 1,
                SingleProbeOutcome::Unhealthy => unhealthy += 1,
                SingleProbeOutcome::Inconclusive => {}
            }
        }

        let finished_at = now_ms();
        info!(
            checked,
            ready,
            cooling,
            unhealthy,
            force_all,
            "completed OpenCode minimal-request per-IP quota check"
        );

        Ok(QuotaProbeSummary {
            checked,
            ready,
            cooling,
            unhealthy,
            checked_at: finished_at,
        })
    }

    async fn probe_single_ip_quota(
        &self,
        proxy: ProxyNode,
        ip_key: String,
        target_url: String,
        probe_body: Bytes,
        is_already_queued: bool,
        next_probe_ms: i64,
    ) -> SingleProbeOutcome {
        // Fresh session & random 40-hex project ID per probe so probes never collide on a shared bucket
        let session = generate_session_id(Some(upstream_now_ms().max(0) as u64));
        let headers = self.build_free_headers(&[], &session, WireFormat::OpenAiChat);

        let relay_resp = self
            .mihomo
            .relay(RelayRequest {
                target_url,
                proxy_name: proxy.name.clone(),
                method: "POST".to_string(),
                fingerprint: "opencode".to_string(),
                timeout_ms: 14_000,
                upstream_authorization: Some("Bearer public".to_string()),
                headers,
                body: probe_body,
            })
            .await;

        let resp = match relay_resp {
            Ok(r) => r,
            Err(err) => {
                debug!(proxy = %proxy.name, error = %err, "quota probe relay transport failed; marking proxy idle");
                self.quarantine_broken_proxy(&proxy.id, &proxy.name).await;
                if is_already_queued {
                    let _ = self
                        .store
                        .quotas()
                        .reschedule_queued_probe(&ip_key, next_probe_ms, now_ms())
                        .await;
                }
                return SingleProbeOutcome::Unhealthy;
            }
        };

        if let Some(date_hdr) = resp.headers().get("date").and_then(|v| v.to_str().ok()) {
            observe_upstream_date_header(date_hdr);
        }

        let status = resp.status().as_u16();
        if status == 502
            && resp.headers().get("x-tesie-proxy-used").is_none()
            && resp.headers().get("x-tesroute-proxy-used").is_none()
        {
            self.quarantine_broken_proxy(&proxy.id, &proxy.name).await;
            if is_already_queued {
                let _ = self
                    .store
                    .quotas()
                    .reschedule_queued_probe(&ip_key, next_probe_ms, now_ms())
                    .await;
            }
            return SingleProbeOutcome::Unhealthy;
        }

        if (200..300).contains(&status) {
            let ts = now_ms();
            let _ = self
                .store
                .proxies()
                .reset_proxy_failures(&proxy.id, ts)
                .await;
            if is_already_queued {
                let _ = self.store.quotas().reset_ip_quota(&ip_key, ts).await;
            }
            let _ = self
                .store
                .quotas()
                .record_probe_available(&ip_key, ts)
                .await;
            // Drop stream immediately so minimal tokens/bandwidth are used
            drop(resp);
            return SingleProbeOutcome::Ready;
        }

        let err_body = read_limited_error(resp).await;
        if status == 429 || status == 403 {
            let is_abuse = status == 403 && is_network_abuse_block(&err_body);
            let cooldown_ms = if is_abuse {
                Some(NETWORK_ABUSE_COOLDOWN_MS)
            } else {
                Some(next_probe_ms)
            };
            let _ = self
                .store
                .quotas()
                .record_rate_limit(&ip_key, cooldown_ms, &err_body, now_ms())
                .await;
            return SingleProbeOutcome::Cooling;
        }

        if is_already_queued {
            let _ = self
                .store
                .quotas()
                .reschedule_queued_probe(&ip_key, next_probe_ms, now_ms())
                .await;
        }
        SingleProbeOutcome::Inconclusive
    }

    /// Verifies live upstream OpenCode availability for a single exit IP before clearing cooldown:
    /// - If the IP is NOT in cooldown (only resetting local window counters), resets immediately.
    /// - If the IP IS in cooldown, sends a live minimal OpenCode request first and ONLY resets
    ///   cooldown when upstream genuinely responds `200 OK` (usage is actually available).
    pub async fn verify_and_reset_ip_quota(&self, ip_key: &str) -> Result<VerifiedQuotaResetResult> {
        let now = now_ms();
        let existing = self
            .store
            .quotas()
            .get_ip_quota(ip_key)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("ip quota {ip_key} not found")))?;

        if existing.status != IpQuotaStatus::Cooldown {
            let reset_q = self.store.quotas().reset_ip_quota(ip_key, now).await?;
            let _ = self.store.quotas().record_probe_available(ip_key, now).await;
            return Ok(VerifiedQuotaResetResult {
                ip_key: ip_key.to_string(),
                reset: true,
                available: true,
                message: format!("Reset window counters for {ip_key}"),
                quota: Some(reset_q),
            });
        }

        let next_probe_ms = self
            .store
            .provider_configs()
            .get_or_default("opencode")
            .await
            .map(|c| c.probe_interval_sec.clamp(5, 86_400) * 1000)
            .unwrap_or((QUOTA_PROBE_INTERVAL_SEC as i64) * 1000);

        let proxies = self.store.proxies().list_proxies().await?;
        let mihomo_states = self.mihomo.fetch_proxy_states().await?;
        let candidate_proxy = proxies.into_iter().find(|p| {
            p.ip_key() == ip_key
                && mihomo_states
                    .get(&p.name)
                    .map(|s| s.alive)
                    .unwrap_or(p.is_direct)
        });

        let Some(proxy) = candidate_proxy else {
            return Ok(VerifiedQuotaResetResult {
                ip_key: ip_key.to_string(),
                reset: false,
                available: false,
                message: format!(
                    "Cannot reset {ip_key}: no healthy proxy in TesieCore currently exits via this IP"
                ),
                quota: Some(existing),
            });
        };

        let probe_body = build_minimal_quota_probe_body(QUOTA_PROBE_MODEL)?;
        let target_url = self.build_upstream_url(WireFormat::OpenAiChat);
        let outcome = self
            .probe_single_ip_quota(
                proxy,
                ip_key.to_string(),
                target_url,
                probe_body,
                true,
                next_probe_ms,
            )
            .await;

        match outcome {
            SingleProbeOutcome::Ready => {
                let updated = self
                    .store
                    .quotas()
                    .reset_ip_quota(ip_key, now_ms())
                    .await?;
                let _ = self
                    .store
                    .quotas()
                    .record_probe_available(ip_key, now_ms())
                    .await;
                let fresh = self.store.quotas().get_ip_quota(ip_key).await?;
                Ok(VerifiedQuotaResetResult {
                    ip_key: ip_key.to_string(),
                    reset: true,
                    available: true,
                    message: format!("Verified live: usage is available on {ip_key} — Ready"),
                    quota: fresh.or(Some(updated)),
                })
            }
            SingleProbeOutcome::Cooling => {
                let fresh = self.store.quotas().get_ip_quota(ip_key).await?;
                Ok(VerifiedQuotaResetResult {
                    ip_key: ip_key.to_string(),
                    reset: false,
                    available: false,
                    message: format!(
                        "{ip_key} is still rate-limited on OpenCode — kept in queue"
                    ),
                    quota: fresh,
                })
            }
            SingleProbeOutcome::Unhealthy => {
                let fresh = self.store.quotas().get_ip_quota(ip_key).await?;
                Ok(VerifiedQuotaResetResult {
                    ip_key: ip_key.to_string(),
                    reset: false,
                    available: false,
                    message: format!(
                        "Proxy for {ip_key} failed dial check — kept in queue"
                    ),
                    quota: fresh,
                })
            }
            SingleProbeOutcome::Inconclusive => {
                let fresh = self.store.quotas().get_ip_quota(ip_key).await?;
                Ok(VerifiedQuotaResetResult {
                    ip_key: ip_key.to_string(),
                    reset: false,
                    available: false,
                    message: format!(
                        "Could not confirm live quota availability for {ip_key} — kept in queue"
                    ),
                    quota: fresh,
                })
            }
        }
    }

    /// Probes all queued exit IPs (scoped to OpenCode provider proxy config) and promotes ONLY those where real upstream usage is available (`200 OK`).
    pub async fn verify_and_reset_all_cooling(&self) -> Result<VerifiedBatchResetResult> {
        let oc_cfg = self
            .store
            .provider_configs()
            .get_or_default("opencode")
            .await?;
        let next_probe_ms = oc_cfg.probe_interval_sec.clamp(5, 86_400) * 1000;
        let all_quotas = self.store.quotas().list_ip_quotas_for_config(&oc_cfg).await?;
        let cooling_ips: HashSet<String> = all_quotas
            .into_iter()
            .filter(|q| q.quota.status == IpQuotaStatus::Cooldown)
            .map(|q| q.quota.ip_key)
            .collect();

        if cooling_ips.is_empty() {
            return Ok(VerifiedBatchResetResult {
                checked: 0,
                reset: 0,
                still_cooling: 0,
                unhealthy: 0,
            });
        }

        let proxies = self.store.proxies().list_for_provider_config(&oc_cfg).await?;
        let mihomo_states = self.mihomo.fetch_proxy_states().await?;
        let mut seen = HashSet::new();
        let mut targets = Vec::new();
        for p in proxies {
            let alive = mihomo_states
                .get(&p.name)
                .map(|s| s.alive)
                .unwrap_or(p.is_direct);
            if !alive {
                continue;
            }
            let ip = p.ip_key();
            if cooling_ips.contains(&ip) && seen.insert(ip.clone()) {
                targets.push((p, ip));
            }
        }

        let probe_body = build_minimal_quota_probe_body(QUOTA_PROBE_MODEL)?;
        let target_url = self.build_upstream_url(WireFormat::OpenAiChat);

        let mut probe_stream = stream::iter(targets)
            .map(|(proxy, ip_key)| {
                let body = probe_body.clone();
                let url = target_url.clone();
                async move {
                    let out = self
                        .probe_single_ip_quota(proxy, ip_key.clone(), url, body, true, next_probe_ms)
                        .await;
                    (ip_key, out)
                }
            })
            .buffer_unordered(QUOTA_PROBE_MANUAL_CONCURRENCY);

        let mut checked = 0usize;
        let mut reset = 0usize;
        let mut still_cooling = 0usize;
        let mut unhealthy = 0usize;

        while let Some((ip_key, outcome)) = probe_stream.next().await {
            checked += 1;
            match outcome {
                SingleProbeOutcome::Ready => {
                    let ts = now_ms();
                    let _ = self.store.quotas().reset_ip_quota(&ip_key, ts).await;
                    let _ = self.store.quotas().record_probe_available(&ip_key, ts).await;
                    reset += 1;
                }
                SingleProbeOutcome::Cooling => still_cooling += 1,
                SingleProbeOutcome::Unhealthy => unhealthy += 1,
                SingleProbeOutcome::Inconclusive => still_cooling += 1,
            }
        }

        Ok(VerifiedBatchResetResult {
            checked,
            reset,
            still_cooling,
            unhealthy,
        })
    }

    pub fn active_requests_snapshot(&self) -> Vec<ActiveRequestInfo> {
        let now = now_ms();
        let mut out = {
            let mut guard = self.active_requests.lock().unwrap();
            guard.retain(|_, v| now.saturating_sub(v.started_at) < 120_000);
            guard.values().cloned().collect::<Vec<_>>()
        };
        out.extend(self.antigravity.active_requests_snapshot());
        out.extend(self.custom.active_requests_snapshot());
        out
    }

    pub fn recent_provider_signals(&self) -> (Option<String>, Option<String>) {
        let now = now_ms();
        let oc_last = self.last_provider.lock().unwrap().clone();
        let oc_err = self
            .last_error_provider
            .lock()
            .unwrap()
            .clone()
            .filter(|(_, ts)| now.saturating_sub(*ts) < 60_000);
        let (ag_last_ts, ag_err_ts) = self.antigravity.recent_signals();
        let (cu_last, cu_err) = self.custom.recent_signals();

        let pick_latest = |items: Vec<(String, i64)>| -> Option<String> {
            items.into_iter().max_by_key(|(_, ts)| *ts).map(|(p, _)| p)
        };

        let mut last_candidates = Vec::new();
        if let Some(x) = oc_last {
            last_candidates.push(x);
        }
        if let Some(ts) = ag_last_ts {
            last_candidates.push(("antigravity".to_string(), ts));
        }
        if let Some(x) = cu_last {
            last_candidates.push(x);
        }

        let mut err_candidates = Vec::new();
        if let Some(x) = oc_err {
            err_candidates.push(x);
        }
        if let Some(ts) = ag_err_ts {
            err_candidates.push(("antigravity".to_string(), ts));
        }
        if let Some(x) = cu_err {
            err_candidates.push(x);
        }

        (pick_latest(last_candidates), pick_latest(err_candidates))
    }

    fn mark_error_provider(&self, provider_id: &str) {
        let mut guard = self.last_error_provider.lock().unwrap();
        *guard = Some((provider_id.to_string(), now_ms()));
    }

    pub async fn execute_chat(&self, req: ProviderRequest) -> Result<ProviderStreamResponse> {
        let now = now_ms();
        if let Some((cp, model_id, effort)) = self.custom.resolve_target(&req.raw_model).await? {
            return self.custom.execute_chat(req, cp, model_id, effort).await;
        }
        let parsed = parse_model_spec(&req.raw_model);
        let provider = resolve_provider_for_model(&parsed);

        if provider == ProviderKind::Antigravity {
            return self.antigravity.execute_chat(req, parsed).await;
        }
        let provider_cfg = self
            .store
            .provider_configs()
            .get_or_default(provider.id())
            .await?;
        if !provider_cfg.enabled {
            return Err(TesRouteError::BadRequest(format!(
                "provider '{}' is currently disabled in Provider Settings",
                provider.id()
            )));
        }

        let target_format = resolve_wire_target(provider, &parsed.model_id, req.source_format);

        let mut upstream_body = crate::translator::translate_request(
            req.source_format,
            target_format,
            &parsed.model_id,
            &req.body,
        );

        let is_title_call =
            is_opencode_title_request(&req.body) || is_opencode_title_request(&upstream_body);
        if is_title_call {
            return Ok(synthesize_title_stream(
                provider,
                &parsed.model_id,
                req.source_format,
                &req.body,
            ));
        }

        let (injected_forced_effort, reasoning_level) = if let Some(effort) = &parsed.reasoning_effort
        {
            let applied = apply_thinking_level(target_format, &mut upstream_body, effort, true);
            (applied, Some(effort.clone()))
        } else {
            let applied = apply_forced_thinking_level(
                target_format,
                &mut upstream_body,
                &provider_cfg.thinking_level,
            );
            let lvl = (!provider_cfg.thinking_level.eq_ignore_ascii_case("auto"))
                .then(|| provider_cfg.thinking_level.clone());
            (applied, lvl)
        };

        normalize_opencode_request_for_lane(&parsed.model_id, &mut upstream_body, target_format);
        let rename_map = apply_fingerprint_tools(&mut upstream_body, target_format);

        let target_url = self.build_upstream_url(target_format);
        let body_bytes = encode_opencode_body(&upstream_body, target_format)?;

        self.execute_free_with_ip_rotation(
            req,
            parsed,
            provider,
            target_format,
            target_url,
            upstream_body,
            body_bytes,
            rename_map,
            injected_forced_effort,
            reasoning_level,
            is_title_call,
            now,
        )
        .await
    }

    fn build_upstream_url(&self, target: WireFormat) -> String {
        let raw_base = self.config.free_base_url.trim().trim_end_matches('/');
        let inference_base = if raw_base.to_ascii_lowercase().ends_with("/inference") {
            raw_base.to_string()
        } else {
            format!("{raw_base}/inference")
        };

        match target {
            WireFormat::OpenAiChat => format!("{inference_base}/openai/v1/chat/completions"),
            WireFormat::OpenAiResponses => format!("{inference_base}/openai/v1/responses"),
            WireFormat::AnthropicMessages => {
                format!("{inference_base}/anthropic/v1/messages?beta=true")
            }
        }
    }

    async fn quarantine_broken_proxy(&self, proxy_id: &str, proxy_name: &str) {
        let ts = now_ms();
        self.mihomo.mark_proxy_unhealthy(proxy_name, ts);
        let _ = self
            .store
            .proxies()
            .sync_mihomo_delay(proxy_name, false, None, ts)
            .await;
        let _ = self
            .store
            .proxies()
            .record_proxy_failure(proxy_id, ts)
            .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_free_with_ip_rotation(
        &self,
        req: ProviderRequest,
        parsed: ParsedModel,
        provider: ProviderKind,
        target_format: WireFormat,
        target_url: String,
        mut upstream_body: Value,
        mut body_bytes: Bytes,
        rename_map: ToolRenameMap,
        injected_forced_effort: bool,
        reasoning_level: Option<String>,
        is_title_call: bool,
        now: i64,
    ) -> Result<ProviderStreamResponse> {
        let conv_hint = extract_conversation_hint(&req.headers, &req.body);
        let native_session = extract_native_opencode_session(&req.headers);
        let parent_session = extract_parent_session(&req.headers, &req.body);
        let child_session = extract_child_session(&req.headers);
        let oc_cfg = self
            .store
            .provider_configs()
            .get_or_default("opencode")
            .await?;

        let candidates = self
            .selector
            .round_robin_candidates_for_config(&self.mihomo, now, &oc_cfg)
            .await?;

        let model_label = display_name_for_model(provider, &parsed.model_id);
        if candidates.is_empty() {
            if is_title_call {
                return Ok(synthesize_title_stream(
                    provider,
                    &parsed.model_id,
                    req.source_format,
                    &req.body,
                ));
            }
            return Err(TesRouteError::ModelQuotaExhausted(format!(
                "No quota available for {model_label}."
            )));
        }

        let wire_format_label = target_format.as_str().to_string();
        let mut last_error: Option<String> = None;
        let mut saw_rate_limit = false;
        let mut attempt_legs: Vec<AttemptLeg> = Vec::new();
        let mut retried_stale_reasoning = false;
        let mut retried_without_forced_effort = false;
        let iter_candidates: Vec<IpCandidate> = if is_title_call {
            candidates.into_iter().take(3).collect()
        } else {
            candidates
        };

        for candidate in iter_candidates {
            let attempt_start = now_ms();
            let ip_key = candidate.quota.ip_key.clone();
            let egress_display = candidate
                .proxy
                .egress_ip
                .clone()
                .or_else(|| candidate.proxy.ip.clone())
                .or_else(|| Some(ip_key.clone()));
            let mut retried_fresh_session = false;
            let mut overload_attempt = 0usize;

            loop {
                let attempt_num = attempt_legs.len() + 1;
                let session = self
                    .resolve_session_for_ip(
                        &ip_key,
                        native_session.as_deref(),
                        conv_hint.as_deref(),
                        oc_cfg.session_ttl_ms,
                    )
                    .await?;

                let headers = self.build_free_headers_with_lineage(
                    &req.headers,
                    &session,
                    parent_session.as_deref(),
                    child_session.as_deref(),
                    target_format,
                );
                let upstream_auth = if target_format == WireFormat::AnthropicMessages {
                    None
                } else {
                    Some("Bearer public".to_string())
                };

                let relay_resp = self
                    .mihomo
                    .relay(RelayRequest {
                        target_url: target_url.clone(),
                        proxy_name: candidate.proxy.name.clone(),
                        method: "POST".to_string(),
                        fingerprint: "opencode".to_string(),
                        timeout_ms: self.config.timeout_ms,
                        upstream_authorization: upstream_auth,
                        headers,
                        body: body_bytes.clone(),
                    })
                    .await;

                let resp = match relay_resp {
                    Ok(r) => r,
                    Err(err) => {
                        let msg = err.to_string();
                        let lat = (now_ms() - attempt_start).max(0);
                        warn!(
                            proxy = %candidate.proxy.name,
                            error = %msg,
                            "relay transport error; marking proxy idle immediately"
                        );
                        self.quarantine_broken_proxy(&candidate.proxy.id, &candidate.proxy.name)
                            .await;
                        attempt_legs.push(AttemptLeg {
                            attempt: attempt_num,
                            proxy: candidate.proxy.name.clone(),
                            egress_ip: egress_display.clone(),
                            status: 502,
                            latency_ms: lat,
                            phase: "relay_transport".into(),
                            model: Some(parsed.model_id.clone()),
                            wire_model: None,
                            reasoning_level: reasoning_level.clone(),
                            cooldown_applied: Some("quarantined until healthy".into()),
                            session_id: Some(session.clone()),
                            detail: Some(truncate_snippet(&msg, 240)),
                        });
                        let trace_json = serde_json::to_string(&attempt_legs).ok();
                        let _ = self
                            .store
                            .usage()
                            .insert_log(UsageLogInput {
                                request_id: req.request_id.clone(),
                                api_key_id: req.api_key_id.clone(),
                                provider_id: provider.id().to_string(),
                                model: parsed.model_id.clone(),
                                proxy_name: candidate.proxy.name.clone(),
                                egress_ip: egress_display.clone(),
                                status_code: 502,
                                prompt_tokens: 0,
                                cached_tokens: 0,
                                completion_tokens: 0,
                                total_tokens: 0,
                                latency_ms: lat,
                                ttft_ms: None,
                                tokens_per_sec: None,
                                error_kind: Some("relay_transport".into()),
                                error_detail: Some(truncate_detail(&msg)),
                                upstream_url: Some(target_url.clone()),
                                attempt_count: attempt_legs.len() as i64,
                                attempt_trace: trace_json,
                                session_id: Some(session),
                                wire_format: Some(wire_format_label.clone()),
                            })
                            .await;
                        self.mark_error_provider(provider.id());
                        last_error = Some(msg);
                        break;
                    }
                };

                if let Some(date_hdr) = resp.headers().get("date").and_then(|v| v.to_str().ok()) {
                    observe_upstream_date_header(date_hdr);
                }

                let status = resp.status().as_u16();
                if status == 502
                    && resp.headers().get("x-tesie-proxy-used").is_none()
                    && resp.headers().get("x-tesroute-proxy-used").is_none()
                {
                    let err_txt = resp.text().await.unwrap_or_default();
                    let lat = (now_ms() - attempt_start).max(0);
                    let dial_phase = extract_dial_phase(&err_txt);
                    let err_kind = format!("upstream_dial_{dial_phase}");
                    warn!(
                        proxy = %candidate.proxy.name,
                        phase = %dial_phase,
                        detail = %err_txt,
                        "TesieCore upstream dial failed; marking proxy idle immediately"
                    );
                    self.quarantine_broken_proxy(&candidate.proxy.id, &candidate.proxy.name)
                        .await;
                    attempt_legs.push(AttemptLeg {
                        attempt: attempt_num,
                        proxy: candidate.proxy.name.clone(),
                        egress_ip: egress_display.clone(),
                        status: 502,
                        latency_ms: lat,
                        phase: err_kind.clone(),
                        model: Some(parsed.model_id.clone()),
                        wire_model: None,
                        reasoning_level: reasoning_level.clone(),
                        cooldown_applied: Some("quarantined until healthy".into()),
                        session_id: Some(session.clone()),
                        detail: Some(truncate_snippet(&err_txt, 240)),
                    });
                    let trace_json = serde_json::to_string(&attempt_legs).ok();
                    let _ = self
                        .store
                        .usage()
                        .insert_log(UsageLogInput {
                            request_id: req.request_id.clone(),
                            api_key_id: req.api_key_id.clone(),
                            provider_id: provider.id().to_string(),
                            model: parsed.model_id.clone(),
                            proxy_name: candidate.proxy.name.clone(),
                            egress_ip: egress_display.clone(),
                            status_code: 502,
                            prompt_tokens: 0,
                            cached_tokens: 0,
                            completion_tokens: 0,
                            total_tokens: 0,
                            latency_ms: lat,
                            ttft_ms: None,
                            tokens_per_sec: None,
                            error_kind: Some(err_kind),
                            error_detail: Some(truncate_detail(&err_txt)),
                            upstream_url: Some(target_url.clone()),
                            attempt_count: attempt_legs.len() as i64,
                            attempt_trace: trace_json,
                            session_id: Some(session),
                            wire_format: Some(wire_format_label.clone()),
                        })
                        .await;
                    self.mark_error_provider(provider.id());
                    last_error = Some(err_txt);
                    break;
                }

                if (200..300).contains(&status) {
                    match pre_commit_sse_response(resp).await {
                        SsePreCommit::Ok { stream } => {
                            let _ = self
                                .store
                                .proxies()
                                .reset_proxy_failures(&candidate.proxy.id, now_ms())
                                .await;
                            return Ok(self.wrap_success_stream(
                                stream,
                                req,
                                parsed,
                                provider,
                                target_format,
                                target_url,
                                session,
                                attempt_legs,
                                rename_map,
                                reasoning_level,
                                candidate,
                                attempt_start,
                            ));
                        }
                        SsePreCommit::UpstreamError {
                            is_quota,
                            raw_detail,
                        } => {
                            let lat = (now_ms() - attempt_start).max(0);
                            let queue_interval_ms = oc_cfg.probe_interval_sec.clamp(5, 86_400) * 1000;
                            if is_quota {
                                self.invalidate_session(&ip_key, conv_hint.as_deref(), &session)
                                    .await;
                                saw_rate_limit = true;
                                let _ = self
                                    .store
                                    .quotas()
                                    .record_rate_limit(
                                        &ip_key,
                                        Some(queue_interval_ms),
                                        &raw_detail,
                                        now_ms(),
                                    )
                                    .await;
                            }
                            let phase = if is_quota {
                                "sse_quota_exhausted"
                            } else {
                                "sse_upstream_error"
                            };
                            attempt_legs.push(AttemptLeg {
                                attempt: attempt_num,
                                proxy: candidate.proxy.name.clone(),
                                egress_ip: egress_display.clone(),
                                status: if is_quota { 429 } else { 502 },
                                latency_ms: lat,
                                phase: phase.into(),
                                model: Some(parsed.model_id.clone()),
                                wire_model: None,
                                reasoning_level: reasoning_level.clone(),
                                cooldown_applied: is_quota.then(|| {
                                    format!(
                                        "queued (verify every {}s)",
                                        oc_cfg.probe_interval_sec.clamp(5, 86_400)
                                    )
                                }),
                                session_id: Some(session.clone()),
                                detail: Some(truncate_snippet(&raw_detail, 240)),
                            });
                            let trace_json = serde_json::to_string(&attempt_legs).ok();
                            let _ = self
                                .store
                                .usage()
                                .insert_log(UsageLogInput {
                                    request_id: req.request_id.clone(),
                                    api_key_id: req.api_key_id.clone(),
                                    provider_id: provider.id().to_string(),
                                    model: parsed.model_id.clone(),
                                    proxy_name: candidate.proxy.name.clone(),
                                    egress_ip: egress_display.clone(),
                                    status_code: if is_quota { 429 } else { 502 },
                                    prompt_tokens: 0,
                                    cached_tokens: 0,
                                    completion_tokens: 0,
                                    total_tokens: 0,
                                    latency_ms: lat,
                                    ttft_ms: None,
                                    tokens_per_sec: None,
                                    error_kind: Some(phase.into()),
                                    error_detail: Some(truncate_detail(&raw_detail)),
                                    upstream_url: Some(target_url.clone()),
                                    attempt_count: attempt_legs.len() as i64,
                                    attempt_trace: trace_json,
                                    session_id: Some(session),
                                    wire_format: Some(wire_format_label.clone()),
                                })
                                .await;
                            self.mark_error_provider(provider.id());
                            last_error = Some(raw_detail);
                            break;
                        }
                        SsePreCommit::TransportError(msg) => {
                            let lat = (now_ms() - attempt_start).max(0);
                            self.quarantine_broken_proxy(&candidate.proxy.id, &candidate.proxy.name)
                                .await;
                            attempt_legs.push(AttemptLeg {
                                attempt: attempt_num,
                                proxy: candidate.proxy.name.clone(),
                                egress_ip: egress_display.clone(),
                                status: 502,
                                latency_ms: lat,
                                phase: "sse_stream_transport".into(),
                                model: Some(parsed.model_id.clone()),
                                wire_model: None,
                                reasoning_level: reasoning_level.clone(),
                                cooldown_applied: Some("quarantined until healthy".into()),
                                session_id: Some(session.clone()),
                                detail: Some(truncate_snippet(&msg, 240)),
                            });
                            last_error = Some(msg);
                            break;
                        }
                    }
                }

                let err_body = read_limited_error(resp).await;
                let lat = (now_ms() - attempt_start).max(0);

                // 1-shot replay without forced thinking level if the upstream model rejects that effort
                if injected_forced_effort
                    && !retried_without_forced_effort
                    && is_effort_rejection(status, &err_body)
                    && strip_forced_thinking_level(target_format, &mut upstream_body)
                    && let Ok(reencoded) = encode_opencode_body(&upstream_body, target_format)
                {
                    retried_without_forced_effort = true;
                    body_bytes = reencoded;
                    attempt_legs.push(AttemptLeg {
                        attempt: attempt_num,
                        proxy: candidate.proxy.name.clone(),
                        egress_ip: egress_display.clone(),
                        status,
                        latency_ms: lat,
                        phase: "effort_fallback_replay".into(),
                        model: Some(parsed.model_id.clone()),
                        wire_model: None,
                        reasoning_level: reasoning_level.clone(),
                        cooldown_applied: None,
                        session_id: Some(session),
                        detail: Some(truncate_snippet(&err_body, 180)),
                    });
                    continue;
                }

                // 1-shot Responses API replay when upstream reports a stale reasoning reference (`rs_...`)
                if status == 400
                    && target_format == WireFormat::OpenAiResponses
                    && !retried_stale_reasoning
                    && is_stale_reasoning_reference(&err_body)
                    && strip_stale_reasoning_inputs(&mut upstream_body)
                    && let Ok(reencoded) = encode_opencode_body(&upstream_body, target_format)
                {
                    retried_stale_reasoning = true;
                    body_bytes = reencoded;
                    attempt_legs.push(AttemptLeg {
                        attempt: attempt_num,
                        proxy: candidate.proxy.name.clone(),
                        egress_ip: egress_display.clone(),
                        status: 400,
                        latency_ms: lat,
                        phase: "stale_reasoning_replay".into(),
                        model: Some(parsed.model_id.clone()),
                        wire_model: None,
                        reasoning_level: reasoning_level.clone(),
                        cooldown_applied: None,
                        session_id: Some(session),
                        detail: Some(truncate_snippet(&err_body, 180)),
                    });
                    continue;
                }

                if status == 429 && OVERLOAD_RE.is_match(&err_body) {
                    overload_attempt += 1;
                    if overload_attempt < MAX_OVERLOAD_RETRIES {
                        let wait_ms = OVERLOAD_BASE_MS * overload_attempt as u64;
                        debug!(
                            proxy = %candidate.proxy.name,
                            attempt = overload_attempt,
                            "OpenCode free lane overloaded; retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                        continue;
                    }
                }

                let is_quota_exceeded = is_upstream_quota_exhausted(&err_body);
                if status == 429 || status == 402 || status == 403 || (status == 400 && is_quota_exceeded) {
                    self.invalidate_session(&ip_key, conv_hint.as_deref(), &session)
                        .await;

                    let is_abuse_block = status == 403 && is_network_abuse_block(&err_body);
                    if status == 403
                        && !is_abuse_block
                        && !is_quota_exceeded
                        && !retried_fresh_session
                        && native_session.is_none()
                    {
                        retried_fresh_session = true;
                        continue;
                    }

                    saw_rate_limit = true;
                    let queue_interval_sec = oc_cfg.probe_interval_sec.clamp(5, 86_400);
                    // Put this exit IP into Queued state (`cooldown`) until an N-second live probe confirms 200 OK
                    let explicit_cooldown_ms = if is_abuse_block {
                        Some(NETWORK_ABUSE_COOLDOWN_MS)
                    } else {
                        Some(queue_interval_sec * 1000)
                    };

                    let _ = self
                        .store
                        .quotas()
                        .record_rate_limit(&ip_key, explicit_cooldown_ms, &err_body, now_ms())
                        .await;

                    let err_kind = if is_abuse_block {
                        "ip_abuse_block_403".to_string()
                    } else if status == 429 || status == 402 {
                        "rate_limited_429".to_string()
                    } else {
                        format!("http_{status}")
                    };

                    attempt_legs.push(AttemptLeg {
                        attempt: attempt_num,
                        proxy: candidate.proxy.name.clone(),
                        egress_ip: egress_display.clone(),
                        status,
                        latency_ms: lat,
                        phase: err_kind.clone(),
                        model: Some(parsed.model_id.clone()),
                        wire_model: None,
                        reasoning_level: reasoning_level.clone(),
                        cooldown_applied: Some(if is_abuse_block {
                            "30m (IP blocked)".into()
                        } else {
                            format!("queued (verify every {queue_interval_sec}s)")
                        }),
                        session_id: Some(session.clone()),
                        detail: Some(truncate_snippet(&err_body, 240)),
                    });
                    let trace_json = serde_json::to_string(&attempt_legs).ok();

                    let _ = self
                        .store
                        .usage()
                        .insert_log(UsageLogInput {
                            request_id: req.request_id.clone(),
                            api_key_id: req.api_key_id.clone(),
                            provider_id: provider.id().to_string(),
                            model: parsed.model_id.clone(),
                            proxy_name: candidate.proxy.name.clone(),
                            egress_ip: egress_display.clone(),
                            status_code: status as i64,
                            prompt_tokens: 0,
                            cached_tokens: 0,
                            completion_tokens: 0,
                            total_tokens: 0,
                            latency_ms: lat,
                            ttft_ms: None,
                            tokens_per_sec: None,
                            error_kind: Some(err_kind),
                            error_detail: Some(truncate_detail(&err_body)),
                            upstream_url: Some(target_url.clone()),
                            attempt_count: attempt_legs.len() as i64,
                            attempt_trace: trace_json,
                            session_id: Some(session),
                            wire_format: Some(wire_format_label.clone()),
                        })
                        .await;
                    self.mark_error_provider(provider.id());

                    last_error = Some(format!("[{status}] {err_body}"));
                    break;
                }

                // Request-scoped deterministic error (400/404/422/5xx) — do NOT cool down exit IP
                let err_kind = if is_model_unavailable(status, &err_body) {
                    "model_unavailable_400".to_string()
                } else {
                    format!("http_{status}")
                };
                attempt_legs.push(AttemptLeg {
                    attempt: attempt_num,
                    proxy: candidate.proxy.name.clone(),
                    egress_ip: egress_display.clone(),
                    status,
                    latency_ms: lat,
                    phase: err_kind.clone(),
                    model: Some(parsed.model_id.clone()),
                    wire_model: None,
                    reasoning_level: reasoning_level.clone(),
                    cooldown_applied: None,
                    session_id: Some(session.clone()),
                    detail: Some(truncate_snippet(&err_body, 240)),
                });
                let trace_json = serde_json::to_string(&attempt_legs).ok();

                let _ = self
                    .store
                    .usage()
                    .insert_log(UsageLogInput {
                        request_id: req.request_id.clone(),
                        api_key_id: req.api_key_id.clone(),
                        provider_id: provider.id().to_string(),
                        model: parsed.model_id.clone(),
                        proxy_name: candidate.proxy.name.clone(),
                        egress_ip: egress_display.clone(),
                        status_code: status as i64,
                        prompt_tokens: 0,
                        cached_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: 0,
                        latency_ms: lat,
                        ttft_ms: None,
                        tokens_per_sec: None,
                        error_kind: Some(err_kind),
                        error_detail: Some(truncate_detail(&err_body)),
                        upstream_url: Some(target_url.clone()),
                        attempt_count: attempt_legs.len() as i64,
                        attempt_trace: trace_json,
                        session_id: Some(session),
                        wire_format: Some(wire_format_label.clone()),
                    })
                    .await;
                self.mark_error_provider(provider.id());

                if (500..600).contains(&status) {
                    last_error = Some(format!("[{status}] {err_body}"));
                    break;
                }

                return Err(TesRouteError::Upstream {
                    status,
                    message: err_body,
                });
            }
        }

        // Cosmetic OpenCode CLI title calls synthesize a clean local title if all tried exits 429'd
        if is_title_call {
            return Ok(synthesize_title_stream(
                provider,
                &parsed.model_id,
                req.source_format,
                &req.body,
            ));
        }

        if saw_rate_limit
            || last_error
                .as_deref()
                .map(is_upstream_quota_exhausted)
                .unwrap_or(false)
        {
            Err(TesRouteError::ModelQuotaExhausted(format!(
                "No quota available for {model_label}."
            )))
        } else {
            Err(TesRouteError::ProxyRelay(
                last_error.unwrap_or_else(|| "All proxy attempts failed".into()),
            ))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn wrap_success_stream<S>(
        &self,
        upstream: S,
        req: ProviderRequest,
        parsed: ParsedModel,
        provider: ProviderKind,
        target_format: WireFormat,
        target_url: String,
        session_id: String,
        prior_legs: Vec<AttemptLeg>,
        rename_map: ToolRenameMap,
        reasoning_level: Option<String>,
        candidate: IpCandidate,
        attempt_start: i64,
    ) -> ProviderStreamResponse
    where
        S: Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    {
        let store = self.store.clone();
        let source_format = req.source_format;
        let wire_format_label = target_format.as_str().to_string();
        let translator = StreamTranslator::new(
            source_format,
            target_format,
            parsed.model_id.clone(),
            rename_map,
        );
        let ip_key = candidate.quota.ip_key.clone();
        let proxy_name = candidate.proxy.name.clone();
        let egress_ip = candidate
            .proxy
            .egress_ip
            .clone()
            .or(candidate.proxy.ip.clone());
        let model_id = parsed.model_id.clone();
        let provider_id = provider.id().to_string();

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
        let guard = ActiveStreamGuard {
            key: req.request_id.clone(),
            active_requests: self.active_requests.clone(),
        };
        let last_provider = self.last_provider.clone();

        struct StreamState<S> {
            upstream: S,
            translator: StreamTranslator,
            pending: std::collections::VecDeque<Bytes>,
            finished: bool,
            committed: bool,
            store: Store,
            ip_key: String,
            proxy_name: String,
            egress_ip: Option<String>,
            request_id: String,
            api_key_id: Option<String>,
            provider_id: String,
            model_id: String,
            target_url: String,
            session_id: String,
            wire_format_label: String,
            prior_legs: Vec<AttemptLeg>,
            reasoning_level: Option<String>,
            attempt_start: i64,
            first_token_at: Option<i64>,
            _guard: ActiveStreamGuard,
            last_provider: Arc<Mutex<Option<(String, i64)>>>,
        }

        let initial = StreamState {
            upstream,
            translator,
            pending: std::collections::VecDeque::new(),
            finished: false,
            committed: false,
            store,
            ip_key,
            proxy_name: proxy_name.clone(),
            egress_ip: egress_ip.clone(),
            request_id: req.request_id,
            api_key_id: req.api_key_id,
            provider_id: provider_id.clone(),
            model_id: model_id.clone(),
            target_url,
            session_id,
            wire_format_label,
            prior_legs,
            reasoning_level,
            attempt_start,
            first_token_at: None,
            _guard: guard,
            last_provider,
        };

        let out_stream = futures::stream::unfold(initial, |mut st| async move {
            loop {
                if let Some(next_chunk) = st.pending.pop_front() {
                    return Some((Ok(next_chunk), st));
                }
                if st.finished {
                    if !st.committed {
                        st.committed = true;
                        st.translator.usage.finalize_fallback();
                        let usage = st.translator.usage;
                        let now = now_ms();
                        {
                            if let Ok(mut lp) = st.last_provider.lock() {
                                *lp = Some((st.provider_id.clone(), now));
                            }
                        }
                        let latency_ms = (now - st.attempt_start).max(1);
                        let ttft_ms = st.first_token_at.map(|t| (t - st.attempt_start).max(1));
                        let tokens_per_sec = if usage.completion_tokens > 0 {
                            let decode_window_ms = match st.first_token_at {
                                Some(t) if now - t >= 100 => now - t,
                                _ => latency_ms.max(50),
                            };
                            let raw =
                                usage.completion_tokens as f64 / (decode_window_ms as f64 / 1000.0);
                            Some((raw * 100.0).round() / 100.0)
                        } else {
                            None
                        };

                        let mut legs = st.prior_legs.clone();
                        legs.push(AttemptLeg {
                            attempt: legs.len() + 1,
                            proxy: st.proxy_name.clone(),
                            egress_ip: st.egress_ip.clone(),
                            status: 200,
                            latency_ms,
                            phase: "ok".into(),
                            model: Some(st.model_id.clone()),
                            wire_model: None,
                            reasoning_level: st.reasoning_level.clone(),
                            cooldown_applied: None,
                            session_id: Some(st.session_id.clone()),
                            detail: None,
                        });
                        let attempt_count = legs.len() as i64;
                        let attempt_trace = serde_json::to_string(&legs).ok();

                        let _ = st
                            .store
                            .quotas()
                            .record_success(
                                &st.ip_key,
                                usage.prompt_tokens,
                                usage.completion_tokens,
                                now,
                            )
                            .await;
                        if let Some(kid) = &st.api_key_id {
                            let _ = st
                                .store
                                .keys()
                                .add_usage(kid, usage.total_tokens, now)
                                .await;
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
                                tokens_per_sec,
                                error_kind: None,
                                error_detail: None,
                                upstream_url: Some(st.target_url.clone()),
                                attempt_count,
                                attempt_trace,
                                session_id: Some(st.session_id.clone()),
                                wire_format: Some(st.wire_format_label.clone()),
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
                        for frame in frames {
                            st.pending.push_back(frame);
                        }
                    }
                    Some(Err(err)) => {
                        st.finished = true;
                        return Some((
                            Err(TesRouteError::ProxyRelay(format!(
                                "upstream stream error: {err}"
                            ))),
                            st,
                        ));
                    }
                    None => {
                        for frame in st.translator.finish() {
                            st.pending.push_back(frame);
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

    async fn resolve_session_for_ip(
        &self,
        ip_key: &str,
        native_session: Option<&str>,
        conv_hint: Option<&str>,
        ttl_ms: i64,
    ) -> Result<String> {
        if let Some(native) = native_session {
            return Ok(native.to_string());
        }

        let now = now_ms();
        let calibrated_now = upstream_now_ms().max(0) as u64;
        if let Some(hint) = conv_hint {
            let key = format!("{ip_key}\0{hint}");
            {
                let mut guard = self.conv_sessions.lock().unwrap();
                if let Some((sid, created_at)) = guard.get(&key)
                    && now.saturating_sub(*created_at) < ttl_ms
                {
                    return Ok(sid.clone());
                }
                if guard.len() >= MAX_CONV_SESSIONS
                    && let Some(oldest) = guard
                        .iter()
                        .min_by_key(|(_, (_, ts))| *ts)
                        .map(|(k, _)| k.clone())
                {
                    guard.remove(&oldest);
                }
                let minted = canonical_session_id(&key);
                guard.insert(key, (minted.clone(), now));
                return Ok(minted);
            }
        }

        self.store
            .quotas()
            .get_or_mint_session(ip_key, ttl_ms, now, || {
                generate_session_id(Some(calibrated_now))
            })
            .await
    }

    async fn invalidate_session(&self, ip_key: &str, conv_hint: Option<&str>, session: &str) {
        if let Some(hint) = conv_hint {
            let key = format!("{ip_key}\0{hint}");
            let mut guard = self.conv_sessions.lock().unwrap();
            if guard.get(&key).is_some_and(|(s, _)| s == session) {
                guard.remove(&key);
            }
        }
        let _ = self.store.quotas().invalidate_session(ip_key, session).await;
    }

    fn build_free_headers(
        &self,
        downstream_headers: &[(String, String)],
        session: &str,
        target_format: WireFormat,
    ) -> Vec<(String, String)> {
        self.build_free_headers_with_lineage(downstream_headers, session, None, None, target_format)
    }

    fn build_free_headers_with_lineage(
        &self,
        downstream_headers: &[(String, String)],
        session: &str,
        parent_session: Option<&str>,
        child_session: Option<&str>,
        target_format: WireFormat,
    ) -> Vec<(String, String)> {
        let downstream_ua = header_ci(downstream_headers, "user-agent").unwrap_or("");
        let ua = if has_valid_opencode_v2_ua(downstream_ua) {
            downstream_ua.to_string()
        } else {
            OPENCODE_V2_UA.to_string()
        };
        let client = header_ci(downstream_headers, "x-opencode-client").unwrap_or(OPENCODE_CLIENT);
        let project = match header_ci(downstream_headers, "x-opencode-project")
            .map(str::trim)
            .filter(|s| is_valid_v2_project_id(s))
        {
            Some(p) => p.to_string(),
            None => random_project_id(),
        };

        let root_session = parent_session.unwrap_or(session);
        let leaf_session = child_session.unwrap_or(session);

        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("User-Agent".into(), ua),
            ("x-opencode-client".into(), client.to_string()),
            ("x-opencode-session".into(), root_session.to_string()),
            ("x-opencode-session-id".into(), leaf_session.to_string()),
            ("x-session-affinity".into(), root_session.to_string()),
            ("X-Session-Id".into(), root_session.to_string()),
            ("x-opencode-project".into(), project),
        ];

        if let Some(parent) = parent_session {
            headers.push(("x-opencode-parent-session-id".into(), parent.to_string()));
            headers.push(("x-parent-session-id".into(), parent.to_string()));
        }

        if target_format == WireFormat::AnthropicMessages {
            headers.push(("x-api-key".into(), "public".into()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
            headers.push((
                "anthropic-beta".into(),
                "interleaved-thinking-2025-05-14".into(),
            ));
        }
        headers
    }
}

enum SingleProbeOutcome {
    Ready,
    Cooling,
    Unhealthy,
    Inconclusive,
}

fn synthesize_title_stream(
    provider: ProviderKind,
    model_id: &str,
    source_format: WireFormat,
    raw_body: &Value,
) -> ProviderStreamResponse {
    let title = synthesize_title_from_body(raw_body);
    let created = now_ms() / 1000;
    let c1 = json!({
        "id": format!("chatcmpl-title-{created}"),
        "object": "chat.completion.chunk",
        "created": created,
        "model": model_id,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": title },
            "finish_reason": Value::Null
        }]
    });
    let c2 = json!({
        "id": format!("chatcmpl-title-{created}"),
        "object": "chat.completion.chunk",
        "created": created,
        "model": model_id,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 0,
            "completion_tokens": 3,
            "total_tokens": 3
        }
    });
    let payload = format!("data: {c1}\n\ndata: {c2}\n\ndata: [DONE]\n\n");
    ProviderStreamResponse {
        provider_id: provider.id().to_string(),
        model: model_id.to_string(),
        proxy_name: "local-title-synth".to_string(),
        egress_ip: None,
        source_format,
        stream: Box::pin(stream::once(async move { Ok(Bytes::from(payload)) })),
    }
}

fn is_model_unavailable(status: u16, body: &str) -> bool {
    if !matches!(status, 400 | 404 | 410 | 422) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    [
        "model is unavailable",
        "model_not_found",
        "model_not_available",
        "model_retired",
        "model_deprecated",
        "unknown model",
        "model does not exist",
        "model has been retired",
        "model is no longer available",
    ]
    .iter()
    .any(|m| lower.contains(m))
}

fn extract_dial_phase(err_txt: &str) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(err_txt)
        && let Some(p) = json.get("phase").and_then(Value::as_str)
        && !p.trim().is_empty()
    {
        return p.trim().to_string();
    }
    let lower = err_txt.to_ascii_lowercase();
    if lower.contains("eof") {
        "eof".to_string()
    } else if lower.contains("x509:") || lower.contains("tls:") || lower.contains("certificate") {
        "tls_verify_failed".to_string()
    } else if lower.contains("timeout") || lower.contains("deadline") {
        "timeout".to_string()
    } else {
        "502".to_string()
    }
}

fn is_network_abuse_block(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    if lower.contains("abusive traffic")
        || lower.contains("abuse traffic")
        || lower.contains("abuse of traffic")
        || (lower.contains("free models aren't available")
            && lower.contains("from your current network"))
    {
        return true;
    }
    (lower.contains("ip") || lower.contains("network") || lower.contains("exit"))
        && (lower.contains("blocked")
            || lower.contains("abuse")
            || lower.contains("suspicious")
            || lower.contains("forbidden"))
}

fn truncate_detail(raw: &str) -> String {
    raw.trim().chars().take(4096).collect()
}

fn truncate_snippet(raw: &str, max_chars: usize) -> String {
    raw.trim().chars().take(max_chars).collect()
}

async fn read_limited_error(resp: reqwest::Response) -> String {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::new();
    while let Some(Ok(chunk)) = stream.next().await {
        let remain = 16_384_usize.saturating_sub(buf.len());
        if remain == 0 {
            break;
        }
        let take = chunk.len().min(remain);
        buf.extend_from_slice(&chunk[..take]);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

pub fn parse_reset_ms(headers: &HeaderMap, body: &str) -> Option<i64> {
    for hname in ["retry-after", "x-ratelimit-reset-after"] {
        if let Some(val) = headers.get(hname).and_then(|v| v.to_str().ok())
            && let Ok(secs) = val.trim().parse::<u64>()
            && secs > 0
        {
            return Some((secs as i64).saturating_mul(1000));
        }
    }

    if let Ok(json) = serde_json::from_str::<Value>(body) {
        for ptr in [
            "/retry_after",
            "/resetAfter",
            "/error/retry_after",
            "/error/resetAfter",
        ] {
            if let Some(secs) = json.pointer(ptr).and_then(Value::as_u64)
                && secs > 0
            {
                return Some((secs as i64).saturating_mul(1000));
            }
        }
    }

    if let Some(caps) = RESET_AFTER_RE.captures(body)
        && let Some(m) = caps.get(1)
        && let Ok(secs) = m.as_str().parse::<i64>()
        && secs > 0
    {
        return Some(secs.saturating_mul(1000));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reset_hints_from_headers_and_body() {
        let mut hm = HeaderMap::new();
        hm.insert("Retry-After", "180".parse().unwrap());
        assert_eq!(parse_reset_ms(&hm, ""), Some(180_000));

        let empty = HeaderMap::new();
        assert_eq!(
            parse_reset_ms(
                &empty,
                r#"{"error":{"message":"Rate limit exceeded, reset after 95s"}}"#
            ),
            Some(95_000)
        );
    }

    #[test]
    fn classifies_dial_phase_model_unavailable_and_network_abuse() {
        assert_eq!(
            extract_dial_phase(r#"{"error":"upstream_dial_failed","phase":"eof","detail":"EOF"}"#),
            "eof"
        );
        assert!(is_network_abuse_block(
            r#"{"error":{"message":"Your IP address has been blocked for suspicious activity"}}"#
        ));
        assert!(is_model_unavailable(
            400,
            r#"{"error":{"type":"server_error","message":"Upstream request failed: Model is unavailable."}}"#
        ));
    }
}
