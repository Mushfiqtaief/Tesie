use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{ProviderConnection, Result, TesRouteError};
use uuid::Uuid;

use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct ConnectionsRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateConnectionInput {
    pub provider_id: String,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub auth_data_json: Option<String>,
    #[serde(default)]
    pub proxy_node_id: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateConnectionInput {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub proxy_node_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub sticky_proxy_id: Option<Option<String>>,
    pub reset_cooldown: Option<bool>,
}

impl ConnectionsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_connections(&self) -> Result<Vec<ProviderConnection>> {
        let rows = sqlx::query(
            "SELECT id, provider_id, name, email, api_key, auth_data_json, quota_json,
                    proxy_node_id, sticky_proxy_id, enabled,
                    cooldown_until, backoff_level, last_error, created_at, updated_at
             FROM provider_connections
             ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        rows.iter().map(row_to_connection).collect()
    }

    pub async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderConnection>> {
        let rows = sqlx::query(
            "SELECT id, provider_id, name, email, api_key, auth_data_json, quota_json,
                    proxy_node_id, sticky_proxy_id, enabled,
                    cooldown_until, backoff_level, last_error, created_at, updated_at
             FROM provider_connections
             WHERE provider_id = ?
             ORDER BY created_at ASC, id ASC",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        rows.iter().map(row_to_connection).collect()
    }

    pub async fn list_enabled_for_provider(
        &self,
        provider_id: &str,
        now: i64,
    ) -> Result<Vec<ProviderConnection>> {
        sqlx::query(
            "UPDATE provider_connections
             SET cooldown_until = NULL, updated_at = ?
             WHERE cooldown_until IS NOT NULL AND cooldown_until <= ?",
        )
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let rows = sqlx::query(
            "SELECT id, provider_id, name, email, api_key, auth_data_json, quota_json,
                    proxy_node_id, sticky_proxy_id, enabled,
                    cooldown_until, backoff_level, last_error, created_at, updated_at
             FROM provider_connections
             WHERE provider_id = ? AND enabled = 1
               AND (cooldown_until IS NULL OR cooldown_until <= ?)
             ORDER BY created_at ASC, id ASC",
        )
        .bind(provider_id)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        rows.iter().map(row_to_connection).collect()
    }

    pub async fn create_connection(
        &self,
        input: CreateConnectionInput,
    ) -> Result<ProviderConnection> {
        let provider_id = input.provider_id.trim().to_ascii_lowercase();
        if provider_id != "opencode" && provider_id != "antigravity" {
            return Err(TesRouteError::BadRequest(format!(
                "unsupported provider_id {provider_id}; expected 'opencode' or 'antigravity'"
            )));
        }
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(TesRouteError::BadRequest(
                "connection name cannot be empty".into(),
            ));
        }

        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        let enabled = input.enabled.unwrap_or(true);

        sqlx::query(
            "INSERT INTO provider_connections (
                id, provider_id, name, email, api_key, auth_data_json, proxy_node_id, enabled,
                backoff_level, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(&provider_id)
        .bind(&name)
        .bind(input.email.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(input.api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(input.auth_data_json.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(input.proxy_node_id.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_connection(&id)
            .await?
            .ok_or_else(|| TesRouteError::Store("created connection missing".into()))
    }

    /// Upserts an OAuth account connection (e.g. Antigravity by deterministic account ID or email).
    pub async fn upsert_oauth_connection(
        &self,
        id: &str,
        provider_id: &str,
        name: &str,
        email: Option<&str>,
        access_token: &str,
        auth_data_json: &str,
    ) -> Result<ProviderConnection> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO provider_connections (
                id, provider_id, name, email, api_key, auth_data_json, enabled,
                backoff_level, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, 1, 0, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                email = COALESCE(excluded.email, provider_connections.email),
                api_key = excluded.api_key,
                auth_data_json = excluded.auth_data_json,
                cooldown_until = NULL,
                backoff_level = 0,
                last_error = NULL,
                updated_at = excluded.updated_at",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .bind(email.map(str::trim).filter(|s| !s.is_empty()))
        .bind(access_token)
        .bind(auth_data_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_connection(id)
            .await?
            .ok_or_else(|| TesRouteError::Store("upserted oauth connection missing".into()))
    }

    pub async fn update_auth_data(
        &self,
        id: &str,
        access_token: &str,
        auth_data_json: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE provider_connections
             SET api_key = ?, auth_data_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(access_token)
        .bind(auth_data_json)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn update_quota_snapshot(
        &self,
        id: &str,
        quota_json: &str,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE provider_connections
             SET quota_json = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(quota_json)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn update_sticky_proxy(
        &self,
        id: &str,
        sticky_proxy_id: Option<&str>,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE provider_connections
             SET sticky_proxy_id = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(sticky_proxy_id)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn record_cooldown(
        &self,
        id: &str,
        cooldown_until: i64,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        let truncated: String = reason.chars().take(512).collect();
        sqlx::query(
            "UPDATE provider_connections
             SET cooldown_until = ?,
                 backoff_level = MIN(backoff_level + 1, 5),
                 last_error = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(cooldown_until)
        .bind(truncated)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn clear_error(&self, id: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE provider_connections
             SET cooldown_until = NULL, backoff_level = 0, last_error = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn clear_all_cooldowns(&self, now: i64) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE provider_connections
             SET cooldown_until = NULL, backoff_level = 0, updated_at = ?
             WHERE cooldown_until IS NOT NULL OR backoff_level > 0",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(res.rows_affected())
    }

    /// Permanently disables a banned/ToS-suspended account (`enabled = 0`) and records the reason
    /// so it stops receiving traffic and its proxy slice redistributes immediately.
    pub async fn disable_banned_connection(
        &self,
        id: &str,
        reason: &str,
        now: i64,
    ) -> Result<()> {
        let truncated: String = reason.chars().take(512).collect();
        sqlx::query(
            "UPDATE provider_connections
             SET enabled = 0,
                 cooldown_until = NULL,
                 backoff_level = 0,
                 last_error = ?,
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(truncated)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn get_connection(&self, id: &str) -> Result<Option<ProviderConnection>> {
        let row = sqlx::query(
            "SELECT id, provider_id, name, email, api_key, auth_data_json, quota_json,
                    proxy_node_id, sticky_proxy_id, enabled,
                    cooldown_until, backoff_level, last_error, created_at, updated_at
             FROM provider_connections
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        row.as_ref().map(row_to_connection).transpose()
    }

    pub async fn update_connection(
        &self,
        id: &str,
        input: UpdateConnectionInput,
    ) -> Result<ProviderConnection> {
        let existing = self
            .get_connection(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("connection {id} not found")))?;

        let next_name = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&existing.name)
            .to_string();
        let next_enabled = input.enabled.unwrap_or(existing.enabled);
        let next_proxy_node_id = match input.proxy_node_id {
            Some(opt) => opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            None => existing.proxy_node_id,
        };
        let next_sticky_proxy_id = match input.sticky_proxy_id {
            Some(opt) => opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            None => existing.sticky_proxy_id,
        };
        let reset = input.reset_cooldown.unwrap_or(false)
            || (input.enabled == Some(true) && !existing.enabled);
        let next_cooldown = if reset { None } else { existing.cooldown_until };
        let next_backoff = if reset { 0 } else { existing.backoff_level };
        let next_error = if reset { None } else { existing.last_error };
        let now = now_ms();

        sqlx::query(
            "UPDATE provider_connections
             SET name = ?, proxy_node_id = ?, sticky_proxy_id = ?, enabled = ?,
                 cooldown_until = ?, backoff_level = ?, last_error = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&next_name)
        .bind(&next_proxy_node_id)
        .bind(&next_sticky_proxy_id)
        .bind(if next_enabled { 1_i64 } else { 0_i64 })
        .bind(next_cooldown)
        .bind(next_backoff)
        .bind(&next_error)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_connection(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("connection {id} not found")))
    }

    pub async fn delete_connection(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM provider_connections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if res.rows_affected() == 0 {
            return Err(TesRouteError::NotFound(format!("connection {id} not found")));
        }
        Ok(())
    }
}

fn row_to_connection(row: &sqlx::sqlite::SqliteRow) -> Result<ProviderConnection> {
    Ok(ProviderConnection {
        id: row.try_get("id").map_err(map_sqlx_err)?,
        provider_id: row.try_get("provider_id").map_err(map_sqlx_err)?,
        name: row.try_get("name").map_err(map_sqlx_err)?,
        email: row.try_get("email").ok().flatten(),
        api_key: row.try_get("api_key").map_err(map_sqlx_err)?,
        auth_data_json: row.try_get("auth_data_json").ok().flatten(),
        quota_json: row.try_get("quota_json").ok().flatten(),
        proxy_node_id: row.try_get("proxy_node_id").map_err(map_sqlx_err)?,
        sticky_proxy_id: row.try_get("sticky_proxy_id").ok().flatten(),
        enabled: row.try_get::<i64, _>("enabled").map_err(map_sqlx_err)? != 0,
        cooldown_until: row.try_get("cooldown_until").map_err(map_sqlx_err)?,
        backoff_level: row.try_get("backoff_level").map_err(map_sqlx_err)?,
        last_error: row.try_get("last_error").map_err(map_sqlx_err)?,
        created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx_err)?,
    })
}
