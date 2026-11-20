pub mod cli_tools;
pub mod system;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tesie_core::{
    compress_text_detailed, default_routing_strategy_for, normalize_routing_strategy,
    style_prompt_for, ApiKeyRecord, CustomProvider, IpQuotaStatus, ProviderConnection, ProxyNode,
    TesRouteError, TokenSaverConfig,
};
use tesie_providers::{
    discover_favicon_data_uri, fetch_custom_provider_models, ModelCatalogCache, OpenCodeEngine,
};
use tesie_proxy::{MihomoController, RelayRequest};
use tesie_store::combos::{CreateComboInput, UpdateComboInput};
use tesie_store::connections::{CreateConnectionInput, UpdateConnectionInput};
use tesie_store::custom_providers::{CreateCustomProviderInput, UpdateCustomProviderInput};
use tesie_store::keys::{CreateApiKeyInput, UpdateApiKeyInput};
use tesie_store::provider_configs::UpdateProviderConfigInput;
use tesie_store::proxies::{CreateProxyInput, UpdateProxyInput};
use tesie_store::quotas::UpdateIpQuotaLimitsInput;
use tesie_store::settings::UpdateSettingsInput;
use tesie_store::usage::LogQueryFilter;
use tesie_store::{now_ms, Store};

#[derive(Clone)]
pub struct AdminState {
    pub store: Store,
    pub mihomo: Arc<MihomoController>,
    pub engine: Arc<OpenCodeEngine>,
    pub catalog: Arc<ModelCatalogCache>,
    pub zen_base_url: String,
    pub fetch_live_models: bool,
    pub admin_token: Option<String>,
    /// When true, requests are rejected while no credential is configured, instead of
    /// falling through to wide-open access. The container image sets this.
    pub require_admin_auth: bool,
    /// Failed-unlock tracking, keyed by client IP.
    pub auth_limiter: Arc<AuthAttemptLimiter>,
}

pub fn admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/auth/status", get(get_auth_status))
        .route("/system/autostart", post(update_system_autostart))
        .route("/topology", get(get_topology))
        .route("/models", get(list_models_admin))
        .route("/providers", get(list_providers))
        .route(
            "/providers/{id}/config",
            put(update_provider_config).patch(update_provider_config),
        )
        .route("/providers/{id}/test", post(test_provider))
        .route("/providers/{id}/fetch-models", post(fetch_provider_models_handler))
        .route(
            "/custom-providers",
            get(list_custom_providers).post(create_custom_provider),
        )
        .route(
            "/custom-providers/favicon",
            get(discover_custom_provider_favicon),
        )
        .route(
            "/custom-providers/fetch-models",
            post(fetch_custom_provider_models_preview),
        )
        .route(
            "/custom-providers/{id}",
            put(update_custom_provider)
                .patch(update_custom_provider)
                .delete(delete_custom_provider),
        )
        .route(
            "/custom-providers/{id}/refresh-favicon",
            post(refresh_custom_provider_favicon),
        )
        .route(
            "/custom-providers/{id}/fetch-models",
            post(fetch_custom_provider_models_handler),
        )
        .route("/combos", get(list_combos).post(create_combo))
        .route("/combos/{id}", put(update_combo).delete(delete_combo))
        .route("/cli-tools/status", get(cli_tools::get_all_cli_statuses))
        .route(
            "/cli-tools/opencode",
            get(cli_tools::get_opencode_settings)
                .post(cli_tools::apply_opencode_settings)
                .delete(cli_tools::reset_opencode_settings),
        )
        .route(
            "/cli-tools/claude",
            get(cli_tools::get_claude_settings)
                .post(cli_tools::apply_claude_settings)
                .delete(cli_tools::reset_claude_settings),
        )
        .route(
            "/cli-tools/codex",
            get(cli_tools::get_codex_settings)
                .post(cli_tools::apply_codex_settings)
                .delete(cli_tools::reset_codex_settings),
        )
        .route("/proxies", get(list_proxies).post(create_proxy))
        .route("/proxies/import", post(import_proxies))
        .route("/proxies/batch-delete", post(batch_delete_proxies))
        .route("/proxies/normalize-names", post(normalize_proxy_names))
        .route("/proxies/test-config", put(update_proxy_test_config))
        .route("/proxies/test", post(test_all_proxies))
        .route("/proxies/probe", post(probe_all_proxies))
        .route("/proxies/{id}", patch(update_proxy).delete(delete_proxy))
        .route("/proxies/{id}/test", post(test_single_proxy))
        .route("/proxies/{id}/probe", post(probe_single_proxy))
        .route("/quotas/ips", get(list_ip_quotas))
        .route("/quotas/ips/check", post(check_ip_quotas_live))
        .route("/quotas/ips/reset-all", post(reset_all_cooling_quotas))
        .route("/quotas/ips/{ip_key}", patch(update_ip_quota_limits))
        .route("/quotas/ips/{ip_key}/reset", post(reset_ip_quota))
        .route("/quotas/providers", get(get_provider_quotas))
        .route("/antigravity/accounts", get(list_antigravity_accounts))
        .route("/antigravity/oauth/start", post(start_antigravity_oauth))
        .route("/antigravity/oauth/poll/{state}", get(poll_antigravity_oauth))
        .route("/antigravity/oauth/complete", post(complete_antigravity_oauth))
        .route("/antigravity/accounts/import", post(import_antigravity_account))
        .route("/antigravity/quotas/refresh", post(refresh_antigravity_quotas))
        .route(
            "/antigravity/accounts/{id}/refresh-quota",
            post(refresh_single_antigravity_quota),
        )
        .route("/connections", get(list_connections).post(create_connection))
        .route(
            "/connections/{id}",
            patch(update_connection).delete(delete_connection),
        )
        .route("/keys", get(list_keys).post(create_key))
        .route("/keys/{id}", patch(update_key).delete(delete_key))
        .route("/usage", get(get_usage))
        .route("/logs", get(query_logs).delete(clear_logs))
        .route("/settings", get(get_settings).put(update_settings))
        .route("/backup/export", get(export_backup))
        .route("/backup/import", post(import_backup))
        .route(
            "/maintenance/reset-cooldowns",
            post(reset_all_cooldowns_maintenance),
        )
        .route(
            "/token-saver",
            get(get_token_saver)
                .put(update_token_saver)
                .patch(update_token_saver),
        )
        .route("/token-saver/test", post(test_token_saver))
        .route(
            "/token-saver/stats",
            axum::routing::delete(clear_token_saver_stats),
        )
        .route(
            "/bridge",
            get(get_bridge_config)
                .put(update_bridge_config)
                .patch(update_bridge_config),
        )
        .route("/bridge/stats", get(get_bridge_stats))
        .route("/bridge/simulate-event", post(simulate_bridge_event))
        .layer(middleware::from_fn_with_state(state.clone(), require_admin_auth))
        .with_state(state)
}

fn extract_provided_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.trim()
                .strip_prefix("Bearer ")
                .or_else(|| s.trim().strip_prefix("bearer "))
        })
        .or_else(|| headers.get("x-admin-token").and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

const MAX_FAILED_UNLOCK_ATTEMPTS: u32 = 5;
const UNLOCK_LOCKOUT_MS: i64 = 60_000;

/// In-memory failed-unlock tracker. Keyed by client address so one attacker cannot consume
/// the attempts of another, and pruned on every check so it cannot grow without bound.
///
/// Semantics: the first `MAX_FAILED_UNLOCK_ATTEMPTS` bad credentials each get a plain 401.
/// Once that many have been recorded, every further attempt from the same client — including
/// one carrying the correct credential — is refused until the lockout window expires.
#[derive(Default)]
pub struct AuthAttemptLimiter {
    inner: std::sync::Mutex<std::collections::HashMap<String, AttemptWindow>>,
}

#[derive(Clone, Copy)]
struct AttemptWindow {
    failures: u32,
    window_started_at: i64,
    seen_at: i64,
}

impl AuthAttemptLimiter {
    /// Returns the remaining lockout in milliseconds, or 0 when the caller may try again.
    pub fn retry_after_ms(&self, key: &str) -> i64 {
        let now = now_ms();
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.retain(|_, w| now - w.seen_at < UNLOCK_LOCKOUT_MS);

        let Some(w) = guard.get_mut(key) else {
            return 0;
        };
        if w.failures < MAX_FAILED_UNLOCK_ATTEMPTS {
            return 0;
        }
        let elapsed = now - w.window_started_at;
        if elapsed >= UNLOCK_LOCKOUT_MS {
            // Window expired: forget the failures so the client starts clean.
            guard.remove(key);
            return 0;
        }
        UNLOCK_LOCKOUT_MS - elapsed
    }

    pub fn record_failure(&self, key: &str) {
        let now = now_ms();
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let window = guard.entry(key.to_string()).or_insert(AttemptWindow {
            failures: 0,
            window_started_at: now,
            seen_at: now,
        });
        if now - window.window_started_at >= UNLOCK_LOCKOUT_MS {
            window.failures = 0;
            window.window_started_at = now;
        }
        window.failures = window.failures.saturating_add(1);
        window.seen_at = now;
    }

    pub fn record_success(&self, key: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.remove(key);
        }
    }
}

fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// Resolves the credential the request must present: the env token wins, then the stored
/// dashboard password. `require_admin_auth` decides what happens when neither exists.
async fn require_admin_auth(
    State(state): State<AdminState>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> std::result::Result<Response, Response> {
    // The status probe must stay reachable so the SPA can tell "locked" from "offline".
    if req.uri().path().ends_with("/auth/status") {
        return Ok(next.run(req).await);
    }

    let provided = extract_provided_token(&headers);
    let key = client_key(&headers);

    let env_token = state
        .admin_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let password_set = state
        .store
        .settings()
        .has_admin_password()
        .await
        .unwrap_or(false);

    if env_token.is_none() && !password_set {
        if state.require_admin_auth {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "admin auth is required but no ADMIN_TOKEN or dashboard password is configured"
                })),
            )
                .into_response());
        }
        return Ok(next.run(req).await);
    }

    let retry_after = state.auth_limiter.retry_after_ms(&key);
    if retry_after > 0 {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "too many failed unlock attempts, try again shortly",
                "retry_after_ms": retry_after,
            })),
        )
            .into_response());
    }

    let Some(provided) = provided else {
        return Err(unauthorized());
    };

    // The env token short-circuits password verification, but a mismatch must still be
    // counted so an attacker cannot probe credentials for free.
    if let Some(expected) = env_token
        && ct_eq_str(expected, provided)
    {
        state.auth_limiter.record_success(&key);
        return Ok(next.run(req).await);
    }

    if password_set
        && state
            .store
            .settings()
            .verify_admin_password(provided)
            .await
            .unwrap_or(false)
    {
        state.auth_limiter.record_success(&key);
        return Ok(next.run(req).await);
    }

    // Record the failure, then reject. The next request from this client is gated by the
    // `retry_after_ms` check above, so a client gets exactly MAX plain rejections.
    state.auth_limiter.record_failure(&key);
    Err(unauthorized())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "unauthorized admin request" })),
    )
        .into_response()
}

