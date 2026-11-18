use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Row, SqlitePool};
use tesie_core::{
    ApiKeyRecord, ComboRecord, CustomProvider, GatewaySettings, ProviderConfig, ProviderConnection,
    ProxyNode, Result, TesRouteError, TokenSaverConfig,
};

use crate::custom_providers::CustomProvidersRepo;
use crate::provider_configs::{ProviderConfigsRepo, UpdateProviderConfigInput};
use crate::settings::{SettingsRepo, UpdateSettingsInput};
use crate::token_saver::TokenSaverRepo;
use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct BackupRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackupImportSummary {
    pub settings_updated: bool,
    pub token_saver_updated: bool,
    pub provider_configs: usize,
    pub custom_providers: usize,
    pub combos: usize,
    pub proxies: usize,
    pub connections: usize,
    pub api_keys: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct BackupDocument {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    settings: Option<GatewaySettings>,
    #[serde(default)]
    admin_password: Option<String>,
    #[serde(default)]
    token_saver_config: Option<TokenSaverConfig>,
    #[serde(default)]
    provider_configs: Vec<ProviderConfig>,
    #[serde(default)]
    custom_providers: Vec<CustomProvider>,
    #[serde(default)]
    combos: Vec<ComboRecord>,
    #[serde(default)]
    proxies: Vec<ProxyNode>,
    #[serde(default)]
    connections: Vec<ProviderConnection>,
    #[serde(default)]
    api_keys: Vec<ApiKeyRecord>,
}

fn default_version() -> u32 {
    1
}

impl BackupRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn export_backup(&self, include_secrets: bool) -> Result<Value> {
        let settings_repo = SettingsRepo::new(self.pool.clone());
        let settings = settings_repo.get_settings().await?;
        let admin_password = if include_secrets {
            settings_repo.get_admin_password().await?
        } else {
            None
        };
        let token_saver_config = TokenSaverRepo::new(self.pool.clone()).get_config().await?;
        let provider_configs = ProviderConfigsRepo::new(self.pool.clone())
            .list_configs()
            .await?;

        let mut custom_providers = CustomProvidersRepo::new(self.pool.clone()).list().await?;
        if !include_secrets {
            for cp in &mut custom_providers {
                cp.api_keys.clear();
            }
        }

        let combos = crate::combos::CombosRepo::new(self.pool.clone())
            .list_combos()
            .await?;
        let mut proxies = crate::proxies::ProxiesRepo::new(self.pool.clone())
            .list_proxies()
            .await?;
        if !include_secrets {
            for px in &mut proxies {
                px.password = None;
            }
        }

        let connections = if include_secrets {
            crate::connections::ConnectionsRepo::new(self.pool.clone())
                .list_connections()
                .await?
        } else {
            Vec::new()
        };

        let api_keys = if include_secrets {
            crate::keys::KeysRepo::new(self.pool.clone())
                .list_keys()
                .await?
        } else {
            Vec::new()
        };

        Ok(json!({
            "version": 1,
            "app": "Tesie",
            "exported_at": now_ms(),
            "include_secrets": include_secrets,
            "settings": settings,
            "admin_password": admin_password,
            "token_saver_config": token_saver_config,
            "provider_configs": provider_configs,
            "custom_providers": custom_providers,
            "combos": combos,
            "proxies": proxies,
            "connections": connections,
            "api_keys": api_keys
        }))
    }

    pub async fn import_backup(&self, payload: &Value, replace: bool) -> Result<BackupImportSummary> {
        let doc: BackupDocument = serde_json::from_value(payload.clone()).map_err(|e| {
            TesRouteError::BadRequest(format!("invalid Tesie backup JSON format: {e}"))
        })?;
        if doc.version == 0 || doc.version > 5 {
            return Err(TesRouteError::BadRequest(format!(
                "unsupported backup version {}",
                doc.version
            )));
        }

        let mut summary = BackupImportSummary::default();
        let now = now_ms();

        if replace {
            for sql in [
                "DELETE FROM custom_providers",
                "DELETE FROM combos",
                "DELETE FROM provider_connections",
                "DELETE FROM api_keys",
                "DELETE FROM proxy_nodes WHERE is_direct = 0",
            ] {
                sqlx::query(sql)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }

        if let Some(s) = doc.settings {
            SettingsRepo::new(self.pool.clone())
                .update_settings(UpdateSettingsInput {
                    strict_proxy_mode: Some(s.strict_proxy_mode),
                    default_free_ip_window_ms: Some(s.default_free_ip_window_ms),
                    default_free_ip_max_requests: Some(s.default_free_ip_max_requests),
                    default_free_ip_max_tokens: Some(s.default_free_ip_max_tokens),
                    session_ttl_ms: Some(s.session_ttl_ms),
                    proxy_test_interval_sec: Some(s.proxy_test_interval_sec),
                    exhaustive_failover: Some(s.exhaustive_failover),
                })
                .await?;
            summary.settings_updated = true;
        }
        if let Some(pw) = doc.admin_password {
            SettingsRepo::new(self.pool.clone())
                .set_admin_password(Some(&pw))
                .await?;
        }
        if let Some(ts) = doc.token_saver_config {
            TokenSaverRepo::new(self.pool.clone())
                .save_config(&ts)
                .await?;
            summary.token_saver_updated = true;
        }

        // Import proxies before provider_configs/connections so foreign references match cleanly
        for px in doc.proxies {
            if px.is_direct || px.name.eq_ignore_ascii_case("DIRECT") {
                continue;
            }
            sqlx::query(
                "INSERT INTO proxy_nodes (
                    id, name, proxy_type, proxy_url, raw_outbound_json, is_direct, enabled, alive,
                    egress_ip, last_delay_ms, last_probed_at, consecutive_failures, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(name) DO UPDATE SET
                    proxy_type = excluded.proxy_type,
                    proxy_url = excluded.proxy_url,
                    raw_outbound_json = excluded.raw_outbound_json,
                    enabled = excluded.enabled,
                    updated_at = excluded.updated_at",
            )
            .bind(&px.id)
            .bind(&px.name)
            .bind(&px.proxy_type)
            .bind(&px.proxy_url)
            .bind(&px.raw_outbound_json)
            .bind(if px.enabled { 1_i64 } else { 0_i64 })
            .bind(if px.alive { 1_i64 } else { 0_i64 })
            .bind(&px.egress_ip)
            .bind(px.last_delay_ms)
            .bind(px.last_probed_at)
            .bind(px.consecutive_failures)
            .bind(if px.created_at > 0 { px.created_at } else { now })
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
            summary.proxies += 1;
        }

        let cp_repo = CustomProvidersRepo::new(self.pool.clone());
        for cp in doc.custom_providers {
            cp_repo.insert_or_replace_raw(cp).await?;
            summary.custom_providers += 1;
        }

        let pc_repo = ProviderConfigsRepo::new(self.pool.clone());
        for pc in doc.provider_configs {
            pc_repo
                .update_config(
                    &pc.provider_id,
                    UpdateProviderConfigInput {
                        enabled: Some(pc.enabled),
                        proxy_mode: Some(pc.proxy_mode),
                        selected_proxy_ids: Some(pc.selected_proxy_ids),
                        window_duration_ms: Some(pc.window_duration_ms),
                        max_requests_per_window: Some(pc.max_requests_per_window),
                        max_tokens_per_window: Some(pc.max_tokens_per_window),
                        session_ttl_ms: Some(pc.session_ttl_ms),
                        thinking_level: Some(pc.thinking_level),
                        probe_enabled: Some(pc.probe_enabled),
                        probe_interval_sec: Some(pc.probe_interval_sec),
                        routing_strategy: Some(pc.routing_strategy),
                    },
                )
                .await?;
            summary.provider_configs += 1;
        }

        for combo in doc.combos {
            let models_json =
                serde_json::to_string(&combo.models).unwrap_or_else(|_| "[]".to_string());
            sqlx::query(
                "INSERT INTO combos (id, name, strategy, models_json, enabled, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(name) DO UPDATE SET
                    strategy = excluded.strategy,
                    models_json = excluded.models_json,
                    enabled = excluded.enabled,
                    updated_at = excluded.updated_at",
            )
            .bind(&combo.id)
            .bind(&combo.name)
            .bind(&combo.strategy)
            .bind(&models_json)
            .bind(if combo.enabled { 1_i64 } else { 0_i64 })
            .bind(if combo.created_at > 0 { combo.created_at } else { now })
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
            summary.combos += 1;
        }

        for conn in doc.connections {
            // Check whether referenced proxy_node_id / sticky_proxy_id exists before binding FK
            let valid_proxy_node_id = if let Some(ref pid) = conn.proxy_node_id {
                let exists = sqlx::query("SELECT 1 FROM proxy_nodes WHERE id = ?")
                    .bind(pid)
                    .fetch_optional(&self.pool)
                    .await
                    .ok()
                    .flatten()
                    .is_some();
                exists.then(|| pid.clone())
            } else {
                None
            };
            sqlx::query(
                "INSERT INTO provider_connections (
                    id, provider_id, name, email, api_key, auth_data_json, quota_json,
                    proxy_node_id, sticky_proxy_id, enabled, cooldown_until, backoff_level,
                    last_error, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    email = excluded.email,
                    api_key = COALESCE(excluded.api_key, provider_connections.api_key),
                    auth_data_json = COALESCE(excluded.auth_data_json, provider_connections.auth_data_json),
                    quota_json = COALESCE(excluded.quota_json, provider_connections.quota_json),
                    proxy_node_id = excluded.proxy_node_id,
                    sticky_proxy_id = excluded.sticky_proxy_id,
                    enabled = excluded.enabled,
                    updated_at = excluded.updated_at",
            )
            .bind(&conn.id)
            .bind(&conn.provider_id)
            .bind(&conn.name)
            .bind(&conn.email)
            .bind(&conn.api_key)
            .bind(&conn.auth_data_json)
            .bind(&conn.quota_json)
            .bind(valid_proxy_node_id)
            .bind(&conn.sticky_proxy_id)
            .bind(if conn.enabled { 1_i64 } else { 0_i64 })
            .bind(conn.cooldown_until)
            .bind(conn.backoff_level)
            .bind(&conn.last_error)
            .bind(if conn.created_at > 0 { conn.created_at } else { now })
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
            summary.connections += 1;
        }

        for key in doc.api_keys {
            if key.key_hash.trim().is_empty() {
                continue;
            }
            let existing_by_hash = sqlx::query("SELECT id FROM api_keys WHERE key_hash = ?")
                .bind(&key.key_hash)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx_err)?;
            if let Some(row) = existing_by_hash {
                let existing_id: String = row.try_get("id").map_err(map_sqlx_err)?;
                sqlx::query(
                    "UPDATE api_keys
                     SET name = ?, quota_tokens_limit = ?, quota_requests_limit = ?, enabled = ?
                     WHERE id = ?",
                )
                .bind(&key.name)
                .bind(key.quota_tokens_limit)
                .bind(key.quota_requests_limit)
                .bind(if key.enabled { 1_i64 } else { 0_i64 })
                .bind(&existing_id)
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_err)?;
            } else {
                sqlx::query(
                    "INSERT INTO api_keys (
                        id, name, key_hash, key_prefix, raw_key,
                        quota_tokens_limit, quota_tokens_used,
                        quota_requests_limit, quota_requests_used,
                        enabled, created_at, last_used_at
                     ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&key.id)
                .bind(&key.name)
                .bind(&key.key_hash)
                .bind(&key.key_prefix)
                .bind(&key.raw_key)
                .bind(key.quota_tokens_limit)
                .bind(key.quota_tokens_used)
                .bind(key.quota_requests_limit)
                .bind(key.quota_requests_used)
                .bind(if key.enabled { 1_i64 } else { 0_i64 })
                .bind(if key.created_at > 0 { key.created_at } else { now })
                .bind(key.last_used_at)
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_err)?;
            }
            summary.api_keys += 1;
        }

        Ok(summary)
    }
}
