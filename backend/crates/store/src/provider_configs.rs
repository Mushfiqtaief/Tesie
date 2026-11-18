use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{
    default_routing_strategy_for, normalize_routing_strategy, ProviderConfig, Result,
};

use crate::{map_sqlx_err, now_ms};

pub const MIN_PROBE_INTERVAL_SEC: i64 = 5;
pub const MAX_PROBE_INTERVAL_SEC: i64 = 86_400;
pub const DEFAULT_PROBE_INTERVAL_SEC: i64 = 30;

#[derive(Clone, Debug)]
pub struct ProviderConfigsRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateProviderConfigInput {
    pub enabled: Option<bool>,
    pub proxy_mode: Option<String>,
    pub selected_proxy_ids: Option<Vec<String>>,
    pub window_duration_ms: Option<i64>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub max_requests_per_window: Option<Option<i64>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub max_tokens_per_window: Option<Option<i64>>,
    pub session_ttl_ms: Option<i64>,
    pub thinking_level: Option<String>,
    pub probe_enabled: Option<bool>,
    pub probe_interval_sec: Option<i64>,
    pub routing_strategy: Option<String>,
}

impl ProviderConfigsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_configs(&self) -> Result<Vec<ProviderConfig>> {
        let rows = sqlx::query(
            "SELECT provider_id, enabled, proxy_mode, selected_proxy_ids_json, window_duration_ms,
                    max_requests_per_window, max_tokens_per_window, session_ttl_ms,
                    thinking_level, probe_enabled, probe_interval_sec, routing_strategy, updated_at
             FROM provider_configs
             ORDER BY provider_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        rows.iter().map(row_to_config).collect()
    }

    pub async fn get_or_default(&self, provider_id: &str) -> Result<ProviderConfig> {
        let row = sqlx::query(
            "SELECT provider_id, enabled, proxy_mode, selected_proxy_ids_json, window_duration_ms,
                    max_requests_per_window, max_tokens_per_window, session_ttl_ms,
                    thinking_level, probe_enabled, probe_interval_sec, routing_strategy, updated_at
             FROM provider_configs
             WHERE provider_id = ?",
        )
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        if let Some(r) = row {
            return row_to_config(&r);
        }

        let is_custom = provider_id.starts_with("cp-");
        let default_probe_interval = if provider_id == "antigravity" {
            600
        } else {
            DEFAULT_PROBE_INTERVAL_SEC
        };
        let default_proxy_mode = if is_custom { "direct" } else { "all" };

        Ok(ProviderConfig {
            provider_id: provider_id.to_string(),
            enabled: true,
            proxy_mode: default_proxy_mode.to_string(),
            selected_proxy_ids: Vec::new(),
            window_duration_ms: 3_600_000,
            max_requests_per_window: None,
            max_tokens_per_window: None,
            session_ttl_ms: 14_400_000,
            thinking_level: "auto".to_string(),
            probe_enabled: !is_custom,
            probe_interval_sec: default_probe_interval,
            routing_strategy: default_routing_strategy_for(provider_id).to_string(),
            updated_at: now_ms(),
        })
    }