/// Length-checked, branch-free string comparison for the env token path.
fn ct_eq_str(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

async fn get_auth_status(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, Response> {
    let env_token_active = state
        .admin_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    let password_set = state
        .store
        .settings()
        .has_admin_password()
        .await
        .unwrap_or(false);
    let protected = env_token_active || password_set || state.require_admin_auth;

    let provided = extract_provided_token(&headers);
    let authenticated = if !protected {
        true
    } else {
        let env_match = state
            .admin_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .zip(provided)
            .is_some_and(|(expected, given)| ct_eq_str(expected, given));
        env_match
            || match provided {
                Some(given) if password_set => state
                    .store
                    .settings()
                    .verify_admin_password(given)
                    .await
                    .unwrap_or(false),
                _ => false,
            }
    };

    Ok(Json(json!({
        "protected": protected,
        "authenticated": authenticated,
        "env_token_active": env_token_active,
        "password_set": password_set,
    })))
}

async fn get_status(State(state): State<AdminState>) -> std::result::Result<Json<Value>, Response> {
    let core_health = state.mihomo.health().await.ok();
    let proxies = state.store.proxies().list_proxies().await.map_err(err_resp)?;
    let oc_cfg = state
        .store
        .provider_configs()
        .get_or_default("opencode")
        .await
        .map_err(err_resp)?;
    let quotas = state
        .store
        .quotas()
        .list_ip_quotas_for_config(&oc_cfg)
        .await
        .map_err(err_resp)?;

    let active_ips = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Active)
        .count();
    let cooling_ips = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Cooldown)
        .count();

    let data_dir = state
        .mihomo
        .work_dir()
        .parent()
        .unwrap_or_else(|| state.mihomo.work_dir())
        .to_string_lossy()
        .to_string();
    let auto_start_enabled = system::is_autostart_enabled();

    Ok(Json(json!({
        "status": "ok",
        "app_name": "Tesie",
        "core_name": "TesieCore",
        "core_healthy": core_health.is_some(),
        "core": core_health,
        "mihomo_healthy": core_health.is_some(),
        "mihomo": core_health,
        "data_dir": data_dir,
        "auto_start_enabled": auto_start_enabled,
        "total_proxies": proxies.len(),
        "active_ips": active_ips,
        "cooling_ips": cooling_ips
    })))
}

#[derive(Deserialize)]
struct UpdateAutostartInput {
    enabled: bool,
}

async fn update_system_autostart(
    State(state): State<AdminState>,
    Json(input): Json<UpdateAutostartInput>,
) -> std::result::Result<Json<Value>, Response> {
    let data_dir = state
        .mihomo
        .work_dir()
        .parent()
        .unwrap_or_else(|| state.mihomo.work_dir());
    let daemon_exe = system::resolve_daemon_exe_path(data_dir);
    let enabled = system::set_autostart_enabled(input.enabled, &daemon_exe)
        .map_err(|msg| err_resp(TesRouteError::Internal(msg)))?;
    Ok(Json(json!({
        "auto_start_enabled": enabled,
        "daemon_path": daemon_exe.to_string_lossy(),
        "data_dir": data_dir.to_string_lossy()
    })))
}

#[derive(Serialize)]
struct ProxyWithQuota {
    #[serde(flatten)]
    proxy: ProxyNode,
    ip_key: String,
    quota_status: Option<IpQuotaStatus>,
    window_started_at: Option<i64>,
    window_duration_ms: i64,
    requests_in_window: i64,
    max_requests_per_window: Option<i64>,
    tokens_in_window: i64,
    max_tokens_per_window: Option<i64>,
    total_requests: i64,
    total_prompt_tokens: i64,
    total_completion_tokens: i64,
    total_rate_limits: i64,
    cooldown_until: Option<i64>,
    backoff_level: i64,
    canonical_session_id: Option<String>,
    last_error: Option<String>,
    last_used_at: Option<i64>,
}

async fn list_models_admin(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let models = state
        .catalog
        .list_models(
            &state.store,
            Some(&state.mihomo),
            &state.zen_base_url,
            state.fetch_live_models,
        )
        .await;
    Ok(Json(json!({ "object": "list", "data": models })))
}

async fn list_proxies(State(state): State<AdminState>) -> std::result::Result<Json<Value>, Response> {
    let settings = state
        .store
        .settings()
        .get_settings()
        .await
        .map_err(err_resp)?;
    let proxies = state.store.proxies().list_proxies().await.map_err(err_resp)?;
    let mihomo_states = state.mihomo.fetch_proxy_states().await.ok();
    let quotas = state
        .store
        .quotas()
        .list_ip_quotas()
        .await
        .map_err(err_resp)?;

    let mut by_ip = std::collections::HashMap::new();
    for item in quotas {
        by_ip.insert(item.quota.ip_key.clone(), item.quota);
    }

    let items: Vec<ProxyWithQuota> = proxies
        .into_iter()
        .map(|mut proxy| {
            // Merge live Mihomo state as the authoritative source of truth
            if let Some(st) = mihomo_states.as_ref().and_then(|m| m.get(&proxy.name)) {
                proxy.alive = st.alive;
                if st.alive {
                    if let Some(d) = st.delay_ms {
                        proxy.latency_ms = Some(d);
                        proxy.last_delay_ms = Some(d);
                    }
                } else {
                    proxy.latency_ms = None;
                    proxy.last_delay_ms = None;
                }
            }
            let ip_key = proxy.ip_key();
            let q = by_ip.get(&ip_key);
            ProxyWithQuota {
                proxy,
                ip_key,
                quota_status: q.map(|x| x.status),
                window_started_at: q.map(|x| x.window_started_at),
                window_duration_ms: q.map(|x| x.window_duration_ms).unwrap_or(3_600_000),
                requests_in_window: q.map(|x| x.requests_in_window).unwrap_or(0),
                max_requests_per_window: q.and_then(|x| x.max_requests_per_window),
                tokens_in_window: q.map(|x| x.tokens_in_window).unwrap_or(0),
                max_tokens_per_window: q.and_then(|x| x.max_tokens_per_window),
                total_requests: q.map(|x| x.total_requests).unwrap_or(0),
                total_prompt_tokens: q.map(|x| x.total_prompt_tokens).unwrap_or(0),
                total_completion_tokens: q.map(|x| x.total_completion_tokens).unwrap_or(0),
                total_rate_limits: q.map(|x| x.total_rate_limits).unwrap_or(0),
                cooldown_until: q.and_then(|x| x.cooldown_until),
                backoff_level: q.map(|x| x.backoff_level).unwrap_or(0),
                canonical_session_id: q.and_then(|x| x.canonical_session_id.clone()),
                last_error: q.and_then(|x| x.last_error.clone()),
                last_used_at: q.and_then(|x| x.last_used_at),
            }
        })
        .collect();

    Ok(Json(json!({
        "proxies": items,
        "test_interval_sec": settings.proxy_test_interval_sec,
        "last_auto_test_at": state.mihomo.last_auto_test_at()
    })))
}

async fn create_proxy(
    State(state): State<AdminState>,
    Json(input): Json<CreateProxyInput>,
) -> std::result::Result<(StatusCode, Json<Value>), Response> {
    if let Some(raw_url) = input
        .proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        tesie_proxy::config_gen::validate_proxy_url(raw_url).map_err(err_resp)?;
    }
    if let Some(raw_json) = input
        .raw_outbound_json
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let val: Value = serde_json::from_str(raw_json).map_err(|e| {
            err_resp(TesRouteError::BadRequest(format!(
                "invalid raw_outbound_json: {e}"
            )))
        })?;
        if !val.is_object() {
            return Err(err_resp(TesRouteError::BadRequest(
                "raw_outbound_json must be a JSON object".into(),
            )));
        }
    }

    let node = state
        .store
        .proxies()
        .upsert_proxy(input)
        .await
        .map_err(err_resp)?;
    state
        .mihomo
        .reload_from_store(&state.store)
        .await
        .map_err(err_resp)?;
    Ok((StatusCode::CREATED, Json(json!({ "proxy": node }))))
}

#[derive(Deserialize)]
struct ImportProxiesRequest {
    urls_text: String,
}

async fn import_proxies(
    State(state): State<AdminState>,
    Json(req): Json<ImportProxiesRequest>,
) -> std::result::Result<Json<Value>, Response> {
    let mut valid_lines = Vec::new();
    let mut skipped_lines = Vec::new();
    for raw_line in req.urls_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match tesie_proxy::config_gen::validate_proxy_url(line) {
            Ok(()) => valid_lines.push(line),
            Err(err) => skipped_lines.push(format!("{line}: {err}")),
        }
    }

    let filtered_text = valid_lines.join("\n");
    let nodes = state
        .store
        .proxies()
        .bulk_import_proxy_urls(&filtered_text)
        .await
        .map_err(err_resp)?;
    state
        .mihomo
        .reload_from_store(&state.store)
        .await
        .map_err(err_resp)?;

    Ok(Json(json!({
        "imported": nodes.len(),
        "skipped": skipped_lines,
        "proxies": nodes
    })))
}

#[derive(Deserialize)]
struct UpdateProxyTestConfigRequest {
    test_interval_sec: u64,
}

async fn update_proxy_test_config(
    State(state): State<AdminState>,
    Json(req): Json<UpdateProxyTestConfigRequest>,
) -> std::result::Result<Json<Value>, Response> {
    let settings = state
        .store
        .settings()
        .update_settings(UpdateSettingsInput {
            proxy_test_interval_sec: Some(req.test_interval_sec.clamp(5, 3600)),
            ..Default::default()
        })
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "test_interval_sec": settings.proxy_test_interval_sec,
        "last_auto_test_at": state.mihomo.last_auto_test_at()
    })))
}

async fn update_proxy(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateProxyInput>,
) -> std::result::Result<Json<Value>, Response> {
    let node = state
        .store
        .proxies()
        .update_proxy(&id, input)
        .await
        .map_err(err_resp)?;
    state
        .mihomo
        .reload_from_store(&state.store)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "proxy": node })))
}

async fn delete_proxy(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<StatusCode, Response> {
    state
        .store
        .proxies()
        .delete_proxy(&id)
        .await
        .map_err(err_resp)?;
    let _ = state
        .store
        .provider_configs()
        .prune_deleted_proxy_ids(std::slice::from_ref(&id))
        .await;
    state
        .mihomo
        .reload_from_store(&state.store)
        .await
        .map_err(err_resp)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct BatchDeleteProxiesRequest {
    ids: Vec<String>,
}

async fn batch_delete_proxies(
    State(state): State<AdminState>,
    Json(req): Json<BatchDeleteProxiesRequest>,
) -> std::result::Result<Json<Value>, Response> {
    let deleted = state
        .store
        .proxies()
        .delete_proxies_batch(&req.ids)
        .await
        .map_err(err_resp)?;
    let _ = state
        .store
        .provider_configs()
        .prune_deleted_proxy_ids(&req.ids)
        .await;
    state
        .mihomo
        .reload_from_store(&state.store)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "deleted": deleted })))
}

async fn normalize_proxy_names(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let renamed = state
        .store
        .proxies()
        .normalize_all_external_names_to_nodes()
        .await
        .map_err(err_resp)?;
    state
        .mihomo
        .reload_from_store(&state.store)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "renamed": renamed })))
}

#[derive(Deserialize, Default)]
struct ProbeQuery {
    url: Option<String>,
}

async fn test_all_proxies(
    State(state): State<AdminState>,
    Query(q): Query<ProbeQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let states = state
        .mihomo
        .test_all_delays(&state.store, q.url.as_deref())
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "tested": states,
        "last_auto_test_at": state.mihomo.last_auto_test_at()
    })))
}

