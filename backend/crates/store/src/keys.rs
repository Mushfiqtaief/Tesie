use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tesie_core::{ApiKeyRecord, Result, TesRouteError};
use uuid::Uuid;

use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct KeysRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateApiKeyInput {
    pub name: String,
    pub quota_tokens_limit: Option<i64>,
    pub quota_requests_limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateApiKeyInput {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub quota_tokens_limit: Option<Option<i64>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub quota_requests_limit: Option<Option<i64>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatedApiKey {
    pub record: ApiKeyRecord,
    pub raw_key: String,
}

impl KeysRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_key(&self, input: CreateApiKeyInput) -> Result<CreatedApiKey> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(TesRouteError::BadRequest(
                "api key name cannot be empty".into(),
            ));
        }

        let mut random_bytes = [0_u8; 16];
        rand::thread_rng().fill_bytes(&mut random_bytes);
        let raw_key = format!("sk-tesie-{}", hex::encode(random_bytes));
        let key_hash = hash_api_key(&raw_key);
        let key_prefix = raw_key.chars().take(12).collect::<String>();
        let id = Uuid::new_v4().to_string();
        let now = now_ms();

        sqlx::query(
            "INSERT INTO api_keys (
                id, name, key_hash, key_prefix, raw_key,
                quota_tokens_limit, quota_tokens_used,
                quota_requests_limit, quota_requests_used,
                enabled, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, 0, ?, 0, 1, ?)",
        )
        .bind(&id)
        .bind(&name)
        .bind(&key_hash)
        .bind(&key_prefix)
        .bind(&raw_key)
        .bind(input.quota_tokens_limit)
        .bind(input.quota_requests_limit)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let record = self
            .get_by_id(&id)
            .await?
            .ok_or_else(|| TesRouteError::Store("created key missing".into()))?;
        Ok(CreatedApiKey { record, raw_key })
    }

    pub async fn list_keys(&self) -> Result<Vec<ApiKeyRecord>> {
        let rows = sqlx::query(
            "SELECT id, name, key_hash, key_prefix, raw_key,
                    quota_tokens_limit, quota_tokens_used,
                    quota_requests_limit, quota_requests_used,
                    enabled, created_at, last_used_at
             FROM api_keys
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        rows.iter().map(row_to_key).collect()
    }

    pub async fn has_any_enabled_keys(&self) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM api_keys WHERE enabled = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(count > 0)
    }

    pub async fn verify_and_check_quota(&self, raw_key: &str) -> Result<ApiKeyRecord> {
        let hash = hash_api_key(raw_key.trim());
        let row = sqlx::query(
            "SELECT id, name, key_hash, key_prefix, raw_key,
                    quota_tokens_limit, quota_tokens_used,
                    quota_requests_limit, quota_requests_used,
                    enabled, created_at, last_used_at
             FROM api_keys
             WHERE key_hash = ?",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let record = row
            .as_ref()
            .map(row_to_key)
            .transpose()?
            .ok_or_else(|| TesRouteError::Unauthorized("invalid API key".into()))?;

        if !record.enabled {
            return Err(TesRouteError::Unauthorized("API key is disabled".into()));
        }

        if let Some(req_limit) = record.quota_requests_limit
            && req_limit >= 0
            && record.quota_requests_used >= req_limit
        {
            return Err(TesRouteError::ApiKeyQuotaExceeded(format!(
                "request quota reached ({}/{})",
                record.quota_requests_used, req_limit
            )));
        }

        if let Some(tok_limit) = record.quota_tokens_limit
            && tok_limit >= 0
            && record.quota_tokens_used >= tok_limit
        {
            return Err(TesRouteError::ApiKeyQuotaExceeded(format!(
                "token quota reached ({}/{})",
                record.quota_tokens_used, tok_limit
            )));
        }

        Ok(record)
    }

    pub async fn add_usage(&self, id: &str, tokens: i64, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE api_keys
             SET quota_requests_used = quota_requests_used + 1,
                 quota_tokens_used = quota_tokens_used + ?,
                 last_used_at = ?
             WHERE id = ?",
        )
        .bind(tokens.max(0))
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn update_key(&self, id: &str, input: UpdateApiKeyInput) -> Result<ApiKeyRecord> {
        let existing = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("api key {id} not found")))?;

        let next_name = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&existing.name)
            .to_string();
        let next_enabled = input.enabled.unwrap_or(existing.enabled);
        let next_tokens_limit = input
            .quota_tokens_limit
            .map(|opt| opt.filter(|&v| v > 0))
            .unwrap_or(existing.quota_tokens_limit);
        let next_requests_limit = input
            .quota_requests_limit
            .map(|opt| opt.filter(|&v| v > 0))
            .unwrap_or(existing.quota_requests_limit);

        sqlx::query(
            "UPDATE api_keys
             SET name = ?, enabled = ?, quota_tokens_limit = ?, quota_requests_limit = ?
             WHERE id = ?",
        )
        .bind(&next_name)
        .bind(if next_enabled { 1_i64 } else { 0_i64 })
        .bind(next_tokens_limit)
        .bind(next_requests_limit)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("api key {id} not found")))
    }

    pub async fn delete_key(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM api_keys WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if res.rows_affected() == 0 {
            return Err(TesRouteError::NotFound(format!("api key {id} not found")));
        }
        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<ApiKeyRecord>> {
        let row = sqlx::query(
            "SELECT id, name, key_hash, key_prefix, raw_key,
                    quota_tokens_limit, quota_tokens_used,
                    quota_requests_limit, quota_requests_used,
                    enabled, created_at, last_used_at
             FROM api_keys
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        row.as_ref().map(row_to_key).transpose()
    }
}

pub fn hash_api_key(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

fn row_to_key(row: &sqlx::sqlite::SqliteRow) -> Result<ApiKeyRecord> {
    Ok(ApiKeyRecord {
        id: row.try_get("id").map_err(map_sqlx_err)?,
        name: row.try_get("name").map_err(map_sqlx_err)?,
        key_hash: row.try_get("key_hash").map_err(map_sqlx_err)?,
        key_prefix: row.try_get("key_prefix").map_err(map_sqlx_err)?,
        raw_key: row.try_get("raw_key").map_err(map_sqlx_err)?,
        quota_tokens_limit: row.try_get("quota_tokens_limit").map_err(map_sqlx_err)?,
        quota_tokens_used: row.try_get("quota_tokens_used").map_err(map_sqlx_err)?,
        quota_requests_limit: row.try_get("quota_requests_limit").map_err(map_sqlx_err)?,
        quota_requests_used: row.try_get("quota_requests_used").map_err(map_sqlx_err)?,
        enabled: row.try_get::<i64, _>("enabled").map_err(map_sqlx_err)? != 0,
        created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
        last_used_at: row.try_get("last_used_at").map_err(map_sqlx_err)?,
    })
}