    pub async fn delete_config(&self, provider_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM provider_configs WHERE provider_id = ?")
            .bind(provider_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn update_config(
        &self,
        provider_id: &str,
        input: UpdateProviderConfigInput,
    ) -> Result<ProviderConfig> {
        let current = self.get_or_default(provider_id).await?;
        let now = now_ms();

        let next_enabled = input.enabled.unwrap_or(current.enabled);
        let mut seen = HashSet::new();
        let next_selected_ids: Vec<String> = input
            .selected_proxy_ids
            .unwrap_or(current.selected_proxy_ids)
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && seen.insert(s.clone()))
            .collect();

        let requested_mode = input
            .proxy_mode
            .as_deref()
            .map(normalize_proxy_mode)
            .unwrap_or_else(|| current.proxy_mode.clone());

        // When mode is "selected" with 0 selected proxies, auto-resolve to "direct" (no proxy)
        let next_mode = if requested_mode == "selected" && next_selected_ids.is_empty() {
            "direct".to_string()
        } else {
            requested_mode
        };

        let selected_json =
            serde_json::to_string(&next_selected_ids).unwrap_or_else(|_| "[]".to_string());

        let next_window = input
            .window_duration_ms
            .filter(|&v| v > 0)
            .unwrap_or(current.window_duration_ms);
        let next_max_req = input
            .max_requests_per_window
            .map(|opt| opt.filter(|&v| v > 0))
            .unwrap_or(current.max_requests_per_window);
        let next_max_tok = input
            .max_tokens_per_window
            .map(|opt| opt.filter(|&v| v > 0))
            .unwrap_or(current.max_tokens_per_window);
        let next_ttl = input
            .session_ttl_ms
            .filter(|&v| v > 0)
            .unwrap_or(current.session_ttl_ms);
        let next_thinking = input
            .thinking_level
            .as_deref()
            .map(normalize_thinking_level)
            .unwrap_or_else(|| normalize_thinking_level(&current.thinking_level));
        let next_probe_enabled = input.probe_enabled.unwrap_or(current.probe_enabled);
        let next_probe_interval = input
            .probe_interval_sec
            .map(|s| s.clamp(MIN_PROBE_INTERVAL_SEC, MAX_PROBE_INTERVAL_SEC))
            .unwrap_or_else(|| {
                current
                    .probe_interval_sec
                    .clamp(MIN_PROBE_INTERVAL_SEC, MAX_PROBE_INTERVAL_SEC)
            });
        let default_strategy = default_routing_strategy_for(provider_id);
        let next_strategy = input
            .routing_strategy
            .as_deref()
            .map(|s| normalize_routing_strategy(s, default_strategy))
            .unwrap_or_else(|| normalize_routing_strategy(&current.routing_strategy, default_strategy));

        sqlx::query(
            "INSERT INTO provider_configs (
                provider_id, enabled, proxy_mode, selected_proxy_ids_json, window_duration_ms,
                max_requests_per_window, max_tokens_per_window, session_ttl_ms,
                thinking_level, probe_enabled, probe_interval_sec, routing_strategy, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(provider_id) DO UPDATE SET
                enabled = excluded.enabled,
                proxy_mode = excluded.proxy_mode,
                selected_proxy_ids_json = excluded.selected_proxy_ids_json,
                window_duration_ms = excluded.window_duration_ms,
                max_requests_per_window = excluded.max_requests_per_window,
                max_tokens_per_window = excluded.max_tokens_per_window,
                session_ttl_ms = excluded.session_ttl_ms,
                thinking_level = excluded.thinking_level,
                probe_enabled = excluded.probe_enabled,
                probe_interval_sec = excluded.probe_interval_sec,
                routing_strategy = excluded.routing_strategy,
                updated_at = excluded.updated_at",
        )
        .bind(provider_id)
        .bind(if next_enabled { 1_i64 } else { 0_i64 })
        .bind(&next_mode)
        .bind(&selected_json)
        .bind(next_window)
        .bind(next_max_req)
        .bind(next_max_tok)
        .bind(next_ttl)
        .bind(&next_thinking)
        .bind(if next_probe_enabled { 1_i64 } else { 0_i64 })
        .bind(next_probe_interval)
        .bind(&next_strategy)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        if provider_id == "opencode" {
            let strict_str = if matches!(next_mode.as_str(), "pool_strict" | "all" | "selected") {
                "true"
            } else {
                "false"
            };
            for (k, v) in [
                ("strict_proxy_mode", strict_str.to_string()),
                ("default_free_ip_window_ms", next_window.to_string()),
                (
                    "default_free_ip_max_requests",
                    next_max_req.map(|n| n.to_string()).unwrap_or_default(),
                ),
                (
                    "default_free_ip_max_tokens",
                    next_max_tok.map(|n| n.to_string()).unwrap_or_default(),
                ),
                ("session_ttl_ms", next_ttl.to_string()),
            ] {
                sqlx::query(
                    "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                )
                .bind(k)
                .bind(v)
                .bind(now)
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_err)?;
            }
        }

        self.get_or_default(provider_id).await
    }

    pub async fn prune_deleted_proxy_ids(&self, deleted_ids: &[String]) -> Result<()> {
        if deleted_ids.is_empty() {
            return Ok(());
        }
        let drop_set: HashSet<&str> = deleted_ids.iter().map(String::as_str).collect();
        for cfg in self.list_configs().await? {
            if cfg.selected_proxy_ids.is_empty() {
                continue;
            }
            let filtered: Vec<String> = cfg
                .selected_proxy_ids
                .iter()
                .filter(|id| !drop_set.contains(id.as_str()))
                .cloned()
                .collect();
            if filtered.len() != cfg.selected_proxy_ids.len() {
                let next_mode = if cfg.proxy_mode == "selected" && filtered.is_empty() {
                    Some("direct".to_string())
                } else {
                    None
                };
                self.update_config(
                    &cfg.provider_id,
                    UpdateProviderConfigInput {
                        proxy_mode: next_mode,
                        selected_proxy_ids: Some(filtered),
                        ..Default::default()
                    },
                )
                .await?;
            }
        }
        Ok(())
    }
}

pub fn normalize_thinking_level(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "disabled" | "off" => "none".to_string(),
        "minimal" => "minimal".to_string(),
        "low" => "low".to_string(),
        "medium" => "medium".to_string(),
        "high" => "high".to_string(),
        "xhigh" => "xhigh".to_string(),
        "max" => "max".to_string(),
        _ => "auto".to_string(),
    }
}