async fn test_single_proxy(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Query(q): Query<ProbeQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let node = state
        .store
        .proxies()
        .get_proxy(&id)
        .await
        .map_err(err_resp)?
        .ok_or_else(|| err_resp(TesRouteError::NotFound(format!("proxy {id} not found"))))?;

    let delay_res = state
        .mihomo
        .test_proxy_delay(&node.name, q.url.as_deref(), 5000)
        .await;
    let delay = match delay_res {
        Ok(d) => d,
        Err(err) if err.to_string().contains("404") || err.to_string().contains("Resource not found") => {
            // Self-heal if TesieCore's running proxy list fell out of sync with SQLite
            let _ = state.mihomo.reload_from_store(&state.store).await;
            state
                .mihomo
                .test_proxy_delay(&node.name, q.url.as_deref(), 5000)
                .await
                .map_err(|e| err_resp(TesRouteError::ProxyRelay(format!("Proxy '{}' check failed: {e}", node.name))))?
        }
        Err(err) => {
            return Err(err_resp(TesRouteError::ProxyRelay(format!(
                "Proxy '{}' check failed: {err}",
                node.name
            ))));
        }
    };
    let alive = delay.is_some();
    let now = now_ms();

    state
        .store
        .proxies()
        .sync_mihomo_delay(&node.name, alive, delay, now)
        .await
        .map_err(err_resp)?;

    if alive && node.egress_ip.is_none() && q.url.is_none() {
        if let Ok(probe) = state.mihomo.probe_proxy(&node.name, None).await {
            let _ = state
                .store
                .proxies()
                .update_proxy_probe(&node.id, &probe.egress_ip, delay.unwrap_or(probe.delay_ms), now)
                .await;
        }
    }

    let updated = state
        .store
        .proxies()
        .get_proxy(&node.id)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "proxy": updated,
        "alive": alive,
        "latency_ms": delay
    })))
}

async fn probe_all_proxies(
    State(state): State<AdminState>,
    Query(q): Query<ProbeQuery>,
) -> std::result::Result<Json<Value>, Response> {
    if q.url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
        let results = state
            .mihomo
            .probe_all_enabled(&state.store, q.url.as_deref())
            .await
            .map_err(err_resp)?;
        return Ok(Json(json!({ "probed": results })));
    }
    let states = state
        .mihomo
        .test_all_delays(&state.store, None)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "probed": states,
        "last_auto_test_at": state.mihomo.last_auto_test_at()
    })))
}

async fn probe_single_proxy(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Query(q): Query<ProbeQuery>,
) -> std::result::Result<Json<Value>, Response> {
    if q.url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
        let node = state
            .store
            .proxies()
            .get_proxy(&id)
            .await
            .map_err(err_resp)?
            .ok_or_else(|| err_resp(TesRouteError::NotFound(format!("proxy {id} not found"))))?;

        let probe = state
            .mihomo
            .probe_proxy(&node.name, q.url.as_deref())
            .await
            .map_err(err_resp)?;
        let updated = state
            .store
            .proxies()
            .update_proxy_probe(&node.id, &probe.egress_ip, probe.delay_ms, now_ms())
            .await
            .map_err(err_resp)?;
        return Ok(Json(json!({ "proxy": updated, "probe": probe })));
    }
    test_single_proxy(State(state), Path(id), Query(q)).await
}

async fn list_ip_quotas(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let oc_cfg = state
        .store
        .provider_configs()
        .get_or_default("opencode")
        .await
        .map_err(err_resp)?;
    let quotas = state
        .store
        .quotas()
        .list_ip_quotas_for_config(&oc_cfg)
        .await
        .map_err(err_resp)?;
    let probe_status = state.engine.quota_probe_status().await;
    Ok(Json(json!({
        "ip_quotas": quotas,
        "proxy_mode": oc_cfg.proxy_mode,
        "selected_proxy_count": oc_cfg.selected_proxy_ids.len(),
        "refreshed_at": now_ms(),
        "refresh_interval_sec": probe_status.interval_sec,
        "probe_status": probe_status
    })))
}

async fn check_ip_quotas_live(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let oc_cfg = state
        .store
        .provider_configs()
        .get_or_default("opencode")
        .await
        .map_err(err_resp)?;
    let summary = state
        .engine
        .probe_ip_quotas_live(true)
        .await
        .map_err(err_resp)?;
    let quotas = state
        .store
        .quotas()
        .list_ip_quotas_for_config(&oc_cfg)
        .await
        .map_err(err_resp)?;
    let probe_status = state.engine.quota_probe_status().await;
    Ok(Json(json!({
        "summary": summary,
        "ip_quotas": quotas,
        "proxy_mode": oc_cfg.proxy_mode,
        "selected_proxy_count": oc_cfg.selected_proxy_ids.len(),
        "probe_status": probe_status
    })))
}

async fn reset_all_cooling_quotas(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let res = state
        .engine
        .verify_and_reset_all_cooling()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!(res)))
}

async fn reset_ip_quota(
    State(state): State<AdminState>,
    Path(ip_key): Path<String>,
) -> std::result::Result<Json<Value>, Response> {
    let res = state
        .engine
        .verify_and_reset_ip_quota(&ip_key)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!(res)))
}

async fn update_ip_quota_limits(
    State(state): State<AdminState>,
    Path(ip_key): Path<String>,
    Json(input): Json<UpdateIpQuotaLimitsInput>,
) -> std::result::Result<Json<Value>, Response> {
    let quota = state
        .store
        .quotas()
        .update_ip_quota_limits(&ip_key, input, now_ms())
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "quota": quota })))
}

#[derive(Serialize)]
struct RedactedConnection {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub has_api_key: bool,
    pub api_key_preview: Option<String>,
    pub proxy_node_id: Option<String>,
    pub enabled: bool,
    pub cooldown_until: Option<i64>,
    pub backoff_level: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<ProviderConnection> for RedactedConnection {
    fn from(c: ProviderConnection) -> Self {
        let preview = c.api_key.as_deref().map(|k| {
            let head: String = k.chars().take(6).collect();
            format!("{head}***")
        });
        Self {
            id: c.id,
            provider_id: c.provider_id,
            name: c.name,
            has_api_key: c.api_key.as_ref().is_some_and(|s| !s.is_empty()),
            api_key_preview: preview,
            proxy_node_id: c.proxy_node_id,
            enabled: c.enabled,
            cooldown_until: c.cooldown_until,
            backoff_level: c.backoff_level,
            last_error: c.last_error,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

async fn list_antigravity_accounts(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let accounts = state
        .engine
        .antigravity()
        .list_accounts_with_partitions()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "accounts": accounts })))
}

async fn start_antigravity_oauth(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let started = state
        .engine
        .antigravity()
        .start_oauth()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!(started)))
}

async fn poll_antigravity_oauth(
    State(state): State<AdminState>,
    Path(oauth_state): Path<String>,
) -> std::result::Result<Json<Value>, Response> {
    let Some(status) = state.engine.antigravity().poll_oauth(&oauth_state) else {
        return Err(err_resp(TesRouteError::NotFound(
            "OAuth state not found or expired".into(),
        )));
    };
    Ok(Json(json!(status)))
}

#[derive(Deserialize)]
struct CompleteAntigravityOAuthRequest {
    callback_url: String,
    #[serde(default)]
    redirect_uri: Option<String>,
}

async fn complete_antigravity_oauth(
    State(state): State<AdminState>,
    Json(req): Json<CompleteAntigravityOAuthRequest>,
) -> std::result::Result<Json<Value>, Response> {
    let conn = state
        .engine
        .antigravity()
        .complete_oauth_manual(&req.callback_url, req.redirect_uri.as_deref())
        .await
        .map_err(err_resp)?;
    let accounts = state
        .engine
        .antigravity()
        .list_accounts_with_partitions()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "connection": RedactedConnection::from(conn),
        "accounts": accounts
    })))
}

#[derive(Deserialize)]
struct ImportAntigravityAccountRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    refresh_token: String,
    #[serde(default)]
    project_id: Option<String>,
}

async fn import_antigravity_account(
    State(state): State<AdminState>,
    Json(req): Json<ImportAntigravityAccountRequest>,
) -> std::result::Result<Json<Value>, Response> {
    let conn = state
        .engine
        .antigravity()
        .import_refresh_token(
            req.name.as_deref(),
            req.email.as_deref(),
            &req.refresh_token,
            req.project_id.as_deref(),
        )
        .await
        .map_err(err_resp)?;
    let accounts = state
        .engine
        .antigravity()
        .list_accounts_with_partitions()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "connection": RedactedConnection::from(conn),
        "accounts": accounts
    })))
}

async fn refresh_antigravity_quotas(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let refreshed = state
        .engine
        .antigravity()
        .refresh_all_account_quotas()
        .await
        .map_err(err_resp)?;
    let accounts = state
        .engine
        .antigravity()
        .list_accounts_with_partitions()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "refreshed": refreshed,
        "accounts": accounts
    })))
}

async fn refresh_single_antigravity_quota(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<Value>, Response> {
    let quota = state
        .engine
        .antigravity()
        .refresh_single_account_quota(&id)
        .await
        .map_err(err_resp)?;
    let accounts = state
        .engine
        .antigravity()
        .list_accounts_with_partitions()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({
        "quota": quota,
        "accounts": accounts
    })))
}

async fn list_connections(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let conns: Vec<RedactedConnection> = state
        .store
        .connections()
        .list_connections()
        .await
        .map_err(err_resp)?
        .into_iter()
        .map(RedactedConnection::from)
        .collect();
    Ok(Json(json!({ "connections": conns })))
}

async fn create_connection(
    State(state): State<AdminState>,
    Json(input): Json<CreateConnectionInput>,
) -> std::result::Result<(StatusCode, Json<Value>), Response> {
    let conn = state
        .store
        .connections()
        .create_connection(input)
        .await
        .map_err(err_resp)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "connection": RedactedConnection::from(conn) })),
    ))
}

async fn update_connection(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateConnectionInput>,
) -> std::result::Result<Json<Value>, Response> {
    let conn = state
        .store
        .connections()
        .update_connection(&id, input)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "connection": RedactedConnection::from(conn) })))
}

async fn delete_connection(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<StatusCode, Response> {
    state
        .store
        .connections()
        .delete_connection(&id)
        .await
        .map_err(err_resp)?;
    Ok(StatusCode::NO_CONTENT)
}

fn redact_api_key(k: &ApiKeyRecord) -> Value {
    json!({
        "id": k.id,
        "name": k.name,
        "key_prefix": k.key_prefix,
        "raw_key": k.raw_key,
        "quota_tokens_limit": k.quota_tokens_limit,
        "quota_tokens_used": k.quota_tokens_used,
        "quota_requests_limit": k.quota_requests_limit,
        "quota_requests_used": k.quota_requests_used,
        "enabled": k.enabled,
        "created_at": k.created_at,
        "last_used_at": k.last_used_at
    })
}

async fn list_keys(State(state): State<AdminState>) -> std::result::Result<Json<Value>, Response> {
    let keys: Vec<Value> = state
        .store
        .keys()
        .list_keys()
        .await
        .map_err(err_resp)?
        .into_iter()
        .map(|k| redact_api_key(&k))
        .collect();
    Ok(Json(json!({ "keys": keys })))
}

async fn create_key(
    State(state): State<AdminState>,
    Json(input): Json<CreateApiKeyInput>,
) -> std::result::Result<(StatusCode, Json<Value>), Response> {
    let created = state
        .store
        .keys()
        .create_key(input)
        .await
        .map_err(err_resp)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": created.record.id,
            "name": created.record.name,
            "key_prefix": created.record.key_prefix,
            "raw_key": created.raw_key,
            "quota_tokens_limit": created.record.quota_tokens_limit,
            "quota_requests_limit": created.record.quota_requests_limit,
            "created_at": created.record.created_at
        })),
    ))
}

async fn update_key(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateApiKeyInput>,
) -> std::result::Result<Json<Value>, Response> {
    let updated = state
        .store
        .keys()
        .update_key(&id, input)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "key": redact_api_key(&updated) })))
}

