pub mod oauth;
pub mod partition;
pub mod translator;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::{Stream, StreamExt};
use rand::seq::SliceRandom;
use serde::Serialize;
use tesie_core::{
    antigravity_quota_short_name, canonical_antigravity_model,
    is_excluded_antigravity_model_key, normalize_routing_strategy, AntigravityQuotaSnapshot,
    ParsedModel, ProviderConnection, ProviderKind, ProxyNode, Result, TesRouteError, UsageLogInput,
    WireFormat,
};
use tesie_proxy::{MihomoController, RelayRequest};
use tesie_store::{now_ms, Store};
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::opencode::executor::{
    ActiveRequestInfo, ProviderRequest, ProviderStreamResponse, QuotaProbeStatus,
};
use crate::translator::sse::{pre_commit_sse_response, SsePreCommit};
use crate::translator::{translate_request, StreamTranslator};

use oauth::{
    antigravity_hub_ua, chat_base_url, complete_oauth_from_callback_or_code,
    ensure_fresh_auth_for_connection, fallback_project_id_for_seed,
    import_antigravity_refresh_token, is_account_service_disabled, is_account_unlicensed,
    normalize_quota_snapshot_for_display, refresh_account_quota, start_local_oauth_flow,
    OAuthSessionPollStatus, StartedOAuthFlow, DEFAULT_ANTIGRAVITY_PROJECT_ID,
};
use partition::{compute_account_proxy_partitions, AccountProxyPartition};
use translator::{build_antigravity_envelope, resolve_antigravity_upstream_model, AntigravitySseDecoder};

/// Real-time live request quota exhaustion cooldown (30 minutes).
const LIVE_EXHAUSTION_COOLDOWN_MS: i64 = 30 * 60 * 1000;
/// Escalating model failure cooldown steps: 5s -> 30s -> 30m.
const FAILURE_STEP_1_MS: i64 = 5_000;
const FAILURE_STEP_2_MS: i64 = 30_000;
const FAILURE_STEP_3_MS: i64 = 30 * 60 * 1000;

#[derive(Debug, Clone, Serialize)]
pub struct AntigravityModelFailureState {
    pub model: String,
    pub display_name: String,
    pub consecutive_failures: u32,
    pub cooldown_until: i64,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
struct AttemptTraceLeg {
    attempt: usize,
    account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_email: Option<String>,
    account_name: String,
    model: String,
    wire_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_level: Option<String>,
    proxy: String,
    egress_ip: Option<String>,
    status: u16,
    latency_ms: i64,
    phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cooldown_applied: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_count: Option<u32>,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntigravityAccountView {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub enabled: bool,
    pub status: String,
    pub cooldown_until: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub quota: Option<AntigravityQuotaSnapshot>,
    pub partition: Option<AccountProxyPartition>,
    pub model_cooldowns: Vec<AntigravityModelFailureState>,
}

pub struct AntigravityEngine {
    store: Store,
    mihomo: Arc<MihomoController>,
    oauth_sessions: Arc<Mutex<HashMap<String, OAuthSessionPollStatus>>>,
    active_requests: Arc<Mutex<HashMap<String, ActiveRequestInfo>>>,
    /// Per-(connection_id, canonical_model) 30-minute quota exhaustion locks.
    model_locks: Arc<Mutex<HashMap<(String, String), i64>>>,
    /// Per-(connection_id, canonical_model) escalating failure cooldowns (`5s -> 30s -> 30m`).
    model_failures: Arc<Mutex<HashMap<(String, String), AntigravityModelFailureState>>>,
    last_active_at: Arc<Mutex<Option<i64>>>,
    last_error_at: Arc<Mutex<Option<i64>>>,
    probe_notify: Notify,
    probe_running: AtomicBool,
    last_probe_at: Mutex<Option<i64>>,
    rr_cursor: AtomicUsize,
}

impl AntigravityEngine {
    pub fn new(store: Store, mihomo: Arc<MihomoController>) -> Self {
        Self {
            store,
            mihomo,
            oauth_sessions: Arc::new(Mutex::new(HashMap::new())),
            active_requests: Arc::new(Mutex::new(HashMap::new())),
            model_locks: Arc::new(Mutex::new(HashMap::new())),
            model_failures: Arc::new(Mutex::new(HashMap::new())),
            last_active_at: Arc::new(Mutex::new(None)),
            last_error_at: Arc::new(Mutex::new(None)),
            probe_notify: Notify::new(),
            probe_running: AtomicBool::new(false),
            last_probe_at: Mutex::new(None),
            rr_cursor: AtomicUsize::new(0),
        }
    }

    pub fn apply_account_routing_strategy(
        &self,
        accounts: &mut [ProviderConnection],
        strategy: &str,
    ) {
        let len = accounts.len();
        if len <= 1 {
            return;
        }
        match normalize_routing_strategy(strategy, "fallback").as_str() {
            "round-robin" => {
                let offset = self.rr_cursor.fetch_add(1, Ordering::Relaxed) % len;
                accounts.rotate_left(offset);
            }
            "random" => {
                accounts.shuffle(&mut rand::thread_rng());
            }
            _ => {}
        }
    }

    pub fn notify_probe_config_changed(&self) {
        self.probe_notify.notify_waiters();
    }

    /// Clears all in-memory per-(account, model) exhaustion locks and escalating failure cooldowns.
    pub fn clear_all_model_cooldowns(&self) -> (usize, usize) {
        let locks = if let Ok(mut g) = self.model_locks.lock() {
            let n = g.len();
            g.clear();
            n
        } else {
            0
        };
        let failures = if let Ok(mut g) = self.model_failures.lock() {
            let n = g.len();
            g.clear();
            n
        } else {
            0
        };
        (locks, failures)
    }

    pub async fn probe_status(&self) -> QuotaProbeStatus {
        let cfg = self
            .store
            .provider_configs()
            .get_or_default("antigravity")
            .await
            .ok();
        let enabled = cfg.as_ref().map(|c| c.probe_enabled).unwrap_or(true);
        let interval_sec = cfg
            .as_ref()
            .map(|c| c.probe_interval_sec.clamp(60, 86_400) as u64)
            .unwrap_or(600);
        let last_probe_at = self.last_probe_at.lock().ok().and_then(|g| *g);
        QuotaProbeStatus {
            enabled,
            interval_sec,
            probe_running: self.probe_running.load(Ordering::Relaxed),
            last_probe_at,
            ..Default::default()
        }
    }

