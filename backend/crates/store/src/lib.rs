pub mod backup;
pub mod bridge;
pub mod combos;
pub mod connections;
pub mod custom_providers;
pub mod keys;
pub mod provider_configs;
pub mod proxies;
pub mod quotas;
pub mod settings;
pub mod token_saver;
pub mod usage;

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use tesie_core::{Result, TesRouteError};

use backup::BackupRepo;
use bridge::BridgeRepo;
use combos::CombosRepo;
use connections::ConnectionsRepo;
use custom_providers::CustomProvidersRepo;
use keys::KeysRepo;
use provider_configs::ProviderConfigsRepo;
use proxies::ProxiesRepo;
use quotas::QuotasRepo;
use settings::SettingsRepo;
use token_saver::TokenSaverRepo;
use usage::UsageRepo;

const INIT_SQL: &str = include_str!("../migrations/0001_init.sql");

#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn connect_file(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| TesRouteError::Store(format!("create db dir: {e}")))?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(map_sqlx_err)?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn connect_url(database_url: &str) -> Result<Self> {
        let is_memory = database_url == ":memory:"
            || database_url.starts_with("sqlite::memory:")
            || database_url.contains("mode=memory");
        let mut opts = SqliteConnectOptions::from_str(database_url)
            .map_err(map_sqlx_err)?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        if !is_memory {
            opts = opts
                .journal_mode(SqliteJournalMode::Wal)
                .synchronous(SqliteSynchronous::Normal);
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(if is_memory { 1 } else { 8 })
            .connect_with(opts)
            .await
            .map_err(map_sqlx_err)?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn connect_in_memory() -> Result<Self> {
        Self::connect_url("sqlite::memory:").await
    }

    async fn migrate(&self) -> Result<()> {
        self.ensure_proxy_node_columns_before_init().await?;
        sqlx::raw_sql(INIT_SQL)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        self.ensure_proxy_node_columns().await?;
        self.ensure_ip_quotas_columns().await?;
        self.ensure_provider_configs_columns().await?;
        self.ensure_provider_connections_columns().await?;
        self.ensure_usage_log_columns().await?;
        self.ensure_api_keys_columns().await?;
        self.ensure_global_proxies_enabled().await?;
        self.seed_provider_configs().await?;
        self.migrate_legacy_proxy_names_once().await?;
        self.recover_probe_burned_cooldowns_once().await?;
        self.clamp_excessive_cooldowns_on_startup().await?;
        self.normalize_uncached_prompt_tokens_once().await?;
        self.auto_disable_tos_connections_once().await?;
        self.relax_antigravity_probe_interval_once().await?;
        self.ensure_token_saver_table().await?;
        self.ensure_custom_providers_table().await?;
        Ok(())
    }

    async fn ensure_custom_providers_table(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS custom_providers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                prefix TEXT NOT NULL UNIQUE,
                protocol TEXT NOT NULL DEFAULT 'openai',
                api_type TEXT NOT NULL DEFAULT 'chat',
                base_url TEXT NOT NULL,
                api_keys_json TEXT NOT NULL DEFAULT '[]',
                models_json TEXT NOT NULL DEFAULT '[]',
                custom_headers_json TEXT NOT NULL DEFAULT '{}',
                favicon_url TEXT,
                color TEXT NOT NULL DEFAULT '#0EA5E9',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn ensure_token_saver_table(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS token_saver_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at INTEGER NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                category TEXT NOT NULL,
                original_tokens INTEGER NOT NULL,
                compressed_tokens INTEGER NOT NULL,
                saved_tokens INTEGER NOT NULL,
                style_mode TEXT NOT NULL DEFAULT 'off'
            )",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_token_saver_logs_created ON token_saver_logs(created_at DESC)",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn ensure_proxy_node_columns_before_init(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(proxy_nodes)")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if rows.is_empty() {
            return Ok(());
        }
        let existing: HashSet<String> = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();

        for (col, sql_type) in [
            ("proxy_type", "TEXT NOT NULL DEFAULT 'socks5'"),
            ("alive", "INTEGER NOT NULL DEFAULT 1"),
        ] {
            if !existing.contains(col) {
                let stmt = format!("ALTER TABLE proxy_nodes ADD COLUMN {col} {sql_type}");
                sqlx::query(&stmt)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }
        Ok(())
    }

    async fn ensure_proxy_node_columns(&self) -> Result<()> {
        let rows = sqlx::query(
            "SELECT id, is_direct, proxy_type, proxy_url, raw_outbound_json FROM proxy_nodes",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        for row in rows {
            let id: String = row.try_get("id").map_err(map_sqlx_err)?;
            let is_direct = row.try_get::<i64, _>("is_direct").map_err(map_sqlx_err)? != 0;
            let current_type: String = row.try_get("proxy_type").map_err(map_sqlx_err)?;
            let proxy_url: Option<String> = row.try_get("proxy_url").map_err(map_sqlx_err)?;
            let raw_json: Option<String> = row.try_get("raw_outbound_json").map_err(map_sqlx_err)?;
            let (inferred_type, _) = proxies::infer_proxy_type_and_host(
                is_direct,
                proxy_url.as_deref(),
                raw_json.as_deref(),
            );
            if current_type != inferred_type {
                sqlx::query("UPDATE proxy_nodes SET proxy_type = ? WHERE id = ?")
                    .bind(&inferred_type)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }
        Ok(())
    }

    async fn ensure_ip_quotas_columns(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(ip_quotas)")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let existing: HashSet<String> = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();

        if !existing.contains("last_quota_checked_at") {
            sqlx::query("ALTER TABLE ip_quotas ADD COLUMN last_quota_checked_at INTEGER")
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_err)?;
        }
        Ok(())
    }

    async fn ensure_provider_configs_columns(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(provider_configs)")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let existing: HashSet<String> = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();

        for (col, sql_type) in [
            ("selected_proxy_ids_json", "TEXT NOT NULL DEFAULT '[]'"),
            ("thinking_level", "TEXT NOT NULL DEFAULT 'auto'"),
            ("probe_enabled", "INTEGER NOT NULL DEFAULT 1"),
            ("probe_interval_sec", "INTEGER NOT NULL DEFAULT 30"),
            ("routing_strategy", "TEXT NOT NULL DEFAULT ''"),
        ] {
            if !existing.contains(col) {
                let stmt = format!("ALTER TABLE provider_configs ADD COLUMN {col} {sql_type}");
                sqlx::query(&stmt)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }
        Ok(())
    }

    async fn ensure_provider_connections_columns(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(provider_connections)")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let existing: HashSet<String> = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();

        for (col, sql_type) in [
            ("email", "TEXT"),
            ("auth_data_json", "TEXT"),
            ("quota_json", "TEXT"),
            ("sticky_proxy_id", "TEXT"),
        ] {
            if !existing.contains(col) {
                let stmt = format!("ALTER TABLE provider_connections ADD COLUMN {col} {sql_type}");
                sqlx::query(&stmt)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }
        Ok(())
    }

    async fn ensure_usage_log_columns(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(usage_logs)")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let existing: HashSet<String> = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();

        for (col, sql_type) in [
            ("cached_tokens", "INTEGER NOT NULL DEFAULT 0"),
            ("ttft_ms", "INTEGER"),
            ("tokens_per_sec", "REAL"),
            ("error_detail", "TEXT"),
            ("upstream_url", "TEXT"),
            ("attempt_count", "INTEGER NOT NULL DEFAULT 1"),
            ("attempt_trace", "TEXT"),
            ("session_id", "TEXT"),
            ("wire_format", "TEXT"),
        ] {
            if !existing.contains(col) {
                let stmt = format!("ALTER TABLE usage_logs ADD COLUMN {col} {sql_type}");
                sqlx::query(&stmt)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }
        Ok(())
    }

    async fn ensure_api_keys_columns(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(api_keys)")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let existing: HashSet<String> = rows
            .into_iter()
            .filter_map(|r| r.try_get::<String, _>("name").ok())
            .collect();

        if !existing.contains("raw_key") {
            sqlx::query("ALTER TABLE api_keys ADD COLUMN raw_key TEXT")
                .execute(&self.pool)
                .await
                .map_err(map_sqlx_err)?;
        }
        Ok(())
    }

    async fn ensure_global_proxies_enabled(&self) -> Result<()> {
        sqlx::query("UPDATE proxy_nodes SET enabled = 1 WHERE enabled = 0")
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn seed_provider_configs(&self) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('proxy_test_interval_sec', '15', ?)",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let settings = SettingsRepo::new(self.pool.clone()).get_settings().await?;
        let _ = settings;

        sqlx::query(
            "INSERT OR IGNORE INTO provider_configs (
                provider_id, enabled, proxy_mode, selected_proxy_ids_json, window_duration_ms,
                max_requests_per_window, max_tokens_per_window, session_ttl_ms, updated_at
             ) VALUES ('opencode', 1, 'all', '[]', ?, ?, ?, ?, ?)",
        )
        .bind(settings.default_free_ip_window_ms)
        .bind(settings.default_free_ip_max_requests)
        .bind(settings.default_free_ip_max_tokens)
        .bind(settings.session_ttl_ms)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        // Upgrade legacy opencode proxy_mode ('pool_with_direct' / 'pool_strict') to 'all' once
        sqlx::query(
            "UPDATE provider_configs SET proxy_mode = 'all'
             WHERE provider_id = 'opencode' AND proxy_mode IN ('pool_with_direct', 'pool_strict', 'pool')",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT OR IGNORE INTO provider_configs (
                provider_id, enabled, proxy_mode, selected_proxy_ids_json, window_duration_ms,
                max_requests_per_window, max_tokens_per_window, session_ttl_ms, probe_interval_sec, updated_at
             ) VALUES ('antigravity', 1, 'all', '[]', 18000000, NULL, NULL, 14400000, 600, ?)",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT OR IGNORE INTO provider_configs (
                provider_id, enabled, proxy_mode, selected_proxy_ids_json, window_duration_ms,
                max_requests_per_window, max_tokens_per_window, session_ttl_ms, probe_interval_sec, routing_strategy, updated_at
             ) VALUES ('ovh', 1, 'all', '[]', 60000, 2, NULL, 14400000, 60, 'round-robin', ?)",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        // Remove deprecated OpenCode Zen & OpenCode Go rows so only OpenCode Free & Antigravity remain
        sqlx::query("DELETE FROM provider_configs WHERE provider_id IN ('opencode-zen', 'opencode-go')")
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        sqlx::query("DELETE FROM provider_connections WHERE provider_id IN ('opencode-zen', 'opencode-go')")
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        Ok(())
    }

    async fn migrate_legacy_proxy_names_once(&self) -> Result<()> {
        let already = sqlx::query("SELECT value FROM settings WHERE key = 'migrated_node_names_v1'")
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if already.is_some() {
            return Ok(());
        }

        let _ = self.proxies().normalize_all_external_names_to_nodes().await?;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ('migrated_node_names_v1', '1', ?)
             ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn recover_probe_burned_cooldowns_once(&self) -> Result<()> {
        let already = sqlx::query(
            "SELECT value FROM settings WHERE key = 'recovered_probe_burned_cooldowns_v2'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        if already.is_some() {
            return Ok(());
        }

        let now = now_ms();
        // Reset exit IPs that have zero user requests (`total_requests = 0`) and were only
        // placed into cooldown by synthetic background/batch probes.
        sqlx::query(
            "UPDATE ip_quotas
             SET status = 'active',
                 cooldown_until = NULL,
                 backoff_level = 0,
                 last_error = NULL,
                 updated_at = ?
             WHERE status = 'cooldown' AND total_requests = 0",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ('recovered_probe_burned_cooldowns_v2', '1', ?)
             ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn clamp_excessive_cooldowns_on_startup(&self) -> Result<()> {
        let now = now_ms();
        let max_cooldown_until = now + 30 * 60 * 1000;
        sqlx::query(
            "UPDATE ip_quotas
             SET cooldown_until = ?, updated_at = ?
             WHERE status = 'cooldown' AND cooldown_until IS NOT NULL AND cooldown_until > ?",
        )
        .bind(max_cooldown_until)
        .bind(now)
        .bind(max_cooldown_until)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn normalize_uncached_prompt_tokens_once(&self) -> Result<()> {
        let already = sqlx::query(
            "SELECT value FROM settings WHERE key = 'normalize_uncached_prompt_tokens_v1'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        if already.is_some() {
            return Ok(());
        }

        let now = now_ms();
        // Historical logs stored gross prompt_tokens (uncached + cached); normalize so `prompt_tokens`
        // represents uncached input tokens only (matching 9router) while `cached_tokens` stays separate.
        sqlx::query(
            "UPDATE usage_logs
             SET prompt_tokens = MAX(0, prompt_tokens - cached_tokens),
                 total_tokens = MAX(0, prompt_tokens - cached_tokens) + completion_tokens
             WHERE cached_tokens > 0 AND prompt_tokens >= cached_tokens",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ('normalize_uncached_prompt_tokens_v1', '1', ?)
             ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn auto_disable_tos_connections_once(&self) -> Result<()> {
        let already = sqlx::query(
            "SELECT value FROM settings WHERE key = 'auto_disable_tos_connections_v1'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        if already.is_some() {
            return Ok(());
        }

        let now = now_ms();
        sqlx::query(
            "UPDATE provider_connections
             SET enabled = 0,
                 cooldown_until = NULL,
                 backoff_level = 0,
                 last_error = 'Banned: Google Cloud Code service disabled in this account for violation of Terms of Service.',
                 updated_at = ?
             WHERE LOWER(COALESCE(last_error, '')) LIKE '%service disabled%'
                OR LOWER(COALESCE(last_error, '')) LIKE '%service has been disabled%'
                OR LOWER(COALESCE(last_error, '')) LIKE '%terms of service%'",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ('auto_disable_tos_connections_v1', '1', ?)
             ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn relax_antigravity_probe_interval_once(&self) -> Result<()> {
        let already = sqlx::query(
            "SELECT value FROM settings WHERE key = 'migration_ag_probe_600s_v1'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        if already.is_some() {
            return Ok(());
        }

        let now = now_ms();
        // Relax aggressive 30s Antigravity quota polling to 10 minutes (600s) to avoid automated spam flags.
        sqlx::query(
            "UPDATE provider_configs
             SET probe_interval_sec = 600, updated_at = ?
             WHERE provider_id = 'antigravity' AND probe_interval_sec <= 30",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES ('migration_ag_probe_600s_v1', '1', ?)
             ON CONFLICT(key) DO UPDATE SET value = '1', updated_at = excluded.updated_at",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn proxies(&self) -> ProxiesRepo {
        ProxiesRepo::new(self.pool.clone())
    }

    pub fn quotas(&self) -> QuotasRepo {
        QuotasRepo::new(self.pool.clone())
    }

    pub fn connections(&self) -> ConnectionsRepo {
        ConnectionsRepo::new(self.pool.clone())
    }

    pub fn keys(&self) -> KeysRepo {
        KeysRepo::new(self.pool.clone())
    }

    pub fn usage(&self) -> UsageRepo {
        UsageRepo::new(self.pool.clone())
    }

    pub fn settings(&self) -> SettingsRepo {
        SettingsRepo::new(self.pool.clone())
    }

    pub fn provider_configs(&self) -> ProviderConfigsRepo {
        ProviderConfigsRepo::new(self.pool.clone())
    }

    pub fn combos(&self) -> CombosRepo {
        CombosRepo::new(self.pool.clone())
    }

    pub fn token_saver(&self) -> TokenSaverRepo {
        TokenSaverRepo::new(self.pool.clone())
    }

    pub fn custom_providers(&self) -> CustomProvidersRepo {
        CustomProvidersRepo::new(self.pool.clone())
    }

    pub fn bridge(&self) -> BridgeRepo {
        BridgeRepo::new(self.pool.clone())
    }

    pub fn backup(&self) -> BackupRepo {
        BackupRepo::new(self.pool.clone())
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(crate) fn map_sqlx_err(err: sqlx::Error) -> TesRouteError {
    TesRouteError::Store(err.to_string())
}

/// Deserializes an optional nullable JSON field (`Some(Some(val))`, `Some(None)` on explicit
/// `null`, or `None` when the key is absent).
pub fn deserialize_double_option<'de, T, D>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}