async fn delete_key(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<StatusCode, Response> {
    state
        .store
        .keys()
        .delete_key(&id)
        .await
        .map_err(err_resp)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct UsageQuery {
    limit: Option<i64>,
    period: Option<String>,
}

async fn get_usage(
    State(state): State<AdminState>,
    Query(q): Query<UsageQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let summary = state
        .store
        .usage()
        .summarize_usage_for_period(q.period.as_deref().unwrap_or("all"), q.limit.unwrap_or(50))
        .await
        .map_err(err_resp)?;
    Ok(Json(json!(summary)))
}

fn public_favicon_url_for_cp(cp: &CustomProvider) -> Option<String> {
    match cp.favicon_url.as_deref() {
        Some(raw) if raw.starts_with("data:") => {
            Some(format!("/api/custom-providers/{}/icon?v={}", cp.id, cp.updated_at))
        }
        other => other.map(str::to_string),
    }
}

async fn get_topology(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let now_ts = tesie_store::now_ms();
    let (
        mihomo_health_res,
        mihomo_states_res,
        proxies_res,
        oc_cfg_res,
        ag_cfg_res,
        ovh_cfg_res,
        usage_res,
        ag_accounts_res,
        combos_res,
        custom_providers_res,
    ) = tokio::join!(
        state.mihomo.health(),
        state.mihomo.fetch_proxy_states(),
        async { state.store.proxies().list_proxies().await },
        async { state.store.provider_configs().get_or_default("opencode").await },
        async { state.store.provider_configs().get_or_default("antigravity").await },
        async { state.store.provider_configs().get_or_default("ovh").await },
        async { state.store.usage().summarize_usage_for_period("24h", 15).await },
        state.engine.antigravity().list_accounts_with_partitions(),
        async { state.store.combos().list_combos().await },
        async { state.store.custom_providers().list().await },
    );

    let mihomo_health = mihomo_health_res.ok();
    let mihomo_states = mihomo_states_res.ok();
    let proxies = proxies_res.map_err(err_resp)?;
    let oc_cfg = oc_cfg_res.map_err(err_resp)?;
    let ag_cfg = ag_cfg_res.map_err(err_resp)?;
    let ovh_cfg = ovh_cfg_res.unwrap_or_else(|_| tesie_core::ProviderConfig {
        provider_id: "ovh".into(),
        enabled: true,
        proxy_mode: "all".into(),
        selected_proxy_ids: Vec::new(),
        window_duration_ms: 60_000,
        max_requests_per_window: Some(2),
        max_tokens_per_window: None,
        session_ttl_ms: 14_400_000,
        thinking_level: "auto".into(),
        probe_enabled: false,
        probe_interval_sec: 60,
        routing_strategy: "round-robin".into(),
        updated_at: now_ts,
    });
    let usage = usage_res.map_err(err_resp)?;
    let ag_accounts = ag_accounts_res.unwrap_or_default();
    let combos = combos_res.unwrap_or_default();
    let custom_providers = custom_providers_res.unwrap_or_default();

    let (quotas_res, ovh_cands_res) = tokio::join!(
        async { state.store.quotas().list_ip_quotas_for_config(&oc_cfg).await },
        async {
            state
                .store
                .quotas()
                .select_healthy_ovh_candidates(now_ts, &ovh_cfg, |p| {
                    mihomo_states
                        .as_ref()
                        .and_then(|m| m.get(&p.name))
                        .map(|s| s.alive)
                        .unwrap_or(p.alive)
                })
                .await
        },
    );
    let quotas = quotas_res.map_err(err_resp)?;
    let ovh_cands = ovh_cands_res.unwrap_or_default();

    let active_ips = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Active)
        .count();
    let cooling_ips = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Cooldown)
        .count();
    let probed_ips = quotas
        .iter()
        .filter(|q| !q.quota.ip_key.starts_with("unprobed:"))
        .count();

    let mut healthy_proxies = 0usize;
    let mut unhealthy_proxies = 0usize;
    for p in &proxies {
        let is_alive = mihomo_states
            .as_ref()
            .and_then(|m| m.get(&p.name))
            .map(|s| s.alive)
            .unwrap_or(p.alive);
        if is_alive {
            healthy_proxies += 1;
        } else {
            unhealthy_proxies += 1;
        }
    }

    let oc_usage = usage
        .by_provider
        .iter()
        .find(|p| p.provider_id == "opencode");
    let ag_usage = usage
        .by_provider
        .iter()
        .find(|p| p.provider_id == "antigravity");

    let ag_ready_accounts = ag_accounts
        .iter()
        .filter(|a| a.enabled && a.status == "ready")
        .count();
    let ag_cooling_accounts = ag_accounts
        .iter()
        .filter(|a| a.enabled && a.status == "cooldown")
        .count();

    let mut providers = vec![
        json!({
            "id": "opencode",
            "name": "OpenCode Free",
            "color": "#E56A4A",
            "logo": "/providers/opencode.png",
            "enabled": oc_cfg.enabled,
            "quota_strategy": "ip_pool",
            "selection_strategy": normalize_routing_strategy(&oc_cfg.routing_strategy, default_routing_strategy_for("opencode")),
            "proxy_mode": oc_cfg.proxy_mode,
            "active_units": active_ips,
            "cooling_units": cooling_ips,
            "total_requests": oc_usage.map(|x| x.total_requests).unwrap_or(0),
            "total_tokens": oc_usage.map(|x| x.total_tokens).unwrap_or(0),
            "avg_tokens_per_sec": oc_usage.map(|x| x.avg_tokens_per_sec).unwrap_or(0.0)
        }),
        json!({
            "id": "antigravity",
            "name": "Antigravity",
            "color": "#38BDF8",
            "logo": "/providers/antigravity.png",
            "enabled": ag_cfg.enabled && !ag_accounts.is_empty(),
            "quota_strategy": "account",
            "selection_strategy": normalize_routing_strategy(&ag_cfg.routing_strategy, default_routing_strategy_for("antigravity")),
            "proxy_mode": ag_cfg.proxy_mode,
            "active_units": ag_ready_accounts,
            "cooling_units": ag_cooling_accounts,
            "total_requests": ag_usage.map(|x| x.total_requests).unwrap_or(0),
            "total_tokens": ag_usage.map(|x| x.total_tokens).unwrap_or(0),
            "avg_tokens_per_sec": ag_usage.map(|x| x.avg_tokens_per_sec).unwrap_or(0.0)
        }),
    ];

    let ovh_usage = usage.by_provider.iter().find(|p| p.provider_id == "ovh");
    let ovh_active = ovh_cands.len();
    let ovh_total = proxies.len();
    let ovh_cooling = ovh_total.saturating_sub(ovh_active);

    providers.push(json!({
        "id": "ovh",
        "name": "OVHcloud",
        "color": "#0053D6",
        "logo": "/providers/ovh.png",
        "enabled": ovh_cfg.enabled,
        "quota_strategy": "ip_pool",
        "selection_strategy": normalize_routing_strategy(&ovh_cfg.routing_strategy, default_routing_strategy_for("ovh")),
        "proxy_mode": ovh_cfg.proxy_mode,
        "active_units": ovh_active,
        "cooling_units": ovh_cooling,
        "total_requests": ovh_usage.map(|x| x.total_requests).unwrap_or(0),
        "total_tokens": ovh_usage.map(|x| x.total_tokens).unwrap_or(0),
        "avg_tokens_per_sec": ovh_usage.map(|x| x.avg_tokens_per_sec).unwrap_or(0.0)
    }));

    for cp in custom_providers {
        let cp_cfg = state
            .store
            .provider_configs()
            .get_or_default(&cp.id)
            .await
            .unwrap_or_else(|_| tesie_core::ProviderConfig {
                provider_id: cp.id.clone(),
                enabled: cp.enabled,
                proxy_mode: "direct".into(),
                selected_proxy_ids: Vec::new(),
                window_duration_ms: 3_600_000,
                max_requests_per_window: None,
                max_tokens_per_window: None,
                session_ttl_ms: 14_400_000,
                thinking_level: "auto".into(),
                probe_enabled: false,
                probe_interval_sec: 30,
                routing_strategy: "fallback".into(),
                updated_at: cp.updated_at,
            });
        let cp_usage = usage.by_provider.iter().find(|p| p.provider_id == cp.id);
        let icon_url = public_favicon_url_for_cp(&cp);
        providers.push(json!({
            "id": cp.id,
            "name": cp.name,
            "prefix": cp.prefix,
            "is_custom": true,
            "color": cp.color,
            "logo": icon_url,
            "enabled": cp.enabled && cp_cfg.enabled,
            "quota_strategy": "api_key",
            "selection_strategy": normalize_routing_strategy(&cp_cfg.routing_strategy, "fallback"),
            "proxy_mode": cp_cfg.proxy_mode,
            "active_units": cp.api_keys.len(),
            "cooling_units": 0,
            "total_requests": cp_usage.map(|x| x.total_requests).unwrap_or(0),
            "total_tokens": cp_usage.map(|x| x.total_tokens).unwrap_or(0),
            "avg_tokens_per_sec": cp_usage.map(|x| x.avg_tokens_per_sec).unwrap_or(0.0)
        }));
    }

    let active_requests = state.engine.active_requests_snapshot();
    let (last_provider, error_provider) = state.engine.recent_provider_signals();

    Ok(Json(json!({
        "core_healthy": mihomo_health.is_some(),
        "core": mihomo_health,
        "mihomo_healthy": mihomo_health.is_some(),
        "mihomo": mihomo_health,
        "active_requests": active_requests,
        "last_provider": last_provider,
        "error_provider": error_provider,
        "bridge_event": state.engine.last_bridge_event(),
        "bridge_stats": state.engine.bridge_stats(),
        "proxy_summary": {
            "total_proxies": proxies.iter().filter(|p| !p.is_direct).count(),
            "healthy_proxies": healthy_proxies,
            "unhealthy_proxies": unhealthy_proxies,
            "probed_ips": probed_ips,
            "active_ips": active_ips,
            "cooling_ips": cooling_ips
        },
        "providers": providers,
        "combos": combos,
        "antigravity_accounts": ag_accounts
    })))
}