    /// Background configurable live per-account quota refresh loop (`fetchAvailableModels`
    /// + `retrieveUserQuotaSummary`) routed through each account's dedicated sticky proxy.
    /// Controlled by `provider_configs` (`probe_enabled`, `probe_interval_sec`, default `600`s).
    pub fn start_auto_quota_sync_loop(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            if let Ok(cfg) = engine
                .store
                .provider_configs()
                .get_or_default("antigravity")
                .await
                && cfg.enabled
                && cfg.probe_enabled
            {
                let _ = engine.refresh_all_account_quotas().await;
            }

            loop {
                let (enabled, interval_sec) = match engine
                    .store
                    .provider_configs()
                    .get_or_default("antigravity")
                    .await
                {
                    Ok(cfg) => (
                        cfg.enabled && cfg.probe_enabled,
                        cfg.probe_interval_sec.clamp(60, 86_400) as u64,
                    ),
                    Err(_) => (true, 600),
                };

                if !enabled {
                    engine.probe_notify.notified().await;
                    continue;
                }

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(interval_sec)) => {
                        let _ = engine.refresh_all_account_quotas().await;
                    }
                    _ = engine.probe_notify.notified() => {
                        continue;
                    }
                }
            }
        });
    }

    pub async fn start_oauth(&self) -> Result<StartedOAuthFlow> {
        start_local_oauth_flow(
            self.store.clone(),
            self.mihomo.clone(),
            self.oauth_sessions.clone(),
        )
        .await
    }

    pub fn poll_oauth(&self, state: &str) -> Option<OAuthSessionPollStatus> {
        self.oauth_sessions.lock().ok()?.get(state).cloned()
    }

    pub async fn complete_oauth_manual(
        &self,
        callback_url_or_code: &str,
        redirect_uri: Option<&str>,
    ) -> Result<ProviderConnection> {
        complete_oauth_from_callback_or_code(
            &self.store,
            &self.mihomo,
            callback_url_or_code,
            redirect_uri,
        )
        .await
    }

    pub async fn import_refresh_token(
        &self,
        name: Option<&str>,
        email: Option<&str>,
        refresh_token: &str,
        project_id: Option<&str>,
    ) -> Result<ProviderConnection> {
        import_antigravity_refresh_token(
            &self.store,
            &self.mihomo,
            name,
            email,
            refresh_token,
            project_id,
        )
        .await
    }

    pub async fn list_accounts_with_partitions(&self) -> Result<Vec<AntigravityAccountView>> {
        let now = now_ms();
        let cfg = self
            .store
            .provider_configs()
            .get_or_default("antigravity")
            .await?;
        let all_conns = self.store.connections().list_for_provider("antigravity").await?;
        let configured_proxies = self.store.proxies().list_for_provider_config(&cfg).await?;
        let mihomo_states = self.mihomo.fetch_proxy_states().await.unwrap_or_default();

        let enabled_conns: Vec<ProviderConnection> =
            all_conns.iter().filter(|c| c.enabled).cloned().collect();
        let partitions =
            compute_account_proxy_partitions(&enabled_conns, &configured_proxies, &mihomo_states);
        let mut part_by_id: HashMap<String, AccountProxyPartition> = partitions
            .into_iter()
            .map(|p| (p.connection_id.clone(), p))
            .collect();

        // Persist updated sticky_proxy_id when auto-assigned or promoted on failover
        for conn in &enabled_conns {
            if let Some(part) = part_by_id.get(&conn.id)
                && let Some(primary) = &part.primary_proxy
                && !primary.is_direct
                && conn.sticky_proxy_id.as_deref() != Some(primary.id.as_str())
            {
                let _ = self
                    .store
                    .connections()
                    .update_sticky_proxy(&conn.id, Some(&primary.id), now)
                    .await;
            }
        }

        let active_locks: HashMap<(String, String), i64> = {
            if let Ok(mut guard) = self.model_locks.lock() {
                guard.retain(|_, &mut until| until > now);
                guard.clone()
            } else {
                HashMap::new()
            }
        };
        let active_failures: HashMap<(String, String), AntigravityModelFailureState> = {
            if let Ok(mut guard) = self.model_failures.lock() {
                // Keep entries whose cooldown hasn't aged far past recovery (keep streak visible slightly)
                guard.retain(|_, st| st.cooldown_until > now - FAILURE_STEP_3_MS);
                guard.clone()
            } else {
                HashMap::new()
            }
        };

        let views = all_conns
            .into_iter()
            .map(|c| {
                let mut quota = c
                    .quota_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<AntigravityQuotaSnapshot>(s).ok())
                    .map(normalize_quota_snapshot_for_display);

                // Overlay 30m quota locks and escalating failure cooldowns onto per-model snapshot
                let mut all_models_exhausted_until: Option<i64> = None;
                let mut acct_model_cooldowns: Vec<AntigravityModelFailureState> = active_failures
                    .iter()
                    .filter(|((cid, _), st)| cid == &c.id && st.cooldown_until > now)
                    .map(|(_, st)| st.clone())
                    .collect();
                acct_model_cooldowns.sort_by(|a, b| a.model.cmp(&b.model));

                if let Some(snap) = quota.as_mut() {
                    for entry in &mut snap.models {
                        if let Some(&lock_until) =
                            active_locks.get(&(c.id.clone(), entry.model.clone()))
                            && lock_until > now
                        {
                            entry.remaining_percentage = 0.0;
                            entry.reset_at = Some(
                                entry
                                    .reset_at
                                    .map(|r| r.max(lock_until))
                                    .unwrap_or(lock_until),
                            );
                        }
                        if let Some(fstate) =
                            active_failures.get(&(c.id.clone(), entry.model.clone()))
                            && fstate.cooldown_until > now
                        {
                            entry.failure_count = fstate.consecutive_failures;
                            entry.failure_cooldown_until = Some(fstate.cooldown_until);
                            entry.failure_reason = fstate.last_error.clone();
                        }
                    }
                    if !snap.models.is_empty()
                        && snap.models.iter().all(|m| m.remaining_percentage <= 0.0)
                    {
                        all_models_exhausted_until =
                            snap.models.iter().filter_map(|m| m.reset_at).max();
                    }
                }

                let effective_cooldown = match (c.cooldown_until, all_models_exhausted_until) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
                let is_cooling = effective_cooldown.is_some_and(|until| until > now);
                let is_banned = !c.enabled
                    && c.last_error
                        .as_deref()
                        .is_some_and(is_account_service_disabled);
                let status = if is_banned {
                    "banned".to_string()
                } else if !c.enabled {
                    "disabled".to_string()
                } else if is_cooling {
                    "cooldown".to_string()
                } else {
                    "ready".to_string()
                };
                let partition = part_by_id.remove(&c.id);
                AntigravityAccountView {
                    id: c.id,
                    name: c.name,
                    email: c.email,
                    enabled: c.enabled,
                    status,
                    cooldown_until: effective_cooldown,
                    last_error: c.last_error,
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                    quota,
                    partition,
                    model_cooldowns: acct_model_cooldowns,
                }
            })
            .collect();

        Ok(views)
    }

    pub async fn refresh_all_account_quotas(&self) -> Result<usize> {
        let cfg = self
            .store
            .provider_configs()
            .get_or_default("antigravity")
            .await?;
        if !cfg.enabled {
            return Ok(0);
        }
        let conns = self.store.connections().list_for_provider("antigravity").await?;
        let enabled: Vec<ProviderConnection> = conns.into_iter().filter(|c| c.enabled).collect();
        if enabled.is_empty() {
            return Ok(0);
        }

        self.probe_running.store(true, Ordering::Relaxed);
        let proxies = self.store.proxies().list_for_provider_config(&cfg).await?;
        let states = self.mihomo.fetch_proxy_states().await.unwrap_or_default();
        let parts = compute_account_proxy_partitions(&enabled, &proxies, &states);
        let part_map: HashMap<String, String> = parts
            .into_iter()
            .filter_map(|p| p.primary_proxy.map(|pr| (p.connection_id, pr.name)))
            .collect();

        let mut refreshed = 0usize;
        for conn in enabled {
            let proxy_name = part_map
                .get(&conn.id)
                .map(String::as_str)
                .unwrap_or("DIRECT");
            if refresh_account_quota(&self.store, &self.mihomo, &conn, Some(proxy_name))
                .await
                .is_ok()
            {
                refreshed += 1;
            }
        }
        self.probe_running.store(false, Ordering::Relaxed);
        if let Ok(mut lp) = self.last_probe_at.lock() {
            *lp = Some(now_ms());
        }
        Ok(refreshed)
    }

    pub async fn refresh_single_account_quota(
        &self,
        conn_id: &str,
    ) -> Result<AntigravityQuotaSnapshot> {
        let conn = self
            .store
            .connections()
            .get_connection(conn_id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("account {conn_id} not found")))?;
        let snap = refresh_account_quota(&self.store, &self.mihomo, &conn, None).await?;
        if let Ok(mut lp) = self.last_probe_at.lock() {
            *lp = Some(now_ms());
        }
        Ok(snap)
    }

    pub fn active_requests_snapshot(&self) -> Vec<ActiveRequestInfo> {
        let now = now_ms();
        let mut guard = self.active_requests.lock().unwrap();
        guard.retain(|_, v| now.saturating_sub(v.started_at) < 120_000);
        guard.values().cloned().collect()
    }

    pub fn recent_signals(&self) -> (Option<i64>, Option<i64>) {
        let now = now_ms();
        let last = self
            .last_active_at
            .lock()
            .unwrap()
            .filter(|&ts| now.saturating_sub(ts) < 600_000);
        let err = self
            .last_error_at
            .lock()
            .unwrap()
            .filter(|&ts| now.saturating_sub(ts) < 120_000);
        (last, err)
    }

    fn check_account_model_availability(
        &self,
        conn: &ProviderConnection,
        wire_model: &str,
        raw_model_id: &str,
        now: i64,
    ) -> AccountModelAvailability {
        if is_excluded_antigravity_model_key(raw_model_id)
            || is_excluded_antigravity_model_key(wire_model)
        {
            return AccountModelAvailability::NoQuota;
        }
        let (canon_id, _) = canonical_antigravity_model(raw_model_id);
        let wire_key = wire_model.to_ascii_lowercase();
        if let Ok(mut locks) = self.model_locks.lock() {
            locks.retain(|_, &mut until| until > now);
            if locks.contains_key(&(conn.id.clone(), canon_id.clone()))
                || locks.contains_key(&(conn.id.clone(), wire_key))
            {
                return AccountModelAvailability::NoQuota;
            }
        }

        if let Some(raw) = conn.quota_json.as_deref()
            && let Ok(raw_snap) = serde_json::from_str::<AntigravityQuotaSnapshot>(raw)
        {
            let snap = normalize_quota_snapshot_for_display(raw_snap);
            if snap.fetched_at > 0 {
                let Some(entry) = snap
                    .models
                    .iter()
                    .find(|e| e.model.eq_ignore_ascii_case(&canon_id))
                else {
                    return AccountModelAvailability::NoQuota;
                };
                if !entry.unlocked
                    || (entry.remaining_percentage <= 0.0 && entry.reset_at.is_none_or(|r| r > now))
                {
                    return AccountModelAvailability::NoQuota;
                }
            }
        }

        // Check per-(account, model) escalating failure cooldown ONLY after confirming quota exists
        if let Ok(failures) = self.model_failures.lock()
            && let Some(st) = failures.get(&(conn.id.clone(), canon_id))
            && st.cooldown_until > now
        {
            return AccountModelAvailability::FailureCooldown {
                until: st.cooldown_until,
            };
        }

        AccountModelAvailability::Ready
    }

    fn record_model_quota_block(
        &self,
        conn_id: &str,
        raw_or_wire_model: &str,
        err_body: &str,
        now: i64,
    ) -> i64 {
        let (canon_id, _) = canonical_antigravity_model(raw_or_wire_model);
        let key = (conn_id.to_string(), canon_id);
        // Real-time exhaustion puts this node/model into at least a 30-minute cooldown,
        // honoring Google's explicit quotaResetTimeStamp if even longer.
        let min_cooldown_until = now.saturating_add(LIVE_EXHAUSTION_COOLDOWN_MS);
        let until = extract_google_quota_reset_ms(err_body, now)
            .map(|r| r.max(min_cooldown_until))
            .unwrap_or(min_cooldown_until);

        if let Ok(mut locks) = self.model_locks.lock() {
            locks.insert(key, until);
        }
        until
    }

    fn record_model_failure(
        &self,
        conn_id: &str,
        raw_model_id: &str,
        reason: &str,
        now: i64,
    ) -> (u32, i64, String) {
        let (canon_id, display_name) = canonical_antigravity_model(raw_model_id);
        let key = (conn_id.to_string(), canon_id.clone());
        let mut guard = self.model_failures.lock().unwrap();
        let next_count = guard
            .get(&key)
            .map(|st| st.consecutive_failures.saturating_add(1))
            .unwrap_or(1);
        let dur_ms = escalate_failure_cooldown_ms(next_count);
        let until = now.saturating_add(dur_ms);
        let clean_reason: String = reason.chars().take(240).collect();

        guard.insert(
            key,
            AntigravityModelFailureState {
                model: canon_id,
                display_name,
                consecutive_failures: next_count,
                cooldown_until: until,
                last_error: Some(clean_reason),
                updated_at: now,
            },
        );
        let label = format!(
            "{} (failure #{next_count})",
            format_cooldown_step_label(dur_ms)
        );
        (next_count, until, label)
    }

    fn clear_model_failures_and_locks(&self, conn_id: &str, raw_or_wire_model: &str) {
        let (canon_id, _) = canonical_antigravity_model(raw_or_wire_model);
        if let Ok(mut failures) = self.model_failures.lock() {
            failures.remove(&(conn_id.to_string(), canon_id.clone()));
        }
        if let Ok(mut locks) = self.model_locks.lock() {
            locks.remove(&(conn_id.to_string(), canon_id));
            locks.remove(&(conn_id.to_string(), raw_or_wire_model.to_ascii_lowercase()));
        }
    }

    async fn quarantine_proxy(&self, proxy: &ProxyNode) {
        if proxy.is_direct {
            return;
        }
        let ts = now_ms();
        self.mihomo.mark_proxy_unhealthy(&proxy.name, ts);
        let _ = self
            .store
            .proxies()
            .sync_mihomo_delay(&proxy.name, false, None, ts)
            .await;
        let _ = self
            .store
            .proxies()
            .record_proxy_failure(&proxy.id, ts)
            .await;
    }

    pub async fn execute_chat(
        &self,
        req: ProviderRequest,
        parsed: ParsedModel,
    ) -> Result<ProviderStreamResponse> {
        let now = now_ms();
        let ag_cfg = self
            .store
            .provider_configs()
            .get_or_default("antigravity")
            .await?;
        if !ag_cfg.enabled {
            return Err(TesRouteError::BadRequest(
                "Antigravity provider is currently disabled in Provider Settings".into(),
            ));
        }

        let short_model_name = antigravity_quota_short_name(&parsed.model_id);

        let all_conns = self
            .store
            .connections()
            .list_for_provider("antigravity")
            .await?;
        let any_enabled = all_conns.iter().any(|c| c.enabled);
        if !any_enabled {
            return Err(TesRouteError::BadRequest(
                "No enabled Antigravity accounts connected — add an account in Providers → Antigravity".into(),
            ));
        }

        let mut accounts = self
            .store
            .connections()
            .list_enabled_for_provider("antigravity", now)
            .await?;
        if accounts.is_empty() {
            let all_tos_disabled = all_conns
                .iter()
                .filter(|c| c.enabled)
                .all(|c| c.last_error.as_deref().is_some_and(is_account_service_disabled));
            if all_tos_disabled {
                return Err(TesRouteError::BadRequest(
                    "All Antigravity Google accounts have Cloud Code service disabled (Terms of Service).".into(),
                ));
            }
            return Err(TesRouteError::ModelQuotaExhausted(format!(
                "No quota available for {short_model_name}."
            )));
        }

        let configured_proxies = self
            .store
            .proxies()
            .list_for_provider_config(&ag_cfg)
            .await?;
        let mihomo_states = self.mihomo.fetch_proxy_states().await?;
        let partitions =
            compute_account_proxy_partitions(&accounts, &configured_proxies, &mihomo_states);
        let part_map: HashMap<String, AccountProxyPartition> = partitions
            .into_iter()
            .map(|p| (p.connection_id.clone(), p))
            .collect();

        // Ensure every enabled account has an initial quota snapshot so fail-fast is 100% accurate
        for acct in &mut accounts {
            if acct.quota_json.is_none() {
                let p_name = part_map
                    .get(&acct.id)
                    .and_then(|p| p.primary_proxy.as_ref())
                    .map(|pr| pr.name.as_str())
                    .unwrap_or("DIRECT");
                if let Ok(snap) =
                    refresh_account_quota(&self.store, &self.mihomo, acct, Some(p_name)).await
                    && let Ok(raw_json) = serde_json::to_string(&snap)
                {
                    acct.quota_json = Some(raw_json);
                }
            }
        }

        let wire_model = resolve_antigravity_upstream_model(
            &parsed.model_id,
            parsed.reasoning_effort.as_deref(),
        );
        let reasoning_level = parsed.reasoning_effort.clone().or_else(|| {
            wire_model
                .rsplit('-')
                .next()
                .filter(|s| matches!(*s, "high" | "medium" | "low"))
                .map(str::to_string)
        });

        // Pre-flight check: separate NoQuota accounts from accounts with quota currently in failure cooldown
        let check_now = now_ms();
        let mut eligible_accounts: Vec<ProviderConnection> = Vec::new();
        let mut failure_cooldown_untils: Vec<i64> = Vec::new();
        for acct in accounts {
            match self.check_account_model_availability(&acct, &wire_model, &parsed.model_id, check_now) {
                AccountModelAvailability::Ready => eligible_accounts.push(acct),
                AccountModelAvailability::FailureCooldown { until } => {
                    failure_cooldown_untils.push(until);
                }
                AccountModelAvailability::NoQuota => {}
            }
        }

        if eligible_accounts.is_empty() {
            if let Some(soonest) = failure_cooldown_untils.into_iter().min() {
                let secs = ((soonest - check_now).max(1000) + 999) / 1000;
                return Err(TesRouteError::ModelCoolingDown {
                    message: format!(
                        "All accounts for {short_model_name} are cooling down, next retry in {secs}s"
                    ),
                    retry_after_secs: secs as u64,
                });
            }
            return Err(TesRouteError::ModelQuotaExhausted(format!(
                "No quota available for {short_model_name}."
            )));
        }

        // Normalize caller request into OpenAI Chat shape
        let chat_body = translate_request(
            req.source_format,
            WireFormat::OpenAiChat,
            &parsed.model_id,
            &req.body,
        );
        let target_url = format!(
            "{}/v1internal:streamGenerateContent?alt=sse",
            chat_base_url().trim_end_matches('/')
        );

        self.apply_account_routing_strategy(&mut eligible_accounts, &ag_cfg.routing_strategy);

        let gateway_settings = self.store.settings().get_settings().await.unwrap_or_default();
        let exhaustive_failover = gateway_settings.exhaustive_failover;

        let eligible_accounts_iter: Vec<ProviderConnection> = if exhaustive_failover {
            eligible_accounts.clone()
        } else {
            eligible_accounts.iter().take(1).cloned().collect()
        };

        let mut trace_legs: Vec<AttemptTraceLeg> = Vec::new();
        let mut last_err_msg: Option<String> = None;
        let mut saw_quota_exhausted = false;
        let mut saw_failure_cooldown = false;

        // Walk eligible Antigravity accounts in configured routing_strategy order (`fallback` | `round-robin` | `random`).
        // Notice `part_map` is keyed by `account.id` so every account always stays pinned to its disjoint proxy slice!
        for account in &eligible_accounts_iter {
            let Some(part) = part_map.get(&account.id) else {
                continue;
            };
            if part.healthy_fallback_chain.is_empty() {
                let (fc, _, cooldown_lbl) = self.record_model_failure(
                    &account.id,
                    &parsed.model_id,
                    "Account has no healthy proxies in its assigned slice",
                    now_ms(),
                );
                saw_failure_cooldown = true;
                trace_legs.push(AttemptTraceLeg {
                    attempt: trace_legs.len() + 1,
                    account_id: account.id.clone(),
                    account_email: account.email.clone(),
                    account_name: account.name.clone(),
                    model: parsed.model_id.clone(),
                    wire_model: wire_model.clone(),
                    reasoning_level: reasoning_level.clone(),
                    proxy: "none".into(),
                    egress_ip: None,
                    status: 502,
                    latency_ms: 0,
                    phase: "no_healthy_proxy".into(),
                    cooldown_applied: Some(cooldown_lbl),
                    failure_count: Some(fc),
                    detail: Some("Account has no healthy proxies in its assigned slice".into()),
                });
                last_err_msg = Some(format!(
                    "Account {} has no healthy proxies in its assigned slice",
                    account.name
                ));
                continue;
            }

            // Walk this account's dedicated proxy slice in Sticky Primary -> Ordered Fallback order
            // Cap to 2 proxies (primary + 1 backup) so a single Bearer token never hops rapidly across many IPs.
            let mut move_to_next_account = false;
            let mut account_had_proxy_failure = false;
            for proxy in part.healthy_fallback_chain.iter().take(2) {
                let mut retried_refresh = false;
                loop {
                    let leg_start = now_ms();
                    let attempt_no = trace_legs.len() + 1;

                    let auth = match ensure_fresh_auth_for_connection(
                        &self.store,
                        &self.mihomo,
                        account,
                        &proxy.name,
                        retried_refresh,
                    )
                    .await
                    {
                        Ok(a) => a,
                        Err(err) => {
                            let err_s = err.to_string();
                            let err_ts = now_ms();
                            if is_account_service_disabled(&err_s) {
                                let _ = self
                                    .store
                                    .connections()
                                    .disable_banned_connection(
                                        &account.id,
                                        "Banned: This service has been disabled in this account for violation of Terms of Service.",
                                        err_ts,
                                    )
                                    .await;
                            } else {
                                let (fc, _, cooldown_lbl) = self.record_model_failure(
                                    &account.id,
                                    &parsed.model_id,
                                    &err_s,
                                    err_ts,
                                );
                                saw_failure_cooldown = true;
                                trace_legs.push(AttemptTraceLeg {
                                    attempt: attempt_no,
                                    account_id: account.id.clone(),
                                    account_email: account.email.clone(),
                                    account_name: account.name.clone(),
                                    model: parsed.model_id.clone(),
                                    wire_model: wire_model.clone(),
                                    reasoning_level: reasoning_level.clone(),
                                    proxy: proxy.name.clone(),
                                    egress_ip: proxy.ip.clone(),
                                    status: 401,
                                    latency_ms: (err_ts - leg_start).max(0),
                                    phase: "oauth_refresh_failed".into(),
                                    cooldown_applied: Some(cooldown_lbl),
                                    failure_count: Some(fc),
                                    detail: Some(err_s.chars().take(200).collect()),
                                });
                            }
                            last_err_msg = Some(err_s);
                            move_to_next_account = true;
                            break;
                        }
                    };

                    let fallback_pid = fallback_project_id_for_seed(&account.id);
                    let project_id = auth
                        .project_id
                        .as_deref()
                        .filter(|s| !s.is_empty() && *s != DEFAULT_ANTIGRAVITY_PROJECT_ID)
                        .unwrap_or(&fallback_pid);

                    let envelope_bytes = build_antigravity_envelope(
                        &chat_body,
                        project_id,
                        &wire_model,
                        &account.id,
                    )?;

                    let relay_res = self
                        .mihomo
                        .relay(RelayRequest {
                            target_url: target_url.clone(),
                            proxy_name: proxy.name.clone(),
                            method: "POST".to_string(),
                            fingerprint: "standard".to_string(),
                            timeout_ms: 120_000,
                            upstream_authorization: Some(format!("Bearer {}", auth.access_token)),
                            headers: vec![
                                ("Content-Type".into(), "application/json".into()),
                                ("User-Agent".into(), antigravity_hub_ua().into()),
                                ("Accept".into(), "text/event-stream".into()),
                            ],
                            body: envelope_bytes,
                        })
                        .await;

                    let resp = match relay_res {
                        Ok(r) => r,
                        Err(err) => {
                            let msg = err.to_string();
                            account_had_proxy_failure = true;
                            warn!(account = %account.name, proxy = %proxy.name, error = %msg, "Antigravity proxy relay error; falling back to next proxy in account slice");
                            self.quarantine_proxy(proxy).await;
                            trace_legs.push(AttemptTraceLeg {
                                attempt: attempt_no,
                                account_id: account.id.clone(),
                                account_email: account.email.clone(),
                                account_name: account.name.clone(),
                                model: parsed.model_id.clone(),
                                wire_model: wire_model.clone(),
                                reasoning_level: reasoning_level.clone(),
                                proxy: proxy.name.clone(),
                                egress_ip: proxy.ip.clone(),
                                status: 502,
                                latency_ms: (now_ms() - leg_start).max(0),
                                phase: "relay_transport".into(),
                                cooldown_applied: Some("proxy quarantined".into()),
                                failure_count: None,
                                detail: Some(msg.chars().take(200).collect()),
                            });
                            last_err_msg = Some(msg);
                            break; // Try next fallback proxy in THIS account's slice
                        }
                    };

                    let status = resp.status().as_u16();
                    if status == 502
                        && resp.headers().get("x-tesie-proxy-used").is_none()
                        && resp.headers().get("x-tesroute-proxy-used").is_none()
                    {
                        let err_txt = resp.text().await.unwrap_or_default();
                        account_had_proxy_failure = true;
                        warn!(account = %account.name, proxy = %proxy.name, detail = %err_txt, "Antigravity proxy dial failed; falling back to next proxy in account slice");
                        self.quarantine_proxy(proxy).await;
                        trace_legs.push(AttemptTraceLeg {
                            attempt: attempt_no,
                            account_id: account.id.clone(),
                            account_email: account.email.clone(),
                            account_name: account.name.clone(),
                            model: parsed.model_id.clone(),
                            wire_model: wire_model.clone(),
                            reasoning_level: reasoning_level.clone(),
                            proxy: proxy.name.clone(),
                            egress_ip: proxy.ip.clone(),
                            status: 502,
                            latency_ms: (now_ms() - leg_start).max(0),
                            phase: "upstream_dial_failed".into(),
                            cooldown_applied: Some("proxy quarantined".into()),
                            failure_count: None,
                            detail: Some(err_txt.chars().take(200).collect()),
                        });
                        last_err_msg = Some(err_txt);
                        break; // Try next fallback proxy in THIS account's slice
                    }

                    if (200..300).contains(&status) {
                        match pre_commit_sse_response(resp).await {
                            SsePreCommit::Ok { stream } => {
                                self.clear_model_failures_and_locks(&account.id, &wire_model);
                                // Promote/persist sticky proxy for this account so subsequent requests stay on this IP!
                                if !proxy.is_direct
                                    && account.sticky_proxy_id.as_deref() != Some(proxy.id.as_str())
                                {
                                    let _ = self
                                        .store
                                        .connections()
                                        .update_sticky_proxy(&account.id, Some(&proxy.id), now_ms())
                                        .await;
                                }
                                let _ = self
                                    .store
                                    .connections()
                                    .clear_error(&account.id, now_ms())
                                    .await;
                                info!(account = %account.name, model = %wire_model, proxy = %proxy.name, "Antigravity stream connected");
                                return Ok(self.wrap_antigravity_stream(
                                    stream,
                                    req,
                                    parsed,
                                    wire_model,
                                    reasoning_level,
                                    target_url,
                                    account.clone(),
                                    proxy.clone(),
                                    trace_legs,
                                    leg_start,
                                ));
                            }
                            SsePreCommit::UpstreamError {
                                is_quota,
                                raw_detail,
                            } => {
                                let err_ts = now_ms();
                                let lat = (err_ts - leg_start).max(0);
                                let (fc, cooldown_lbl, phase) = if is_account_service_disabled(&raw_detail) {
                                    let (reason, lbl, kind) = if is_account_unlicensed(&raw_detail) {
                                        (
                                            "Disabled: No active Gemini Code Assist license on this account (#3501).",
                                            "auto-disabled (no license)",
                                            "account_unlicensed_403",
                                        )
                                    } else {
                                        (
                                            "Banned: This service has been disabled in this account for violation of Terms of Service.",
                                            "auto-disabled (ToS ban)",
                                            "account_banned_403",
                                        )
                                    };
                                    let _ = self
                                        .store
                                        .connections()
                                        .disable_banned_connection(
                                            &account.id,
                                            reason,
                                            err_ts,
                                        )
                                        .await;
                                    (None, lbl.to_string(), kind)
                                } else if is_quota && is_quota_exhaustion_error(429, &raw_detail) {
                                    saw_quota_exhausted = true;
                                    let _ = self.record_model_quota_block(
                                        &account.id,
                                        &parsed.model_id,
                                        &raw_detail,
                                        err_ts,
                                    );
                                    (None, "30m (quota exhausted)".to_string(), "sse_quota_exhausted")
                                } else {
                                    saw_failure_cooldown = true;
                                    let (cnt, _, lbl) = self.record_model_failure(
                                        &account.id,
                                        &parsed.model_id,
                                        &raw_detail,
                                        err_ts,
                                    );
                                    (Some(cnt), lbl, "sse_model_failure")
                                };
                                trace_legs.push(AttemptTraceLeg {
                                    attempt: attempt_no,
                                    account_id: account.id.clone(),
                                    account_email: account.email.clone(),
                                    account_name: account.name.clone(),
                                    model: parsed.model_id.clone(),
                                    wire_model: wire_model.clone(),
                                    reasoning_level: reasoning_level.clone(),
                                    proxy: proxy.name.clone(),
                                    egress_ip: proxy.ip.clone(),
                                    status: 502,
                                    latency_ms: lat,
                                    phase: phase.into(),
                                    cooldown_applied: Some(cooldown_lbl),
                                    failure_count: fc,
                                    detail: Some(raw_detail.chars().take(220).collect()),
                                });
                                let trace_json = serde_json::to_string(&trace_legs).ok();
                                let _ = self
                                    .store
                                    .usage()
                                    .insert_log(UsageLogInput {
                                        request_id: req.request_id.clone(),
                                        api_key_id: req.api_key_id.clone(),
                                        provider_id: "antigravity".into(),
                                        model: parsed.model_id.clone(),
                                        proxy_name: proxy.name.clone(),
                                        egress_ip: proxy.ip.clone(),
                                        status_code: if is_quota { 429 } else { 502 },
                                        prompt_tokens: 0,
                                        cached_tokens: 0,
                                        completion_tokens: 0,
                                        total_tokens: 0,
                                        latency_ms: lat,
                                        ttft_ms: None,
                                        tokens_per_sec: None,
                                        error_kind: Some(phase.into()),
                                        error_detail: Some(raw_detail.chars().take(1500).collect()),
                                        upstream_url: Some(target_url.clone()),
                                        attempt_count: trace_legs.len() as i64,
                                        attempt_trace: trace_json,
                                        session_id: Some(account.id.clone()),
                                        wire_format: Some(format!("gemini-native ({wire_model})")),
                                    })
                                    .await;
                                *self.last_error_at.lock().unwrap() = Some(err_ts);
                                last_err_msg = Some(raw_detail);
                                move_to_next_account = true;
                                break;
                            }
                            SsePreCommit::TransportError(msg) => {
                                account_had_proxy_failure = true;
                                self.quarantine_proxy(proxy).await;
                                trace_legs.push(AttemptTraceLeg {
                                    attempt: attempt_no,
                                    account_id: account.id.clone(),
                                    account_email: account.email.clone(),
                                    account_name: account.name.clone(),
                                    model: parsed.model_id.clone(),
                                    wire_model: wire_model.clone(),
                                    reasoning_level: reasoning_level.clone(),
                                    proxy: proxy.name.clone(),
                                    egress_ip: proxy.ip.clone(),
                                    status: 502,
                                    latency_ms: (now_ms() - leg_start).max(0),
                                    phase: "sse_stream_transport".into(),
                                    cooldown_applied: Some("proxy quarantined".into()),
                                    failure_count: None,
                                    detail: Some(msg.chars().take(200).collect()),
                                });
                                last_err_msg = Some(msg);
                                break;
                            }
                        }
                    }

                    let err_body = resp.text().await.unwrap_or_default();
                    let err_ts = now_ms();
                    let lat = (err_ts - leg_start).max(0);
                    let is_unlicensed = is_account_unlicensed(&err_body);
                    let account_tos_disabled = is_account_service_disabled(&err_body);

                    if (status == 401 || (status == 403 && !account_tos_disabled))
                        && !retried_refresh
                        && !auth.refresh_token.is_empty()
                    {
                        retried_refresh = true;
                        continue;
                    }

                    // Context length / token limit exceeded: fast-fail direct to client without retrying other accounts
                    if tesie_core::is_context_length_error(status, &err_body) {
                        return Err(TesRouteError::ContextLengthExceeded(
                            tesie_core::extract_clean_context_message(&err_body),
                        ));
                    }

                    // Deterministic client 400 / 422 invalid-request error: do NOT burn cooldown or retry other accounts
                    if matches!(status, 400 | 422) {
                        return Err(TesRouteError::BadRequest(
                            tesie_core::extract_clean_context_message(&err_body),
                        ));
                    }

                    // Classify outcome into Banned vs Unlicensed vs Quota Exhausted vs Escalating Model Failure Cooldown
                    let (cooldown_lbl, fc, err_kind) = if account_tos_disabled {
                        let (reason, lbl, kind) = if is_unlicensed {
                            (
                                "Disabled: No active Gemini Code Assist license on this account (#3501).",
                                "auto-disabled (no license)",
                                "account_unlicensed_403",
                            )
                        } else {
                            (
                                "Banned: This service has been disabled in this account for violation of Terms of Service.",
                                "auto-disabled (ToS ban)",
                                "account_banned_403",
                            )
                        };
                        let _ = self
                            .store
                            .connections()
                            .disable_banned_connection(&account.id, reason, err_ts)
                            .await;
                        (
                            Some(lbl.to_string()),
                            None,
                            kind.to_string(),
                        )
                    } else if is_quota_exhaustion_error(status, &err_body) {
                        saw_quota_exhausted = true;
                        let _ = self.record_model_quota_block(
                            &account.id,
                            &parsed.model_id,
                            &err_body,
                            err_ts,
                        );
                        (
                            Some("30m (quota exhausted)".to_string()),
                            None,
                            "account_quota_429".to_string(),
                        )
                    } else {
                        saw_failure_cooldown = true;
                        let (cnt, _, lbl) = self.record_model_failure(
                            &account.id,
                            &parsed.model_id,
                            &format!("[{status}] {}", err_body.chars().take(180).collect::<String>()),
                            err_ts,
                        );
                        (
                            Some(lbl),
                            Some(cnt),
                            if status == 429 {
                                "rate_burst_429".to_string()
                            } else {
                                format!("http_{status}")
                            },
                        )
                    };

                    trace_legs.push(AttemptTraceLeg {
                        attempt: attempt_no,
                        account_id: account.id.clone(),
                        account_email: account.email.clone(),
                        account_name: account.name.clone(),
                        model: parsed.model_id.clone(),
                        wire_model: wire_model.clone(),
                        reasoning_level: reasoning_level.clone(),
                        proxy: proxy.name.clone(),
                        egress_ip: proxy.ip.clone(),
                        status,
                        latency_ms: lat,
                        phase: err_kind.clone(),
                        cooldown_applied: cooldown_lbl,
                        failure_count: fc,
                        detail: Some(err_body.chars().take(220).collect()),
                    });

                    let trace_json = serde_json::to_string(&trace_legs).ok();
                    let _ = self
                        .store
                        .usage()
                        .insert_log(UsageLogInput {
                            request_id: req.request_id.clone(),
                            api_key_id: req.api_key_id.clone(),
                            provider_id: "antigravity".into(),
                            model: parsed.model_id.clone(),
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
                            error_kind: Some(err_kind),
                            error_detail: Some(err_body.chars().take(1500).collect()),
                            upstream_url: Some(target_url.clone()),
                            attempt_count: trace_legs.len() as i64,
                            attempt_trace: trace_json,
                            session_id: Some(account.id.clone()),
                            wire_format: Some(format!("gemini-native ({wire_model})")),
                        })
                        .await;
                    *self.last_error_at.lock().unwrap() = Some(err_ts);

                    if !account_tos_disabled {
                        let _ = refresh_account_quota(
                            &self.store,
                            &self.mihomo,
                            account,
                            Some(&proxy.name),
                        )
                        .await;
                    }
                    last_err_msg = Some(format!("[{status}] {err_body}"));
                    move_to_next_account = true;
                    break;
                }

                if move_to_next_account {
                    break;
                }
            }

            if !move_to_next_account && account_had_proxy_failure {
                // Every proxy in this account's fallback slice timed out / failed transport
                saw_failure_cooldown = true;
                let (fc, _, cooldown_lbl) = self.record_model_failure(
                    &account.id,
                    &parsed.model_id,
                    last_err_msg.as_deref().unwrap_or("All slice proxies timed out"),
                    now_ms(),
                );
                if let Some(last_leg) = trace_legs.last_mut()
                    && last_leg.account_id == account.id
                {
                    last_leg.cooldown_applied = Some(cooldown_lbl);
                    last_leg.failure_count = Some(fc);
                }
            }
        }

        if saw_failure_cooldown {
            let cur = now_ms();
            let canon_id = canonical_antigravity_model(&parsed.model_id).0;
            let soonest_until = self
                .model_failures
                .lock()
                .ok()
                .and_then(|guard| {
                    eligible_accounts
                        .iter()
                        .filter_map(|a| guard.get(&(a.id.clone(), canon_id.clone())))
                        .map(|st| st.cooldown_until)
                        .filter(|&u| u > cur)
                        .min()
                });
            if let Some(until) = soonest_until {
                let secs = ((until - cur).max(1000) + 999) / 1000;
                return Err(TesRouteError::ModelCoolingDown {
                    message: format!(
                        "All accounts for {short_model_name} are cooling down, next retry in {secs}s"
                    ),
                    retry_after_secs: secs as u64,
                });
            }
        }

        if saw_quota_exhausted {
            return Err(TesRouteError::ModelQuotaExhausted(format!(
                "No quota available for {short_model_name}."
            )));
        }

        Err(TesRouteError::ProxyRelay(
            last_err_msg.unwrap_or_else(|| "All Antigravity proxy attempts failed".into()),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn wrap_antigravity_stream<S>(
        &self,
        mut raw_stream: S,
        req: ProviderRequest,
        parsed: ParsedModel,
        wire_model: String,
        reasoning_level: Option<String>,
        target_url: String,
        account: ProviderConnection,
        proxy: ProxyNode,
        mut trace_legs: Vec<AttemptTraceLeg>,
        started_at: i64,
    ) -> ProviderStreamResponse
    where
        S: Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    {
        let store = self.store.clone();
        let active_map = self.active_requests.clone();
        let last_active_at = self.last_active_at.clone();
        let req_id = req.request_id.clone();
        let api_key_id = req.api_key_id.clone();
        let display_model = parsed.model_id.clone();
        let source_format = req.source_format;
        let proxy_name = proxy.name.clone();
        let egress_ip = proxy.ip.clone();
        let account_id = account.id.clone();
        let account_email = account.email.clone();
        let account_name = account.name.clone();

        *last_active_at.lock().unwrap() = Some(started_at);
        active_map.lock().unwrap().insert(
            req_id.clone(),
            ActiveRequestInfo {
                request_id: req_id.clone(),
                provider_id: "antigravity".to_string(),
                model: display_model.clone(),
                proxy_name: proxy_name.clone(),
                egress_ip: egress_ip.clone(),
                started_at,
            },
        );

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes>>(64);
        tokio::spawn(async move {
            let mut ag_decoder = AntigravitySseDecoder::new(&display_model);
            let mut downstream_translator = if source_format != WireFormat::OpenAiChat {
                Some(StreamTranslator::new(
                    source_format,
                    WireFormat::OpenAiChat,
                    display_model.clone(),
                    Default::default(),
                ))
            } else {
                None
            };
            let mut ttft_ms: Option<i64> = None;

            while let Some(item) = raw_stream.next().await {
                let bytes = match item {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx
                            .send(Err(TesRouteError::Upstream {
                                status: 502,
                                message: format!("Antigravity stream read error: {e}"),
                            }))
                            .await;
                        active_map.lock().unwrap().remove(&req_id);
                        return;
                    }
                };
                if ttft_ms.is_none() && !bytes.is_empty() {
                    ttft_ms = Some((now_ms() - started_at).max(1));
                }

                for oa_chunk in ag_decoder.push_bytes(&bytes) {
                    if let Some(tr) = &mut downstream_translator {
                        for out_chunk in tr.push_bytes(&oa_chunk) {
                            if tx.send(Ok(out_chunk)).await.is_err() {
                                active_map.lock().unwrap().remove(&req_id);
                                return;
                            }
                        }
                    } else if tx.send(Ok(oa_chunk)).await.is_err() {
                        active_map.lock().unwrap().remove(&req_id);
                        return;
                    }
                }
            }

            for oa_chunk in ag_decoder.finish() {
                if let Some(tr) = &mut downstream_translator {
                    for out_chunk in tr.push_bytes(&oa_chunk) {
                        let _ = tx.send(Ok(out_chunk)).await;
                    }
                } else {
                    let _ = tx.send(Ok(oa_chunk)).await;
                }
            }
            if let Some(tr) = &mut downstream_translator {
                for out_chunk in tr.finish() {
                    let _ = tx.send(Ok(out_chunk)).await;
                }
            }

            active_map.lock().unwrap().remove(&req_id);
            let finished = now_ms();
            let latency_ms = (finished - started_at).max(1);
            let gen_ms = match ttft_ms {
                Some(t) if latency_ms > t => latency_ms - t,
                _ => latency_ms,
            };
            let tps = if ag_decoder.completion_tokens > 0 && gen_ms > 0 {
                Some((ag_decoder.completion_tokens as f64) * 1000.0 / (gen_ms as f64))
            } else {
                None
            };

            trace_legs.push(AttemptTraceLeg {
                attempt: trace_legs.len() + 1,
                account_id: account_id.clone(),
                account_email,
                account_name,
                model: display_model.clone(),
                wire_model: wire_model.clone(),
                reasoning_level,
                proxy: proxy_name.clone(),
                egress_ip: egress_ip.clone(),
                status: 200,
                latency_ms,
                phase: "ok".into(),
                cooldown_applied: None,
                failure_count: None,
                detail: Some(format!("wire_model={wire_model}")),
            });
            let trace_json = serde_json::to_string(&trace_legs).ok();
            let uncached_prompt = ag_decoder.uncached_prompt_tokens();
            let total_tokens = uncached_prompt + ag_decoder.completion_tokens;

            if let Some(kid) = &api_key_id
                && total_tokens > 0
            {
                let _ = store.keys().add_usage(kid, total_tokens, finished).await;
            }

            let _ = store
                .usage()
                .insert_log(UsageLogInput {
                    request_id: req_id,
                    api_key_id,
                    provider_id: "antigravity".into(),
                    model: display_model,
                    proxy_name,
                    egress_ip,
                    status_code: 200,
                    prompt_tokens: uncached_prompt,
                    cached_tokens: ag_decoder.cached_tokens,
                    completion_tokens: ag_decoder.completion_tokens,
                    total_tokens,
                    latency_ms,
                    ttft_ms,
                    tokens_per_sec: tps,
                    error_kind: None,
                    error_detail: None,
                    upstream_url: Some(target_url),
                    attempt_count: trace_legs.len() as i64,
                    attempt_trace: trace_json,
                    session_id: Some(account_id),
                    wire_format: Some(format!("gemini-native ({wire_model})")),
                })
                .await;
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        ProviderStreamResponse {
            provider_id: ProviderKind::Antigravity.id().to_string(),
            model: parsed.model_id,
            proxy_name: proxy.name,
            egress_ip: proxy.ip,
            source_format,
            stream: Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<Bytes>> + Send + 'static>>,
        }
    }
}

enum AccountModelAvailability {
    Ready,
    NoQuota,
    FailureCooldown { until: i64 },
}

fn escalate_failure_cooldown_ms(failure_count: u32) -> i64 {
    match failure_count {
        0 | 1 => FAILURE_STEP_1_MS,
        2 => FAILURE_STEP_2_MS,
        _ => FAILURE_STEP_3_MS,
    }
}

fn format_cooldown_step_label(dur_ms: i64) -> &'static str {
    if dur_ms <= FAILURE_STEP_1_MS {
        "5s"
    } else if dur_ms <= FAILURE_STEP_2_MS {
        "30s"
    } else {
        "30m"
    }
}

fn is_quota_exhaustion_error(status: u16, body: &str) -> bool {
    if status == 402 {
        return true;
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("quotaresettimestamp")
        || lower.contains("quota exhausted")
        || lower.contains("exhausted your capacity")
        || lower.contains("insufficient_quota")
        || lower.contains("quota_exhausted")
        || lower.contains("quota exceeded")
        || lower.contains("free usage limit")
        || lower.contains("freeusagelimiterror")
    {
        return true;
    }
    // Google `429 RESOURCE_EXHAUSTED` with `quota` or explicit long reset delay (> 60s)
    status == 429 && lower.contains("quota")
}

fn extract_google_quota_reset_ms(err_body: &str, now_ms: i64) -> Option<i64> {
    let val: serde_json::Value = serde_json::from_str(err_body.trim()).ok()?;
    let details = val.get("error")?.get("details")?.as_array()?;
    for d in details {
        if let Some(ts_str) = d
            .get("metadata")
            .and_then(|m| m.get("quotaResetTimeStamp"))
            .and_then(serde_json::Value::as_str)
            && let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str.trim())
        {
            let ms = dt.timestamp_millis();
            if ms > now_ms {
                return Some(ms);
            }
        }
        if let Some(retry_str) = d.get("retryDelay").and_then(serde_json::Value::as_str) {
            let secs_str = retry_str.trim().trim_end_matches('s');
            if let Ok(sec) = secs_str.parse::<f64>()
                && sec > 0.0
            {
                let ms = now_ms + (sec * 1000.0).round() as i64;
                if ms > now_ms {
                    return Some(ms);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn escalating_model_failure_cooldowns_isolate_models_and_reset_on_success() {
        let store = Store::connect_in_memory().await.unwrap();
        let mihomo = Arc::new(
            MihomoController::connect_external(
                "http://127.0.0.1:0",
                "",
                std::env::temp_dir(),
                true,
                false,
            )
            .unwrap(),
        );
        let engine = AntigravityEngine::new(store, mihomo);
        let t0 = 1_700_000_000_000_i64;

        let (c1, u1, l1) = engine.record_model_failure("acct1", "gemini-3.8-flash", "500 boom", t0);
        assert_eq!(c1, 1);
        assert_eq!(u1, t0 + 5_000);
        assert_eq!(l1, "5s (failure #1)");

        let (c2, u2, l2) = engine.record_model_failure("acct1", "gemini-3.8-flash", "502 timeout", t0 + 6_000);
        assert_eq!(c2, 2);
        assert_eq!(u2, t0 + 6_000 + 30_000);
        assert_eq!(l2, "30s (failure #2)");

        // Other model on the SAME account must remain completely unaffected
        let fake_conn = ProviderConnection {
            id: "acct1".into(),
            provider_id: "antigravity".into(),
            name: "a1".into(),
            email: None,
            api_key: None,
            auth_data_json: None,
            quota_json: None,
            proxy_node_id: None,
            sticky_proxy_id: None,
            enabled: true,
            cooldown_until: None,
            backoff_level: 0,
            last_error: None,
            created_at: t0,
            updated_at: t0,
        };
        assert!(matches!(
            engine.check_account_model_availability(&fake_conn, "gemini-3.8-flash-medium", "gemini-3.8-flash", t0 + 10_000),
            AccountModelAvailability::FailureCooldown { .. }
        ));
        assert!(matches!(
            engine.check_account_model_availability(&fake_conn, "gemini-4-carbon-high", "gemini-4-carbon", t0 + 10_000),
            AccountModelAvailability::Ready
        ));

        let (c3, u3, l3) = engine.record_model_failure("acct1", "gemini-3.8-flash", "503 unavailable", t0 + 40_000);
        assert_eq!(c3, 3);
        assert_eq!(u3, t0 + 40_000 + 1_800_000);
        assert_eq!(l3, "30m (failure #3)");

        // Success resets the streak back to 0
        engine.clear_model_failures_and_locks("acct1", "gemini-3.8-flash");
        assert!(matches!(
            engine.check_account_model_availability(&fake_conn, "gemini-3.8-flash-medium", "gemini-3.8-flash", t0 + 41_000),
            AccountModelAvailability::Ready
        ));

        assert!(is_account_service_disabled(
            r#"{"error":{"code":403,"status":"PERMISSION_DENIED","message":"This service has been disabled in this account for violation of Terms of Service."}}"#
        ));

        let make_acct = |id: &str| ProviderConnection {
            id: id.into(),
            provider_id: "antigravity".into(),
            name: id.into(),
            email: None,
            api_key: None,
            auth_data_json: None,
            quota_json: None,
            proxy_node_id: None,
            sticky_proxy_id: None,
            enabled: true,
            cooldown_until: None,
            backoff_level: 0,
            last_error: None,
            created_at: t0,
            updated_at: t0,
        };
        let base = vec![make_acct("a1"), make_acct("a2"), make_acct("a3")];

        // Fallback preserves primary-first deterministic order
        let mut fb = base.clone();
        engine.apply_account_routing_strategy(&mut fb, "fallback");
        assert_eq!(fb.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["a1", "a2", "a3"]);

        // Round-robin rotates starting account cleanly across successive calls
        let mut rr1 = base.clone();
        engine.apply_account_routing_strategy(&mut rr1, "round-robin");
        let mut rr2 = base.clone();
        engine.apply_account_routing_strategy(&mut rr2, "round-robin");
        assert_ne!(rr1[0].id, rr2[0].id);

        // Random keeps all eligible accounts without dropping any
        let mut rnd = base;
        engine.apply_account_routing_strategy(&mut rnd, "random");
        assert_eq!(rnd.len(), 3);
    }
}