fn normalize_proxy_mode(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "all" => "all".to_string(),
        "selected" | "custom" => "selected".to_string(),
        "none" | "direct" => "direct".to_string(),
        "pool_strict" | "strict" => "pool_strict".to_string(),
        "pool_with_direct" | "pool_fallback" => "pool_with_direct".to_string(),
        "pool" => "all".to_string(),
        _ => "all".to_string(),
    }
}

fn row_to_config(row: &sqlx::sqlite::SqliteRow) -> Result<ProviderConfig> {
    let provider_id: String = row.try_get("provider_id").map_err(map_sqlx_err)?;
    let raw_ids: String = row
        .try_get("selected_proxy_ids_json")
        .unwrap_or_else(|_| "[]".to_string());
    let selected_proxy_ids: Vec<String> = serde_json::from_str(&raw_ids).unwrap_or_default();
    let raw_thinking: String = row
        .try_get("thinking_level")
        .unwrap_or_else(|_| "auto".to_string());
    let probe_enabled: bool = row
        .try_get::<i64, _>("probe_enabled")
        .map(|v| v != 0)
        .unwrap_or(true);
    let probe_interval_sec: i64 = row
        .try_get::<i64, _>("probe_interval_sec")
        .unwrap_or(DEFAULT_PROBE_INTERVAL_SEC)
        .clamp(MIN_PROBE_INTERVAL_SEC, MAX_PROBE_INTERVAL_SEC);
    let raw_strategy: String = row
        .try_get("routing_strategy")
        .unwrap_or_default();
    let default_strategy = default_routing_strategy_for(&provider_id);

    Ok(ProviderConfig {
        provider_id,
        enabled: row.try_get::<i64, _>("enabled").map_err(map_sqlx_err)? != 0,
        proxy_mode: row.try_get("proxy_mode").map_err(map_sqlx_err)?,
        selected_proxy_ids,
        window_duration_ms: row.try_get("window_duration_ms").map_err(map_sqlx_err)?,
        max_requests_per_window: row
            .try_get("max_requests_per_window")
            .map_err(map_sqlx_err)?,
        max_tokens_per_window: row
            .try_get("max_tokens_per_window")
            .map_err(map_sqlx_err)?,
        session_ttl_ms: row.try_get("session_ttl_ms").map_err(map_sqlx_err)?,
        thinking_level: normalize_thinking_level(&raw_thinking),
        probe_enabled,
        probe_interval_sec,
        routing_strategy: normalize_routing_strategy(&raw_strategy, default_strategy),
        updated_at: row.try_get("updated_at").map_err(map_sqlx_err)?,
    })
}