async fn get_provider_quotas(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let now = now_ms();
    let oc_cfg = state
        .store
        .provider_configs()
        .get_or_default("opencode")
        .await
        .map_err(err_resp)?;
    let ag_cfg = state
        .store
        .provider_configs()
        .get_or_default("antigravity")
        .await
        .map_err(err_resp)?;
    let quotas = state
        .store
        .quotas()
        .list_ip_quotas_for_config(&oc_cfg)
        .await
        .map_err(err_resp)?;
    let ag_accounts = state
        .engine
        .antigravity()
        .list_accounts_with_partitions()
        .await
        .unwrap_or_default();

    let oc_active = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Active)
        .count();
    let oc_cooling = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Cooldown)
        .count();
    let oc_probed = quotas
        .iter()
        .filter(|q| !q.quota.ip_key.starts_with("unprobed:"))
        .count();

    let ag_active = ag_accounts
        .iter()
        .filter(|a| a.enabled && a.status == "ready")
        .count();
    let ag_cooling = ag_accounts
        .iter()
        .filter(|a| a.enabled && a.status == "cooldown")
        .count();

    let oc_probe = state.engine.quota_probe_status().await;
    let ag_probe = state.engine.antigravity().probe_status().await;

    let oc_strategy = normalize_routing_strategy(
        &oc_cfg.routing_strategy,
        default_routing_strategy_for("opencode"),
    );
    let ag_strategy = normalize_routing_strategy(
        &ag_cfg.routing_strategy,
        default_routing_strategy_for("antigravity"),
    );

    let mut providers = vec![json!({
        "id": "opencode",
        "name": "OpenCode Free",
        "description": "Per-IP quota tracking across selected TesieCore proxies with configurable IP routing strategy and queued-until-verified probing.",
        "strategy": "ip_pool",
        "selection_strategy": oc_strategy,
        "routing_strategy": oc_strategy,
        "unit_label": "Exit IP",
        "active_count": oc_active,
        "cooling_count": oc_cooling,
        "probed_count": oc_probed,
        "quotas_count": quotas.len(),
        "proxy_mode": oc_cfg.proxy_mode,
        "window_duration_ms": oc_cfg.window_duration_ms,
        "default_max_requests": oc_cfg.max_requests_per_window,
        "default_max_tokens": oc_cfg.max_tokens_per_window,
        "probe_enabled": oc_cfg.probe_enabled,
        "probe_interval_sec": oc_cfg.probe_interval_sec,
        "probe_status": oc_probe,
        "refresh_interval_sec": oc_cfg.probe_interval_sec,
        "refreshed_at": now
    })];

    if !ag_accounts.is_empty() {
        providers.push(json!({
            "id": "antigravity",
            "name": "Antigravity",
            "description": "Per-account & per-model Google Cloud Code quota tracking with disjoint Smart Proxy Partitioning and configurable multi-account routing.",
            "strategy": "account",
            "selection_strategy": ag_strategy,
            "routing_strategy": ag_strategy,
            "unit_label": "Account",
            "active_count": ag_active,
            "cooling_count": ag_cooling,
            "probed_count": ag_accounts.len(),
            "quotas_count": ag_accounts.len(),
            "proxy_mode": ag_cfg.proxy_mode,
            "window_duration_ms": ag_cfg.window_duration_ms,
            "probe_enabled": ag_cfg.probe_enabled,
            "probe_interval_sec": ag_cfg.probe_interval_sec,
            "probe_status": ag_probe,
            "refresh_interval_sec": ag_cfg.probe_interval_sec,
            "refreshed_at": now
        }));
    }

    let ovh_cfg = state
        .store
        .provider_configs()
        .get_or_default("ovh")
        .await
        .unwrap_or_else(|_| tesie_core::ProviderConfig {
            provider_id: "ovh".into(),
            enabled: true,
            proxy_mode: "all".into(),
            selected_proxy_ids: Vec::new(),
            window_duration_ms: 60_000,
            max_requests_per_window: Some(2),
            max_tokens_per_window: None,
            session_ttl_ms: 14_400_000,
            thinking_level: "auto".into(),
            probe_enabled: false,
            probe_interval_sec: 60,
            routing_strategy: "round-robin".into(),
            updated_at: now,
        });
    let ovh_strategy = normalize_routing_strategy(
        &ovh_cfg.routing_strategy,
        default_routing_strategy_for("ovh"),
    );
    let ovh_cands = state
        .store
        .quotas()
        .select_healthy_ovh_candidates(now, &ovh_cfg, |_| true)
        .await
        .unwrap_or_default();
    let ovh_active = ovh_cands.len();
    let ovh_proxies = state
        .store
        .proxies()
        .list_for_provider_config(&ovh_cfg)
        .await
        .unwrap_or_default();
    let ovh_total = ovh_proxies.len();
    let ovh_cooling = ovh_total.saturating_sub(ovh_active);

    providers.push(json!({
        "id": "ovh",
        "name": "OVHcloud",
        "description": "Per-IP rate tracking across proxy pool (2 requests/minute per IP) with automatic cooldown and rolling 60s windows.",
        "strategy": "ip_pool",
        "selection_strategy": ovh_strategy,
        "routing_strategy": ovh_strategy,
        "unit_label": "Exit IP",
        "active_count": ovh_active,
        "cooling_count": ovh_cooling,
        "probed_count": ovh_total,
        "quotas_count": ovh_total,
        "proxy_mode": ovh_cfg.proxy_mode,
        "window_duration_ms": ovh_cfg.window_duration_ms,
        "default_max_requests": ovh_cfg.max_requests_per_window,
        "default_max_tokens": ovh_cfg.max_tokens_per_window,
        "probe_enabled": false,
        "probe_interval_sec": 60,
        "probe_status": json!({ "status": "idle" }),
        "refresh_interval_sec": 60,
        "refreshed_at": now
    }));

    Ok(Json(json!({
        "providers": providers,
        "antigravity_accounts": ag_accounts,
        "antigravity_probe_status": ag_probe,
        "refreshed_at": now,
        "refresh_interval_sec": oc_cfg.probe_interval_sec
    })))
}

async fn list_providers(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let now = now_ms();
    let (
        settings_res,
        oc_cfg_res,
        ag_cfg_res,
        ovh_cfg_res,
        mihomo_states_res,
        ag_accounts_res,
        all_models,
        custom_providers_res,
    ) = tokio::join!(
        async { state.store.settings().get_settings().await },
        async { state.store.provider_configs().get_or_default("opencode").await },
        async { state.store.provider_configs().get_or_default("antigravity").await },
        async { state.store.provider_configs().get_or_default("ovh").await },
        state.mihomo.fetch_proxy_states(),
        state.engine.antigravity().list_accounts_with_partitions(),
        state.catalog.list_models(
            &state.store,
            Some(&state.mihomo),
            &state.zen_base_url,
            state.fetch_live_models,
        ),
        async { state.store.custom_providers().list().await },
    );

    let settings = settings_res.map_err(err_resp)?;
    let oc_cfg = oc_cfg_res.map_err(err_resp)?;
    let ag_cfg = ag_cfg_res.map_err(err_resp)?;
    let ovh_cfg = ovh_cfg_res.map_err(err_resp)?;
    let mihomo_states = mihomo_states_res.ok();
    let ag_accounts = ag_accounts_res.unwrap_or_default();
    let custom_providers = custom_providers_res.map_err(err_resp)?;

    let (oc_proxies_res, ag_proxies_res, ovh_proxies_res, quotas_res) = tokio::join!(
        async { state.store.proxies().list_for_provider_config(&oc_cfg).await },
        async { state.store.proxies().list_for_provider_config(&ag_cfg).await },
        async { state.store.proxies().list_for_provider_config(&ovh_cfg).await },
        async { state.store.quotas().list_ip_quotas_for_config(&oc_cfg).await },
    );

    let oc_proxies = oc_proxies_res.map_err(err_resp)?;
    let ag_proxies = ag_proxies_res.map_err(err_resp)?;
    let ovh_proxies = ovh_proxies_res.map_err(err_resp)?;
    let quotas = quotas_res.map_err(err_resp)?;

    let count_health = |nodes: &[ProxyNode]| -> (usize, usize) {
        let mut h = 0usize;
        let mut u = 0usize;
        for p in nodes {
            let alive = mihomo_states
                .as_ref()
                .and_then(|m| m.get(&p.name))
                .map(|s| s.alive)
                .unwrap_or(p.is_direct || p.alive);
            if alive {
                h += 1;
            } else {
                u += 1;
            }
        }
        (h, u)
    };

    let (oc_healthy, oc_unhealthy) = count_health(&oc_proxies);
    let (ag_healthy, ag_unhealthy) = count_health(&ag_proxies);

    let oc_models: Vec<String> = all_models
        .iter()
        .filter(|m| m.owned_by == "opencode")
        .map(|m| {
            if m.id.contains('/') {
                m.id.clone()
            } else {
                format!("oc/{}", m.id)
            }
        })
        .collect();

    let ag_models: Vec<String> = all_models
        .iter()
        .filter(|m| m.owned_by == "antigravity")
        .map(|m| {
            if m.id.contains('/') {
                m.id.clone()
            } else {
                format!("ag/{}", m.id)
            }
        })
        .collect();

    let oc_active = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Active)
        .count();
    let oc_cooling = quotas
        .iter()
        .filter(|q| q.quota.status == IpQuotaStatus::Cooldown)
        .count();

    let ag_ready = ag_accounts
        .iter()
        .filter(|a| a.enabled && a.status == "ready")
        .count();
    let ag_cooling = ag_accounts
        .iter()
        .filter(|a| a.enabled && a.status == "cooldown")
        .count();

    let oc_strategy = normalize_routing_strategy(
        &oc_cfg.routing_strategy,
        default_routing_strategy_for("opencode"),
    );
    let ag_strategy = normalize_routing_strategy(
        &ag_cfg.routing_strategy,
        default_routing_strategy_for("antigravity"),
    );

    let mut providers = vec![
        json!({
            "id": "opencode",
            "name": "OpenCode Free",
            "prefix": "oc",
            "category": "free",
            "is_custom": false,
            "auth_type": "none",
            "logo": "/providers/opencode.png",
            "color": "#E56A4A",
            "description": "Zero-key OpenCode upstream with Bun TLS impersonation and smart proxy routing.",
            "config": oc_cfg,
            "selection_strategy": oc_strategy,
            "routing_strategy": oc_strategy,
            "quota_tracking": "per_ip",
            "quota_refresh_interval_sec": oc_cfg.probe_interval_sec,
            "proxy_test_interval_sec": settings.proxy_test_interval_sec,
            "health_source": "tesie-core",
            "healthy_proxies": oc_healthy,
            "unhealthy_proxies": oc_unhealthy,
            "active_units": oc_active,
            "cooling_units": oc_cooling,
            "total_units": quotas.len(),
            "refreshed_at": now,
            "models": oc_models
        }),
        json!({
            "id": "antigravity",
            "name": "Antigravity",
            "prefix": "ag",
            "category": "oauth",
            "is_custom": false,
            "auth_type": "oauth",
            "logo": "/providers/antigravity.png",
            "color": "#38BDF8",
            "description": "Multi-account Google Cloud Code / Antigravity provider with disjoint Smart Proxy Partitioning (e.g. 15/15 per account), configurable account routing, and live per-model quota tracking.",
            "config": ag_cfg,
            "selection_strategy": ag_strategy,
            "routing_strategy": ag_strategy,
            "quota_tracking": "per_account",
            "quota_refresh_interval_sec": ag_cfg.probe_interval_sec,
            "proxy_test_interval_sec": settings.proxy_test_interval_sec,
            "health_source": "tesie-core",
            "healthy_proxies": ag_healthy,
            "unhealthy_proxies": ag_unhealthy,
            "active_units": ag_ready,
            "cooling_units": ag_cooling,
            "total_units": ag_accounts.len(),
            "accounts": ag_accounts,
            "refreshed_at": now,
            "models": ag_models
        }),
    ];

    let (ovh_healthy, ovh_unhealthy) = count_health(&ovh_proxies);
    let ovh_strategy = normalize_routing_strategy(
        &ovh_cfg.routing_strategy,
        default_routing_strategy_for("ovh"),
    );
    let ovh_models: Vec<String> = tesie_core::catalog::OVH_MODELS
        .iter()
        .map(|m| format!("ovh/{}", m.id))
        .collect();

    providers.push(json!({
        "id": "ovh",
        "name": "OVHcloud AI Endpoints",
        "prefix": "ovh",
        "category": "free",
        "is_custom": false,
        "auth_type": "none",
        "logo": "/providers/ovh.png",
        "color": "#0053D6",
        "description": "Zero-key OVHcloud AI Endpoints free tier with 2 requests per minute per IP, proxy pool load balancing, and rate limit database caching.",
        "config": ovh_cfg,
        "selection_strategy": ovh_strategy,
        "routing_strategy": ovh_strategy,
        "quota_tracking": "per_ip",
        "quota_refresh_interval_sec": 60,
        "proxy_test_interval_sec": settings.proxy_test_interval_sec,
        "health_source": "tesie-core",
        "healthy_proxies": ovh_healthy,
        "unhealthy_proxies": ovh_unhealthy,
        "active_units": ovh_healthy,
        "cooling_units": 0,
        "total_units": ovh_proxies.len(),
        "refreshed_at": now,
        "models": ovh_models
    }));

    for cp in custom_providers {
        let mut cp_cfg = state
            .store
            .provider_configs()
            .get_or_default(&cp.id)
            .await
            .map_err(err_resp)?;
        cp_cfg.enabled = cp.enabled && cp_cfg.enabled;
        let cp_proxies = state
            .store
            .proxies()
            .list_for_provider_config(&cp_cfg)
            .await
            .map_err(err_resp)?;
        let (cp_healthy, cp_unhealthy) = count_health(&cp_proxies);
        let cp_strategy = normalize_routing_strategy(&cp_cfg.routing_strategy, "fallback");
        let cp_prefix_slash = format!("{}/", cp.prefix);
        let cp_models: Vec<String> = cp
            .models
            .iter()
            .map(|m| {
                if m.starts_with(&cp_prefix_slash) {
                    m.clone()
                } else {
                    format!("{cp_prefix_slash}{m}")
                }
            })
            .collect();
        let icon_url = public_favicon_url_for_cp(&cp);
        let mut cp_summary = cp.clone();
        cp_summary.favicon_url = icon_url.clone();
        providers.push(json!({
            "id": cp.id,
            "name": cp.name,
            "prefix": cp.prefix,
            "category": "custom",
            "is_custom": true,
            "auth_type": "api_key",
            "protocol": cp.protocol,
            "api_type": cp.api_type,
            "base_url": cp.base_url,
            "logo": icon_url,
            "favicon_url": icon_url,
            "color": cp.color,
            "description": format!(
                "{} ({} {}) — {} model(s), {} key(s)",
                cp.base_url,
                cp.protocol.to_ascii_uppercase(),
                cp.api_type,
                cp.models.len(),
                cp.api_keys.len()
            ),
            "config": cp_cfg,
            "selection_strategy": cp_strategy,
            "routing_strategy": cp_strategy,
            "quota_tracking": "api_key",
            "quota_refresh_interval_sec": cp_cfg.probe_interval_sec,
            "proxy_test_interval_sec": settings.proxy_test_interval_sec,
            "health_source": "tesie-core",
            "healthy_proxies": cp_healthy,
            "unhealthy_proxies": cp_unhealthy,
            "active_units": cp.api_keys.len(),
            "cooling_units": 0,
            "total_units": cp.api_keys.len(),
            "custom_provider": cp_summary,
            "refreshed_at": now,
            "models": cp_models
        }));
    }

    Ok(Json(json!({
        "providers": providers,
        "refreshed_at": now
    })))
}

