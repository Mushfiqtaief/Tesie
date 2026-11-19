use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{
    GatewaySettings, IpCandidate, IpQuota, IpQuotaStatus, ProviderConfig, ProxyNode, Result,
    TesRouteError,
};

use crate::proxies::ProxiesRepo;
use crate::settings::SettingsRepo;
use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct QuotasRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IpQuotaWithProxies {
    #[serde(flatten)]
    pub quota: IpQuota,
    pub proxies: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateIpQuotaLimitsInput {
    pub window_duration_ms: Option<i64>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub max_requests_per_window: Option<Option<i64>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub max_tokens_per_window: Option<Option<i64>>,
}

impl QuotasRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn ensure_ip_quota_row(
        &self,
        ip_key: &str,
        settings: &GatewaySettings,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO ip_quotas (
                ip_key, provider_id, window_started_at, window_duration_ms,
                requests_in_window, max_requests_per_window,
                tokens_in_window, max_tokens_per_window,
                total_requests, total_prompt_tokens, total_completion_tokens,
                total_rate_limits, status, backoff_level, updated_at
            ) VALUES (?, 'opencode', ?, ?, 0, ?, 0, ?, 0, 0, 0, 0, 'active', 0, ?)",
        )
        .bind(ip_key)
        .bind(now)
        .bind(settings.default_free_ip_window_ms)
        .bind(settings.default_free_ip_max_requests)
        .bind(settings.default_free_ip_max_tokens)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn ensure_ip_quota_row_for_provider(
        &self,
        provider_id: &str,
        ip_key: &str,
        window_duration_ms: i64,
        max_requests: Option<i64>,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO ip_quotas (
                ip_key, provider_id, window_started_at, window_duration_ms,
                requests_in_window, max_requests_per_window,
                tokens_in_window, max_tokens_per_window,
                total_requests, total_prompt_tokens, total_completion_tokens,
                total_rate_limits, status, backoff_level, updated_at
            ) VALUES (?, ?, ?, ?, 0, ?, 0, NULL, 0, 0, 0, 0, 'active', 0, ?)",
        )
        .bind(ip_key)
        .bind(provider_id)
        .bind(now)
        .bind(window_duration_ms)
        .bind(max_requests)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn refresh_windows_and_cooldowns(&self, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE ip_quotas
             SET window_started_at = ?, requests_in_window = 0, tokens_in_window = 0, updated_at = ?
             WHERE ? - window_started_at >= window_duration_ms",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        // Queued (`status = 'cooldown'`) exit IPs NEVER auto-promote back into the live pool on a timer.
        // Only a live upstream probe or reset confirming 200 OK returns an IP to `'active'`.
        Ok(())
    }

    pub async fn select_candidates(
        &self,
        now: i64,
        strict_proxy_mode: bool,
    ) -> Result<Vec<IpCandidate>> {
        let mode = if strict_proxy_mode {
            "pool_strict"
        } else {
            "pool_with_direct"
        };
        self.select_candidates_for_mode(now, mode).await
    }

    pub async fn select_candidates_for_mode(
        &self,
        now: i64,
        proxy_mode: &str,
    ) -> Result<Vec<IpCandidate>> {
        self.select_healthy_candidates_for_mode(now, proxy_mode, |p| p.alive)
            .await
    }

    pub async fn select_healthy_candidates_for_mode<F>(
        &self,
        now: i64,
        proxy_mode: &str,
        is_healthy: F,
    ) -> Result<Vec<IpCandidate>>
    where
        F: Fn(&ProxyNode) -> bool,
    {
        self.refresh_windows_and_cooldowns(now).await?;
        let proxies = ProxiesRepo::new(self.pool.clone())
            .list_for_mode(proxy_mode)
            .await?;

        self.select_healthy_candidates_from_proxies(now, proxies, is_healthy)
            .await
    }

    pub async fn select_healthy_candidates_for_config<F>(
        &self,
        now: i64,
        cfg: &ProviderConfig,
        is_healthy: F,
    ) -> Result<Vec<IpCandidate>>
    where
        F: Fn(&ProxyNode) -> bool,
    {
        if cfg.provider_id == "ovh" {
            return self.select_healthy_ovh_candidates(now, cfg, is_healthy).await;
        }
        self.refresh_windows_and_cooldowns(now).await?;
        let proxies = ProxiesRepo::new(self.pool.clone())
            .list_for_provider_config(cfg)
            .await?;
        self.select_healthy_candidates_from_proxies(now, proxies, is_healthy)
            .await
    }

    pub async fn select_healthy_ovh_candidates<F>(
        &self,
        now: i64,
        cfg: &ProviderConfig,
        is_healthy: F,
    ) -> Result<Vec<IpCandidate>>
    where
        F: Fn(&ProxyNode) -> bool,
    {
        // 1. Auto-reset any expired cooldowns for OVH
        let _ = sqlx::query(
            "UPDATE ip_quotas
             SET status = 'active', cooldown_until = NULL, backoff_level = 0, updated_at = ?
             WHERE provider_id = 'ovh' AND status = 'cooldown' AND cooldown_until IS NOT NULL AND cooldown_until <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await;

        // 2. Auto-reset any expired windows for OVH (window_duration_ms = 60_000)
        let _ = sqlx::query(
            "UPDATE ip_quotas
             SET window_started_at = ?, requests_in_window = 0, tokens_in_window = 0, updated_at = ?
             WHERE provider_id = 'ovh' AND ? - window_started_at >= window_duration_ms",
        )
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await;

        let proxies = ProxiesRepo::new(self.pool.clone())
            .list_for_provider_config(cfg)
            .await?;

        for node in &proxies {
            let ovh_key = format!("ovh:{}", node.ip_key());
            self.ensure_ip_quota_row_for_provider("ovh", &ovh_key, 60_000, Some(2), now)
                .await?;
        }

        let all_quotas = self.list_ip_quotas_raw().await?;
        let quota_map: HashMap<String, IpQuota> = all_quotas
            .into_iter()
            .filter(|q| q.provider_id == "ovh")
            .map(|q| (q.ip_key.clone(), q))
            .collect();

        let mut seen_ips = HashSet::new();
        let mut candidates = Vec::new();
        for proxy in proxies {
            if !is_healthy(&proxy) {
                continue;
            }
            let ip_key = proxy.ip_key();
            let ovh_key = format!("ovh:{ip_key}");
            if !seen_ips.insert(ip_key.clone()) {
                continue;
            }
            let Some(quota) = quota_map.get(&ovh_key).cloned() else {
                continue;
            };
            if quota.status == IpQuotaStatus::Cooldown {
                if let Some(until) = quota.cooldown_until {
                    if until > now {
                        continue;
                    }
                }
            }
            if let Some(max_req) = quota.max_requests_per_window {
                if max_req > 0 && quota.requests_in_window >= max_req {
                    if now - quota.window_started_at < quota.window_duration_ms {
                        continue;
                    }
                }
            }
            candidates.push(IpCandidate { proxy, quota });
        }

        Ok(interleave_candidates_by_subnet(candidates))
    }

    async fn select_healthy_candidates_from_proxies<F>(
        &self,
        now: i64,
        proxies: Vec<ProxyNode>,
        is_healthy: F,
    ) -> Result<Vec<IpCandidate>>
    where
        F: Fn(&ProxyNode) -> bool,
    {
        let settings = SettingsRepo::new(self.pool.clone()).get_settings().await?;
        for node in &proxies {
            self.ensure_ip_quota_row(&node.ip_key(), &settings, now)
                .await?;
        }

        let all_quotas = self.list_ip_quotas_raw().await?;
        let quota_map: HashMap<String, IpQuota> = all_quotas
            .into_iter()
            .map(|q| (q.ip_key.clone(), q))
            .collect();

        let mut seen_ips = HashSet::new();
        let mut candidates = Vec::new();
        for proxy in proxies {
            if !is_healthy(&proxy) {
                continue;
            }
            let ip_key = proxy.ip_key();
            if !seen_ips.insert(ip_key.clone()) {
                continue;
            }
            let Some(quota) = quota_map.get(&ip_key).cloned() else {
                continue;
            };
            // Cooling IPs remain excluded until live upstream probe confirms real availability
            if quota.status == IpQuotaStatus::Cooldown {
                continue;
            }
            if let Some(max_req) = quota.max_requests_per_window
                && max_req > 0
                && quota.requests_in_window >= max_req
            {
                continue;
            }
            if let Some(max_tok) = quota.max_tokens_per_window
                && max_tok > 0
                && quota.tokens_in_window >= max_tok
            {
                continue;
            }
            candidates.push(IpCandidate { proxy, quota });
        }

        Ok(interleave_candidates_by_subnet(candidates))
    }

    pub async fn earliest_cooldown_until(
        &self,
        now: i64,
        strict_proxy_mode: bool,
    ) -> Result<Option<i64>> {
        let mode = if strict_proxy_mode {
            "pool_strict"
        } else {
            "pool_with_direct"
        };
        self.earliest_cooldown_until_for_mode(now, mode).await
    }

    pub async fn earliest_cooldown_until_for_mode(
        &self,
        now: i64,
        proxy_mode: &str,
    ) -> Result<Option<i64>> {
        let proxies = ProxiesRepo::new(self.pool.clone())
            .list_for_mode(proxy_mode)
            .await?;
        let active_keys: HashSet<String> = proxies.iter().map(|p| p.ip_key()).collect();
        let quotas = self.list_ip_quotas_raw().await?;
        let earliest = quotas
            .into_iter()
            .filter(|q| active_keys.contains(&q.ip_key))
            .filter_map(|q| q.cooldown_until)
            .filter(|&until| until > now)
            .min();
        Ok(earliest)
    }

    pub async fn earliest_cooldown_until_for_config(
        &self,
        now: i64,
        cfg: &ProviderConfig,
    ) -> Result<Option<i64>> {
        let proxies = ProxiesRepo::new(self.pool.clone())
            .list_for_provider_config(cfg)
            .await?;
        let active_keys: HashSet<String> = proxies.iter().map(|p| p.ip_key()).collect();
        let quotas = self.list_ip_quotas_raw().await?;
        let earliest = quotas
            .into_iter()
            .filter(|q| active_keys.contains(&q.ip_key))
            .filter_map(|q| q.cooldown_until)
            .filter(|&until| until > now)
            .min();
        Ok(earliest)
    }

    pub async fn record_success(
        &self,
        ip_key: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
        now: i64,
    ) -> Result<()> {
        let total = prompt_tokens.saturating_add(completion_tokens);
        sqlx::query(
            "UPDATE ip_quotas
             SET requests_in_window = requests_in_window + 1,
                 tokens_in_window = tokens_in_window + ?,
                 total_requests = total_requests + 1,
                 total_prompt_tokens = total_prompt_tokens + ?,
                 total_completion_tokens = total_completion_tokens + ?,
                 backoff_level = 0,
                 status = 'active',
                 cooldown_until = NULL,
                 last_error = NULL,
                 last_used_at = ?,
                 last_quota_checked_at = ?,
                 updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(total)
        .bind(prompt_tokens)
        .bind(completion_tokens)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn record_ovh_rate_limit(
        &self,
        ip_key: &str,
        cooldown_until: i64,
        error_msg: &str,
        now: i64,
    ) -> Result<()> {
        let ovh_key = if ip_key.starts_with("ovh:") {
            ip_key.to_string()
        } else {
            format!("ovh:{ip_key}")
        };
        sqlx::query(
            "UPDATE ip_quotas
             SET status = 'cooldown', cooldown_until = ?, total_rate_limits = total_rate_limits + 1,
                 last_error = ?, updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(cooldown_until)
        .bind(error_msg)
        .bind(now)
        .bind(&ovh_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn record_ovh_success(
        &self,
        ip_key: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
        remaining_reqs: Option<i64>,
        reset_sec: Option<i64>,
        now: i64,
    ) -> Result<()> {
        let ovh_key = if ip_key.starts_with("ovh:") {
            ip_key.to_string()
        } else {
            format!("ovh:{ip_key}")
        };
        let (status, cooldown_until) = if remaining_reqs == Some(0) {
            let dur = reset_sec.unwrap_or(60).max(1);
            ("cooldown", Some(now + dur * 1000))
        } else {
            ("active", None)
        };

        sqlx::query(
            "UPDATE ip_quotas
             SET requests_in_window = requests_in_window + 1,
                 total_requests = total_requests + 1,
                 total_prompt_tokens = total_prompt_tokens + ?,
                 total_completion_tokens = total_completion_tokens + ?,
                 status = ?,
                 cooldown_until = ?,
                 last_used_at = ?,
                 last_quota_checked_at = ?,
                 updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(prompt_tokens)
        .bind(completion_tokens)
        .bind(status)
        .bind(cooldown_until)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(&ovh_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn record_rate_limit(
        &self,
        ip_key: &str,
        explicit_cooldown_ms: Option<i64>,
        reason: &str,
        now: i64,
    ) -> Result<i64> {
        let current = self
            .get_ip_quota(ip_key)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("ip quota {ip_key} not found")))?;

        let new_level = (current.backoff_level + 1).min(4);
        // Queued nodes wait until the next live probe check interval (default 30s)
        // and stay in the queue until upstream confirms 200 OK.
        let cooldown_ms = explicit_cooldown_ms
            .filter(|&ms| ms > 0)
            .unwrap_or(30_000);
        let cooldown_until = now.saturating_add(cooldown_ms);
        let truncated_reason: String = reason.chars().take(512).collect();

        sqlx::query(
            "UPDATE ip_quotas
             SET status = 'cooldown',
                 cooldown_until = ?,
                 backoff_level = ?,
                 total_rate_limits = total_rate_limits + 1,
                 canonical_session_id = NULL,
                 session_created_at = NULL,
                 last_error = ?,
                 last_used_at = ?,
                 last_quota_checked_at = ?,
                 updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(cooldown_until)
        .bind(new_level)
        .bind(truncated_reason)
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(cooldown_until)
    }

    /// Keeps a queued IP in `cooldown` until the next `N`-second live probe check without incrementing rate-limit counters.
    pub async fn reschedule_queued_probe(
        &self,
        ip_key: &str,
        next_check_ms: i64,
        now: i64,
    ) -> Result<()> {
        let next_until = now.saturating_add(next_check_ms.max(5_000));
        sqlx::query(
            "UPDATE ip_quotas
             SET cooldown_until = ?,
                 last_quota_checked_at = ?,
                 updated_at = ?
             WHERE ip_key = ? AND status = 'cooldown'",
        )
        .bind(next_until)
        .bind(now)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    /// Marks an exit IP as confirmed available (`active`) after a lightweight 30s upstream
    /// minimal-request probe succeeds, without inflating user request/token counters.
    pub async fn record_probe_available(&self, ip_key: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE ip_quotas
             SET status = 'active',
                 cooldown_until = NULL,
                 backoff_level = 0,
                 last_error = NULL,
                 last_quota_checked_at = ?,
                 updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(now)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn get_or_mint_session<F>(
        &self,
        ip_key: &str,
        ttl_ms: i64,
        now: i64,
        mint: F,
    ) -> Result<String>
    where
        F: FnOnce() -> String,
    {
        if let Some(quota) = self.get_ip_quota(ip_key).await?
            && let (Some(sid), Some(created)) =
                (quota.canonical_session_id, quota.session_created_at)
            && !sid.is_empty()
            && now.saturating_sub(created) < ttl_ms
        {
            return Ok(sid);
        }

        let fresh = mint();
        sqlx::query(
            "UPDATE ip_quotas
             SET canonical_session_id = ?, session_created_at = ?, updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(&fresh)
        .bind(now)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(fresh)
    }

    pub async fn invalidate_session(&self, ip_key: &str, session_id: &str) -> Result<bool> {
        let now = now_ms();
        let res = sqlx::query(
            "UPDATE ip_quotas
             SET canonical_session_id = NULL, session_created_at = NULL, updated_at = ?
             WHERE ip_key = ? AND canonical_session_id = ?",
        )
        .bind(now)
        .bind(ip_key)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn get_ip_quota(&self, ip_key: &str) -> Result<Option<IpQuota>> {
        let row = sqlx::query(
            "SELECT ip_key, provider_id, window_started_at, window_duration_ms,
                    requests_in_window, max_requests_per_window,
                    tokens_in_window, max_tokens_per_window,
                    total_requests, total_prompt_tokens, total_completion_tokens,
                    total_rate_limits, status, cooldown_until, backoff_level,
                    canonical_session_id, session_created_at, last_error,
                    last_used_at, last_quota_checked_at, updated_at
             FROM ip_quotas
             WHERE ip_key = ?",
        )
        .bind(ip_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        row.as_ref().map(row_to_ip_quota).transpose()
    }

    pub async fn list_ip_quotas(&self) -> Result<Vec<IpQuotaWithProxies>> {
        let now = now_ms();
        self.refresh_windows_and_cooldowns(now).await?;
        let quotas = self.list_ip_quotas_raw().await?;
        let proxies = ProxiesRepo::new(self.pool.clone()).list_proxies().await?;
        let mut by_ip: HashMap<String, Vec<String>> = HashMap::new();
        for p in proxies {
            by_ip.entry(p.ip_key()).or_default().push(p.name);
        }
        Ok(quotas
            .into_iter()
            .filter_map(|quota| {
                // Keep only quotas tied to at least one existing proxy node or active history
                let names = by_ip.remove(&quota.ip_key).unwrap_or_default();
                if names.is_empty() && quota.ip_key.starts_with("unprobed:") {
                    return None;
                }
                Some(IpQuotaWithProxies {
                    quota,
                    proxies: names,
                })
            })
            .collect())
    }

    /// Returns IP quota rows strictly matching the provider's configured proxy selection
    /// (`"all"`, `"selected"` proxy IDs, or `"direct"` / `"none"`). Ensures a quota row exists
    /// for each selected proxy so Newly Selected or Single Proxy mode always reflects accurately.
    pub async fn list_ip_quotas_for_config(
        &self,
        cfg: &ProviderConfig,
    ) -> Result<Vec<IpQuotaWithProxies>> {
        let now = now_ms();
        self.refresh_windows_and_cooldowns(now).await?;
        let settings = SettingsRepo::new(self.pool.clone()).get_settings().await?;
        let selected_proxies = ProxiesRepo::new(self.pool.clone())
            .list_for_provider_config(cfg)
            .await?;

        let mut ordered_ip_keys: Vec<String> = Vec::new();
        let mut by_ip: HashMap<String, Vec<String>> = HashMap::new();
        for p in &selected_proxies {
            self.ensure_ip_quota_row(&p.ip_key(), &settings, now)
                .await?;
            let ip_key = p.ip_key();
            if !by_ip.contains_key(&ip_key) {
                ordered_ip_keys.push(ip_key.clone());
            }
            by_ip.entry(ip_key).or_default().push(p.name.clone());
        }

        let quotas = self.list_ip_quotas_raw().await?;
        let mut quota_map: HashMap<String, IpQuota> = quotas
            .into_iter()
            .map(|q| (q.ip_key.clone(), q))
            .collect();

        let mut out = Vec::with_capacity(ordered_ip_keys.len());
        for ip_key in ordered_ip_keys {
            if let Some(quota) = quota_map.remove(&ip_key) {
                let proxies = by_ip.remove(&ip_key).unwrap_or_default();
                out.push(IpQuotaWithProxies { quota, proxies });
            }
        }
        Ok(out)
    }

    async fn list_ip_quotas_raw(&self) -> Result<Vec<IpQuota>> {
        let rows = sqlx::query(
            "SELECT ip_key, provider_id, window_started_at, window_duration_ms,
                    requests_in_window, max_requests_per_window,
                    tokens_in_window, max_tokens_per_window,
                    total_requests, total_prompt_tokens, total_completion_tokens,
                    total_rate_limits, status, cooldown_until, backoff_level,
                    canonical_session_id, session_created_at, last_error,
                    last_used_at, last_quota_checked_at, updated_at
             FROM ip_quotas
             ORDER BY ip_key ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        rows.iter().map(row_to_ip_quota).collect()
    }

    pub async fn reset_all_cooling(&self, now: i64) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE ip_quotas
             SET window_started_at = ?, requests_in_window = 0, tokens_in_window = 0,
                 status = 'active', cooldown_until = NULL, backoff_level = 0,
                 canonical_session_id = NULL, session_created_at = NULL,
                 last_error = NULL, last_quota_checked_at = NULL, updated_at = ?
             WHERE status = 'cooldown' OR cooldown_until IS NOT NULL",
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(res.rows_affected())
    }

    pub async fn reset_ip_quota(&self, ip_key: &str, now: i64) -> Result<IpQuota> {
        let res = sqlx::query(
            "UPDATE ip_quotas
             SET window_started_at = ?, requests_in_window = 0, tokens_in_window = 0,
                 status = 'active', cooldown_until = NULL, backoff_level = 0,
                 canonical_session_id = NULL, session_created_at = NULL,
                 last_error = NULL, last_quota_checked_at = NULL, updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(now)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        if res.rows_affected() == 0 {
            return Err(TesRouteError::NotFound(format!(
                "ip quota {ip_key} not found"
            )));
        }

        self.get_ip_quota(ip_key)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("ip quota {ip_key} not found")))
    }

    pub async fn update_ip_quota_limits(
        &self,
        ip_key: &str,
        input: UpdateIpQuotaLimitsInput,
        now: i64,
    ) -> Result<IpQuota> {
        let existing = self
            .get_ip_quota(ip_key)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("ip quota {ip_key} not found")))?;

        let window_duration_ms = input
            .window_duration_ms
            .filter(|&w| w > 0)
            .unwrap_or(existing.window_duration_ms);
        let max_requests = input
            .max_requests_per_window
            .map(|opt| opt.filter(|&v| v > 0))
            .unwrap_or(existing.max_requests_per_window);
        let max_tokens = input
            .max_tokens_per_window
            .map(|opt| opt.filter(|&v| v > 0))
            .unwrap_or(existing.max_tokens_per_window);

        sqlx::query(
            "UPDATE ip_quotas
             SET window_duration_ms = ?, max_requests_per_window = ?, max_tokens_per_window = ?,
                 updated_at = ?
             WHERE ip_key = ?",
        )
        .bind(window_duration_ms)
        .bind(max_requests)
        .bind(max_tokens)
        .bind(now)
        .bind(ip_key)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_ip_quota(ip_key)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("ip quota {ip_key} not found")))
    }
}

fn row_to_ip_quota(row: &sqlx::sqlite::SqliteRow) -> Result<IpQuota> {
    let status_raw: String = row.try_get("status").map_err(map_sqlx_err)?;
    Ok(IpQuota {
        ip_key: row.try_get("ip_key").map_err(map_sqlx_err)?,
        provider_id: row.try_get("provider_id").map_err(map_sqlx_err)?,
        window_started_at: row.try_get("window_started_at").map_err(map_sqlx_err)?,
        window_duration_ms: row.try_get("window_duration_ms").map_err(map_sqlx_err)?,
        requests_in_window: row.try_get("requests_in_window").map_err(map_sqlx_err)?,
        max_requests_per_window: row
            .try_get("max_requests_per_window")
            .map_err(map_sqlx_err)?,
        tokens_in_window: row.try_get("tokens_in_window").map_err(map_sqlx_err)?,
        max_tokens_per_window: row.try_get("max_tokens_per_window").map_err(map_sqlx_err)?,
        total_requests: row.try_get("total_requests").map_err(map_sqlx_err)?,
        total_prompt_tokens: row.try_get("total_prompt_tokens").map_err(map_sqlx_err)?,
        total_completion_tokens: row
            .try_get("total_completion_tokens")
            .map_err(map_sqlx_err)?,
        total_rate_limits: row.try_get("total_rate_limits").map_err(map_sqlx_err)?,
        status: IpQuotaStatus::parse(&status_raw),
        cooldown_until: row.try_get("cooldown_until").map_err(map_sqlx_err)?,
        backoff_level: row.try_get("backoff_level").map_err(map_sqlx_err)?,
        canonical_session_id: row.try_get("canonical_session_id").map_err(map_sqlx_err)?,
        session_created_at: row.try_get("session_created_at").map_err(map_sqlx_err)?,
        last_error: row.try_get("last_error").map_err(map_sqlx_err)?,
        last_used_at: row.try_get("last_used_at").map_err(map_sqlx_err)?,
        last_quota_checked_at: row.try_get("last_quota_checked_at").map_err(map_sqlx_err)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx_err)?,
    })
}

fn ipv4_slash24_bucket(ip_key: &str) -> String {
    let mut parts = ip_key.split('.');
    if let (Some(a), Some(b), Some(c), Some(d)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
        && parts.next().is_none()
        && a.parse::<u8>().is_ok()
        && b.parse::<u8>().is_ok()
        && c.parse::<u8>().is_ok()
        && d.parse::<u8>().is_ok()
    {
        return format!("{a}.{b}.{c}");
    }
    ip_key.to_string()
}

/// Interleaves healthy candidates across distinct IPv4 `/24` subnets so Round-Robin selection
/// and failover immediately hop across separate network blocks instead of walking sequential
/// siblings inside a single rate-limited `/24` datacenter subnet.
fn interleave_candidates_by_subnet(candidates: Vec<IpCandidate>) -> Vec<IpCandidate> {
    if candidates.len() <= 2 {
        return candidates;
    }

    let mut bucket_index: HashMap<String, usize> = HashMap::new();
    let mut buckets: Vec<VecDeque<IpCandidate>> = Vec::new();

    for cand in candidates {
        let key = ipv4_slash24_bucket(&cand.quota.ip_key);
        if let Some(&idx) = bucket_index.get(&key) {
            buckets[idx].push_back(cand);
        } else {
            let idx = buckets.len();
            bucket_index.insert(key, idx);
            let mut q = VecDeque::new();
            q.push_back(cand);
            buckets.push(q);
        }
    }

    if buckets.len() <= 1 {
        return buckets.into_iter().flatten().collect();
    }

    let total: usize = buckets.iter().map(VecDeque::len).sum();
    let mut out = Vec::with_capacity(total);
    loop {
        let mut progressed = false;
        for bucket in &mut buckets {
            if let Some(cand) = bucket.pop_front() {
                out.push(cand);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ipv4_slash24_bucket_cleanly() {
        assert_eq!(ipv4_slash24_bucket("103.17.36.115"), "103.17.36");
        assert_eq!(ipv4_slash24_bucket("163.53.180.59"), "163.53.180");
        assert_eq!(ipv4_slash24_bucket("direct"), "direct");
    }
}