async fn update_provider_config(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateProviderConfigInput>,
) -> std::result::Result<Json<Value>, Response> {
    let custom_match = if id != "opencode" && id != "antigravity" && id != "ovh" {
        state
            .store
            .custom_providers()
            .get(&id)
            .await
            .map_err(err_resp)?
    } else {
        None
    };

    if id != "opencode" && id != "antigravity" && id != "ovh" && custom_match.is_none() {
        return Err(err_resp(TesRouteError::NotFound(format!(
            "provider '{id}' not found"
        ))));
    }

    let enabled_sync = input.enabled;
    let updated = state
        .store
        .provider_configs()
        .update_config(&id, input)
        .await
        .map_err(err_resp)?;

    if custom_match.is_some()
        && let Some(en) = enabled_sync
    {
        let _ = state
            .store
            .custom_providers()
            .update(
                &id,
                UpdateCustomProviderInput {
                    enabled: Some(en),
                    ..Default::default()
                },
            )
            .await;
    }

    state.engine.notify_probe_config_changed();
    Ok(Json(json!({ "config": updated })))
}

async fn test_provider(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<Value>, Response> {
    let custom_match = if id != "opencode" && id != "antigravity" && id != "ovh" {
        state
            .store
            .custom_providers()
            .get(&id)
            .await
            .map_err(err_resp)?
    } else {
        None
    };

    if id != "opencode" && id != "antigravity" && id != "ovh" && custom_match.is_none() {
        return Err(err_resp(TesRouteError::NotFound(format!(
            "provider '{id}' not found"
        ))));
    }
    let started = now_ms();
    let cfg = state
        .store
        .provider_configs()
        .get_or_default(&id)
        .await
        .map_err(err_resp)?;

    if !cfg.enabled || custom_match.as_ref().is_some_and(|c| !c.enabled) {
        return Ok(Json(json!({
            "ok": false,
            "provider_id": id,
            "latency_ms": 0,
            "message": "Provider is currently disabled"
        })));
    }

    if id == "ovh" {
        let cands = state
            .store
            .quotas()
            .select_healthy_candidates_for_config(started, &cfg, |p| p.alive)
            .await
            .map_err(err_resp)?;
        let Some(first) = cands.first() else {
            return Ok(Json(json!({
                "ok": false,
                "provider_id": id,
                "latency_ms": now_ms() - started,
                "message": "No healthy active exit IPs available for OVHcloud"
            })));
        };

        let resp = state
            .mihomo
            .relay(RelayRequest {
                target_url: "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/models".to_string(),
                proxy_name: first.proxy.name.clone(),
                method: "GET".to_string(),
                fingerprint: "opencode".to_string(),
                timeout_ms: 10000,
                upstream_authorization: None,
                headers: vec![("Accept".into(), "application/json".into())],
                body: Bytes::new(),
            })
            .await;

        let latency = (now_ms() - started).max(1);
        return match resp {
            Ok(r) if r.status().is_success() => Ok(Json(json!({
                "ok": true,
                "provider_id": id,
                "proxy": first.proxy.name,
                "egress_ip": first.proxy.ip,
                "latency_ms": latency,
                "message": format!("Reachable via {} ({}ms, OVHcloud AI Endpoints live)", first.proxy.name, latency)
            }))),
            Ok(r) => Ok(Json(json!({
                "ok": false,
                "provider_id": id,
                "latency_ms": latency,
                "message": format!("OVH upstream returned HTTP {}", r.status().as_u16())
            }))),
            Err(e) => Ok(Json(json!({
                "ok": false,
                "provider_id": id,
                "latency_ms": latency,
                "message": e.to_string()
            }))),
        };
    }

    if let Some(cp) = custom_match {
        return match fetch_custom_provider_models(&cp, &state.mihomo, &cfg, &state.store).await {
            Ok(models) => {
                let lat = (now_ms() - started).max(1);
                Ok(Json(json!({
                    "ok": true,
                    "provider_id": id,
                    "latency_ms": lat,
                    "model_count": models.len(),
                    "models": models,
                    "message": format!("Connected to {} ({} models discovered in {}ms)", cp.name, models.len(), lat)
                })))
            }
            Err(err) => Ok(Json(json!({
                "ok": false,
                "provider_id": id,
                "latency_ms": (now_ms() - started).max(1),
                "message": err.to_string()
            }))),
        };
    }

    if id == "antigravity" {
        let accounts = state
            .engine
            .antigravity()
            .list_accounts_with_partitions()
            .await
            .map_err(err_resp)?;
        let Some(first_acc) = accounts.iter().find(|a| a.enabled) else {
            return Ok(Json(json!({
                "ok": false,
                "provider_id": id,
                "latency_ms": now_ms() - started,
                "message": "No enabled Antigravity accounts connected"
            })));
        };
        let proxy_label = first_acc
            .partition
            .as_ref()
            .and_then(|p| p.primary_proxy.as_ref())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "DIRECT".into());
        return match state
            .engine
            .antigravity()
            .refresh_single_account_quota(&first_acc.id)
            .await
        {
            Ok(snap) => {
                let lat = (now_ms() - started).max(1);
                Ok(Json(json!({
                    "ok": true,
                    "provider_id": id,
                    "proxy": proxy_label,
                    "latency_ms": lat,
                    "message": format!(
                        "Verified {} via {} ({} models synced in {}ms)",
                        first_acc.name,
                        proxy_label,
                        snap.models.len(),
                        lat
                    )
                })))
            }
            Err(err) => Ok(Json(json!({
                "ok": false,
                "provider_id": id,
                "latency_ms": (now_ms() - started).max(1),
                "message": err.to_string()
            }))),
        };
    }

    let cands = state
        .engine
        .selector()
        .select_candidates_for_config(&state.mihomo, started, &cfg)
        .await
        .map_err(err_resp)?;
    let Some(first) = cands.first() else {
        return Ok(Json(json!({
            "ok": false,
            "provider_id": id,
            "latency_ms": now_ms() - started,
            "message": "No healthy active exit IPs available in TesieCore for selected proxies"
        })));
    };

    let target_url = if state.zen_base_url.contains("127.0.0.1") || state.zen_base_url.contains("localhost") {
        format!("{}/zen/v1/models", state.zen_base_url.trim_end_matches('/'))
    } else {
        "https://models.opencode.ai/api.json".to_string()
    };
    let resp = state
        .mihomo
        .relay(RelayRequest {
            target_url,
            proxy_name: first.proxy.name.clone(),
            method: "GET".to_string(),
            fingerprint: "opencode".to_string(),
            timeout_ms: 8000,
            upstream_authorization: None,
            headers: vec![("Accept".into(), "application/json".into())],
            body: Bytes::new(),
        })
        .await;

    let latency = (now_ms() - started).max(1);
    match resp {
        Ok(r) if r.status().is_success() => Ok(Json(json!({
            "ok": true,
            "provider_id": id,
            "proxy": first.proxy.name,
            "egress_ip": first.proxy.ip,
            "latency_ms": latency,
            "message": format!("Reachable via {} ({}ms)", first.proxy.name, latency)
        }))),
        Ok(r) => Ok(Json(json!({
            "ok": false,
            "provider_id": id,
            "latency_ms": latency,
            "message": format!("Upstream returned HTTP {}", r.status().as_u16())
        }))),
        Err(e) => Ok(Json(json!({
            "ok": false,
            "provider_id": id,
            "latency_ms": latency,
            "message": e.to_string()
        }))),
    }
}

async fn fetch_provider_models_handler(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<Value>, Response> {
    match id.as_str() {
        "opencode" => {
            let live = state
                .catalog
                .force_fetch_live_free(Some(&state.mihomo), &state.zen_base_url)
                .await;
            let count = live.len();
            Ok(Json(json!({
                "status": "ok",
                "provider": "opencode",
                "name": "OpenCode",
                "count": count,
                "models": live,
            })))
        }
        "ovh" => {
            let live = state
                .catalog
                .force_fetch_live_ovh(Some(&state.mihomo))
                .await;
            let count = live.len();
            Ok(Json(json!({
                "status": "ok",
                "provider": "ovh",
                "name": "OVHcloud AI Endpoints",
                "count": count,
                "models": live,
            })))
        }
        other => {
            let cp = state
                .store
                .custom_providers()
                .get(other)
                .await
                .map_err(err_resp)?;
            if let Some(cp) = cp {
                let cfg = state
                    .store
                    .provider_configs()
                    .get_or_default(other)
                    .await
                    .map_err(err_resp)?;
                let fetched = fetch_custom_provider_models(&cp, &state.mihomo, &cfg, &state.store)
                    .await
                    .map_err(err_resp)?;
                let mut merged = cp.models.clone();
                let mut seen: HashSet<String> = merged.iter().cloned().collect();
                for m in &fetched {
                    if seen.insert(m.clone()) {
                        merged.push(m.clone());
                    }
                }
                let updated = state
                    .store
                    .custom_providers()
                    .update(
                        other,
                        UpdateCustomProviderInput {
                            models: Some(merged.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(err_resp)?;
                Ok(Json(json!({
                    "status": "ok",
                    "provider": other,
                    "name": cp.name,
                    "count": merged.len(),
                    "models": merged,
                    "custom_provider": updated,
                })))
            } else {
                Err(err_resp(TesRouteError::NotFound(format!(
                    "provider '{id}' not found or does not support dynamic catalog fetch"
                ))))
            }
        }
    }
}

async fn list_combos(State(state): State<AdminState>) -> std::result::Result<Json<Value>, Response> {
    let combos = state.store.combos().list_combos().await.map_err(err_resp)?;
    Ok(Json(json!({ "combos": combos })))
}

async fn create_combo(
    State(state): State<AdminState>,
    Json(input): Json<CreateComboInput>,
) -> std::result::Result<(StatusCode, Json<Value>), Response> {
    let combo = state
        .store
        .combos()
        .create_combo(input)
        .await
        .map_err(err_resp)?;
    Ok((StatusCode::CREATED, Json(json!({ "combo": combo }))))
}

async fn update_combo(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateComboInput>,
) -> std::result::Result<Json<Value>, Response> {
    let combo = state
        .store
        .combos()
        .update_combo(&id, input)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "combo": combo })))
}

async fn delete_combo(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<StatusCode, Response> {
    state
        .store
        .combos()
        .delete_combo(&id)
        .await
        .map_err(err_resp)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct LogsQuery {
    limit: Option<i64>,
    status: Option<String>,
    proxy: Option<String>,
    model: Option<String>,
    q: Option<String>,
}

async fn query_logs(
    State(state): State<AdminState>,
    Query(q): Query<LogsQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let logs = state
        .store
        .usage()
        .query_logs(LogQueryFilter {
            limit: q.limit.unwrap_or(100),
            status: q.status,
            proxy: q.proxy,
            model: q.model,
            search: q.q,
        })
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "logs": logs })))
}

async fn clear_logs(State(state): State<AdminState>) -> std::result::Result<Json<Value>, Response> {
    let deleted = state.store.usage().clear_logs().await.map_err(err_resp)?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Deserialize)]
struct FaviconQuery {
    base_url: String,
}

async fn list_custom_providers(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let custom_providers = state
        .store
        .custom_providers()
        .list()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "custom_providers": custom_providers })))
}

async fn discover_custom_provider_favicon(
    Query(q): Query<FaviconQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let favicon_url = discover_favicon_data_uri(&q.base_url).await;
    Ok(Json(json!({
        "base_url": q.base_url,
        "favicon_url": favicon_url
    })))
}

#[derive(Deserialize)]
struct PreviewCustomModelsRequest {
    base_url: String,
    #[serde(default = "default_preview_protocol")]
    protocol: String,
    #[serde(default = "default_preview_api_type")]
    api_type: String,
    #[serde(default)]
    api_keys: Vec<String>,
    #[serde(default)]
    custom_headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    proxy_mode: Option<String>,
}

fn default_preview_protocol() -> String {
    "openai".to_string()
}

fn default_preview_api_type() -> String {
    "chat".to_string()
}

async fn fetch_custom_provider_models_preview(
    State(state): State<AdminState>,
    Json(req): Json<PreviewCustomModelsRequest>,
) -> std::result::Result<Json<Value>, Response> {
    let clean_base = req.base_url.trim().trim_end_matches('/').to_string();
    let cp = CustomProvider {
        id: "cp-preview".into(),
        name: "Preview".into(),
        prefix: "preview".into(),
        protocol: req.protocol,
        api_type: req.api_type,
        base_url: clean_base,
        api_keys: req
            .api_keys
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        models: Vec::new(),
        custom_headers: req.custom_headers,
        favicon_url: None,
        color: "#0EA5E9".into(),
        enabled: true,
        created_at: 0,
        updated_at: 0,
    };
    let mut cfg = state
        .store
        .provider_configs()
        .get_or_default("cp-preview")
        .await
        .map_err(err_resp)?;
    if let Some(pm) = req.proxy_mode.as_deref().filter(|s| !s.is_empty()) {
        cfg.proxy_mode = pm.to_string();
    }
    let models = fetch_custom_provider_models(&cp, &state.mihomo, &cfg, &state.store)
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "models": models })))
}

async fn create_custom_provider(
    State(state): State<AdminState>,
    Json(mut input): Json<CreateCustomProviderInput>,
) -> std::result::Result<(StatusCode, Json<Value>), Response> {
    if input
        .favicon_url
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        input.favicon_url = discover_favicon_data_uri(&input.base_url).await;
    }
    let cp = state
        .store
        .custom_providers()
        .create(input)
        .await
        .map_err(err_resp)?;
    Ok((StatusCode::CREATED, Json(json!({ "custom_provider": cp }))))
}

async fn update_custom_provider(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    Json(mut input): Json<UpdateCustomProviderInput>,
) -> std::result::Result<Json<Value>, Response> {
    let existing = state
        .store
        .custom_providers()
        .get(&id)
        .await
        .map_err(err_resp)?
        .ok_or_else(|| err_resp(TesRouteError::NotFound(format!("custom provider '{id}' not found"))))?;

    if let Some(Some(ref url)) = input.favicon_url
        && url.starts_with("/api/custom-providers/")
    {
        input.favicon_url = None;
    }

    let base_changed = input
        .base_url
        .as_deref()
        .map(|u| u.trim().trim_end_matches('/') != existing.base_url.trim_end_matches('/'))
        .unwrap_or(false);
    if base_changed && input.favicon_url.is_none() {
        let target_url = input.base_url.as_deref().unwrap_or(&existing.base_url);
        if let Some(icon) = discover_favicon_data_uri(target_url).await {
            input.favicon_url = Some(Some(icon));
        }
    }

    let enabled_sync = input.enabled;
    let cp = state
        .store
        .custom_providers()
        .update(&id, input)
        .await
        .map_err(err_resp)?;

    if let Some(en) = enabled_sync {
        let _ = state
            .store
            .provider_configs()
            .update_config(
                &id,
                UpdateProviderConfigInput {
                    enabled: Some(en),
                    ..Default::default()
                },
            )
            .await;
    }
    Ok(Json(json!({ "custom_provider": cp })))
}

async fn delete_custom_provider(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<StatusCode, Response> {
    state
        .store
        .custom_providers()
        .delete(&id)
        .await
        .map_err(err_resp)?;
    let _ = state.store.provider_configs().delete_config(&id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh_custom_provider_favicon(
    State(state): State<AdminState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<Value>, Response> {
    let existing = state
        .store
        .custom_providers()
        .get(&id)
        .await
        .map_err(err_resp)?
        .ok_or_else(|| err_resp(TesRouteError::NotFound(format!("custom provider '{id}' not found"))))?;
    let icon = discover_favicon_data_uri(&existing.base_url).await;
    let updated = if let Some(ref data_uri) = icon {
        state
            .store
            .custom_providers()
            .update(
                &id,
                UpdateCustomProviderInput {
                    favicon_url: Some(Some(data_uri.clone())),
                    ..Default::default()
                },
            )
            .await
            .map_err(err_resp)?
    } else {
        existing
    };
    Ok(Json(json!({
        "custom_provider": updated,
        "favicon_url": icon
    })))
}

#[derive(Deserialize, Default)]
struct FetchCustomModelsOptions {
    #[serde(default)]
    save: Option<bool>,
}

async fn fetch_custom_provider_models_handler(
    State(state): State<AdminState>,
    Path(id): Path<String>,
    body: Option<Json<FetchCustomModelsOptions>>,
) -> std::result::Result<Json<Value>, Response> {
    let cp = state
        .store
        .custom_providers()
        .get(&id)
        .await
        .map_err(err_resp)?
        .ok_or_else(|| err_resp(TesRouteError::NotFound(format!("custom provider '{id}' not found"))))?;
    let cfg = state
        .store
        .provider_configs()
        .get_or_default(&id)
        .await
        .map_err(err_resp)?;
    let fetched = fetch_custom_provider_models(&cp, &state.mihomo, &cfg, &state.store)
        .await
        .map_err(err_resp)?;

    let should_save = body.and_then(|b| b.save).unwrap_or(false);
    let updated = if should_save && !fetched.is_empty() {
        let mut merged = cp.models.clone();
        let mut seen: HashSet<String> = merged.iter().cloned().collect();
        for m in &fetched {
            if seen.insert(m.clone()) {
                merged.push(m.clone());
            }
        }
        state
            .store
            .custom_providers()
            .update(
                &id,
                UpdateCustomProviderInput {
                    models: Some(merged),
                    ..Default::default()
                },
            )
            .await
            .map_err(err_resp)?
    } else {
        cp
    };

    Ok(Json(json!({
        "models": fetched,
        "custom_provider": updated
    })))
}

fn data_dir_from_state(state: &AdminState) -> PathBuf {
    state
        .mihomo
        .work_dir()
        .parent()
        .unwrap_or_else(|| state.mihomo.work_dir())
        .to_path_buf()
}

fn compute_db_size_bytes(data_dir: &std::path::Path) -> u64 {
    ["tesie.db", "tesie.db-wal", "tesie.db-shm"]
        .iter()
        .filter_map(|name| std::fs::metadata(data_dir.join(name)).ok())
        .map(|m| m.len())
        .sum()
}

async fn build_settings_payload(state: &AdminState) -> std::result::Result<Value, Response> {
    let settings = state
        .store
        .settings()
        .get_settings()
        .await
        .map_err(err_resp)?;
    let env_token_active = state
        .admin_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    let has_admin_password = state
        .store
        .settings()
        .has_admin_password()
        .await
        .unwrap_or(false);
    let data_dir = data_dir_from_state(state);
    let db_path = data_dir.join("tesie.db");
    let db_size_bytes = compute_db_size_bytes(&data_dir);
    let core_health = state.mihomo.health().await.ok();
    let tray_active = std::env::var("TESIE_TRAY_ACTIVE")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));

    Ok(json!({
        "settings": settings,
        "system": {
            "data_dir": data_dir.to_string_lossy(),
            "db_path": db_path.to_string_lossy(),
            "db_size_bytes": db_size_bytes,
            "auto_start_enabled": system::is_autostart_enabled(),
            "daemon_path": system::resolve_daemon_exe_path(&data_dir).to_string_lossy(),
            "tray_active": tray_active,
            "core_healthy": core_health.is_some(),
            "proxy_test_interval_sec": settings.proxy_test_interval_sec,
        },
        "security": {
            "has_admin_password": has_admin_password,
            "env_admin_token_active": env_token_active,
            "protection_enabled": env_token_active || has_admin_password,
        }
    }))
}

async fn get_settings(State(state): State<AdminState>) -> std::result::Result<Json<Value>, Response> {
    Ok(Json(build_settings_payload(&state).await?))
}

#[derive(Deserialize, Default)]
struct UpdateSettingsBody {
    pub strict_proxy_mode: Option<bool>,
    pub default_free_ip_window_ms: Option<i64>,
    #[serde(default, deserialize_with = "tesie_store::deserialize_double_option")]
    pub default_free_ip_max_requests: Option<Option<i64>>,
    #[serde(default, deserialize_with = "tesie_store::deserialize_double_option")]
    pub default_free_ip_max_tokens: Option<Option<i64>>,
    pub session_ttl_ms: Option<i64>,
    pub proxy_test_interval_sec: Option<u64>,
    pub exhaustive_failover: Option<bool>,
    #[serde(default, deserialize_with = "tesie_store::deserialize_double_option")]
    pub admin_password: Option<Option<String>>,
    #[serde(default)]
    pub clear_admin_password: Option<bool>,
}

async fn update_settings(
    State(state): State<AdminState>,
    Json(body): Json<UpdateSettingsBody>,
) -> std::result::Result<Json<Value>, Response> {
    let input = UpdateSettingsInput {
        strict_proxy_mode: body.strict_proxy_mode,
        default_free_ip_window_ms: body.default_free_ip_window_ms,
        default_free_ip_max_requests: body.default_free_ip_max_requests,
        default_free_ip_max_tokens: body.default_free_ip_max_tokens,
        session_ttl_ms: body.session_ttl_ms,
        proxy_test_interval_sec: body.proxy_test_interval_sec,
        exhaustive_failover: body.exhaustive_failover,
    };
    state
        .store
        .settings()
        .update_settings(input)
        .await
        .map_err(err_resp)?;

    if body.clear_admin_password.unwrap_or(false) {
        state
            .store
            .settings()
            .set_admin_password(None)
            .await
            .map_err(err_resp)?;
    } else if let Some(pw_opt) = body.admin_password {
        state
            .store
            .settings()
            .set_admin_password(pw_opt.as_deref())
            .await
            .map_err(err_resp)?;
    }

    state.engine.notify_probe_config_changed();
    Ok(Json(build_settings_payload(&state).await?))
}

#[derive(Deserialize, Default)]
struct BackupExportQuery {
    #[serde(default)]
    include_secrets: Option<bool>,
}

async fn export_backup(
    State(state): State<AdminState>,
    Query(q): Query<BackupExportQuery>,
) -> std::result::Result<Json<Value>, Response> {
    let include_secrets = q.include_secrets.unwrap_or(true);
    let doc = state
        .store
        .backup()
        .export_backup(include_secrets)
        .await
        .map_err(err_resp)?;
    Ok(Json(doc))
}

#[derive(Deserialize, Default)]
struct BackupImportQuery {
    #[serde(default)]
    replace: Option<bool>,
}

async fn import_backup(
    State(state): State<AdminState>,
    Query(q): Query<BackupImportQuery>,
    Json(payload): Json<Value>,
) -> std::result::Result<Json<Value>, Response> {
    let (doc_val, replace_opt) = if let Some(inner) = payload.get("backup") {
        (
            inner,
            payload
                .get("replace")
                .and_then(Value::as_bool)
                .or(q.replace),
        )
    } else {
        (
            &payload,
            payload
                .get("replace")
                .and_then(Value::as_bool)
                .or(q.replace),
        )
    };
    let replace = replace_opt.unwrap_or(false);
    let summary = state
        .store
        .backup()
        .import_backup(doc_val, replace)
        .await
        .map_err(err_resp)?;

    let _ = state.mihomo.reload_from_store(&state.store).await;
    state.engine.notify_probe_config_changed();
    state.engine.antigravity().notify_probe_config_changed();

    Ok(Json(json!({
        "ok": true,
        "replace": replace,
        "summary": summary
    })))
}

async fn reset_all_cooldowns_maintenance(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let now = now_ms();
    let cleared_ips = state
        .store
        .quotas()
        .reset_all_cooling(now)
        .await
        .map_err(err_resp)?;
    let cleared_connections = state
        .store
        .connections()
        .clear_all_cooldowns(now)
        .await
        .map_err(err_resp)?;
    let (cleared_model_locks, cleared_model_failures) =
        state.engine.antigravity().clear_all_model_cooldowns();

    Ok(Json(json!({
        "ok": true,
        "cleared_ips": cleared_ips,
        "cleared_connections": cleared_connections,
        "cleared_model_locks": cleared_model_locks,
        "cleared_model_failures": cleared_model_failures,
    })))
}

#[derive(Deserialize, Default)]
struct TokenSaverCategoriesPatch {
    git: Option<bool>,
    build: Option<bool>,
    test: Option<bool>,
    package: Option<bool>,
    docker: Option<bool>,
    system: Option<bool>,
}

#[derive(Deserialize, Default)]
struct UpdateTokenSaverInput {
    rtk_enabled: Option<bool>,
    rtk_intensity: Option<String>,
    rtk_max_lines: Option<usize>,
    rtk_deduplicate: Option<bool>,
    rtk_categories: Option<TokenSaverCategoriesPatch>,
    style_mode: Option<String>,
    style_level: Option<String>,
    custom_prompt: Option<String>,
}

async fn get_token_saver(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let config = state
        .store
        .token_saver()
        .get_config()
        .await
        .map_err(err_resp)?;
    let stats = state
        .store
        .token_saver()
        .get_stats()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "config": config, "stats": stats })))
}

async fn update_token_saver(
    State(state): State<AdminState>,
    Json(input): Json<UpdateTokenSaverInput>,
) -> std::result::Result<Json<Value>, Response> {
    let mut cfg = state
        .store
        .token_saver()
        .get_config()
        .await
        .map_err(err_resp)?;

    if let Some(v) = input.rtk_enabled {
        cfg.rtk_enabled = v;
    }
    if let Some(v) = input.rtk_intensity {
        let low = v.trim().to_ascii_lowercase();
        if matches!(low.as_str(), "minimal" | "standard" | "aggressive") {
            cfg.rtk_intensity = low;
        }
    }
    if let Some(v) = input.rtk_max_lines {
        cfg.rtk_max_lines = v.clamp(40, 2000);
    }
    if let Some(v) = input.rtk_deduplicate {
        cfg.rtk_deduplicate = v;
    }
    if let Some(cats) = input.rtk_categories {
        if let Some(v) = cats.git {
            cfg.rtk_categories.git = v;
        }
        if let Some(v) = cats.build {
            cfg.rtk_categories.build = v;
        }
        if let Some(v) = cats.test {
            cfg.rtk_categories.test = v;
        }
        if let Some(v) = cats.package {
            cfg.rtk_categories.package = v;
        }
        if let Some(v) = cats.docker {
            cfg.rtk_categories.docker = v;
        }
        if let Some(v) = cats.system {
            cfg.rtk_categories.system = v;
        }
    }
    if let Some(v) = input.style_mode {
        let low = v.trim().to_ascii_lowercase();
        if matches!(low.as_str(), "off" | "caveman" | "ponytail" | "adhd" | "custom") {
            cfg.style_mode = low;
        }
    }
    if let Some(v) = input.style_level {
        let low = v.trim().to_ascii_lowercase();
        if matches!(low.as_str(), "lite" | "full" | "ultra") {
            cfg.style_level = low;
        }
    }
    if let Some(v) = input.custom_prompt {
        cfg.custom_prompt = v;
    }

    let saved = state
        .store
        .token_saver()
        .save_config(&cfg)
        .await
        .map_err(err_resp)?;
    let stats = state
        .store
        .token_saver()
        .get_stats()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "config": saved, "stats": stats })))
}

#[derive(Deserialize)]
struct TestTokenSaverInput {
    text: String,
    tool_name: Option<String>,
    command: Option<String>,
    config: Option<TokenSaverConfig>,
}

async fn test_token_saver(
    State(state): State<AdminState>,
    Json(input): Json<TestTokenSaverInput>,
) -> std::result::Result<Json<Value>, Response> {
    let cfg = match input.config {
        Some(c) => c,
        None => state
            .store
            .token_saver()
            .get_config()
            .await
            .map_err(err_resp)?,
    };
    let result = compress_text_detailed(
        &input.text,
        input.tool_name.as_deref(),
        input.command.as_deref(),
        &cfg,
    );
    let style_prompt_preview =
        style_prompt_for(&cfg.style_mode, &cfg.style_level, &cfg.custom_prompt);
    Ok(Json(json!({
        "result": result,
        "style_prompt_preview": style_prompt_preview,
    })))
}

async fn clear_token_saver_stats(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let deleted = state
        .store
        .token_saver()
        .clear_stats()
        .await
        .map_err(err_resp)?;
    Ok(Json(json!({ "deleted": deleted })))
}

#[derive(Debug, Clone, Deserialize)]
struct UpdateBridgeInput {
    pub vision_enabled: Option<bool>,
    pub vision_model: Option<Option<String>>,
    pub vision_mode: Option<String>,
    pub vision_prompt: Option<Option<String>>,
    pub audio_enabled: Option<bool>,
    pub audio_model: Option<Option<String>>,
    pub audio_prompt: Option<Option<String>>,
}

async fn get_bridge_config(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let config = state.store.bridge().get_config().await.map_err(err_resp)?;
    Ok(Json(json!({ "config": config })))
}

async fn update_bridge_config(
    State(state): State<AdminState>,
    Json(input): Json<UpdateBridgeInput>,
) -> std::result::Result<Json<Value>, Response> {
    let mut cfg = state.store.bridge().get_config().await.map_err(err_resp)?;
    if let Some(v) = input.vision_enabled {
        cfg.vision_enabled = v;
    }
    if let Some(v) = input.vision_model {
        cfg.vision_model = v;
    }
    if let Some(v) = input.vision_mode {
        cfg.vision_mode = v;
    }
    if let Some(v) = input.vision_prompt {
        cfg.vision_prompt = v;
    }
    if let Some(v) = input.audio_enabled {
        cfg.audio_enabled = v;
    }
    if let Some(v) = input.audio_model {
        cfg.audio_model = v;
    }
    if let Some(v) = input.audio_prompt {
        cfg.audio_prompt = v;
    }
    state.store.bridge().save_config(&cfg).await.map_err(err_resp)?;
    Ok(Json(json!({ "config": cfg })))
}

async fn get_bridge_stats(
    State(state): State<AdminState>,
) -> std::result::Result<Json<Value>, Response> {
    let stats = state.engine.bridge_stats();
    Ok(Json(json!({ "stats": stats })))
}

#[derive(Debug, Clone, Deserialize)]
struct SimulateBridgeInput {
    pub transcriber_provider: Option<String>,
    pub target_provider: Option<String>,
    pub transcriber_model: Option<String>,
    pub target_model: Option<String>,
    pub kind: Option<String>,
}

async fn simulate_bridge_event(
    State(state): State<AdminState>,
    Json(input): Json<SimulateBridgeInput>,
) -> std::result::Result<Json<Value>, Response> {
    let event = tesie_core::BridgeEvent {
        transcriber_provider: input.transcriber_provider.unwrap_or_else(|| "antigravity".into()),
        target_provider: input.target_provider.unwrap_or_else(|| "opencode".into()),
        transcriber_model: input.transcriber_model.unwrap_or_else(|| "ag/gemini-3.8-flash".into()),
        target_model: input.target_model.unwrap_or_else(|| "oc/deepseek-v4.1-flash-free".into()),
        kind: input.kind.unwrap_or_else(|| "image".into()),
        timestamp_ms: tesie_store::now_ms(),
    };
    state.engine.record_bridge_event(event.clone());
    Ok(Json(json!({ "ok": true, "event": event })))
}

fn err_resp(err: TesRouteError) -> Response {
    let status = match &err {
        TesRouteError::BadRequest(_) => StatusCode::BAD_REQUEST,
        TesRouteError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
        TesRouteError::ModelQuotaExhausted(_) => StatusCode::FORBIDDEN,
        TesRouteError::NotFound(_) => StatusCode::NOT_FOUND,
        TesRouteError::Conflict(_) => StatusCode::CONFLICT,
        TesRouteError::ProxyRelay(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(json!({ "error": err.to_string() }))).into_response()
}
