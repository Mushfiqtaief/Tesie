use std::collections::HashMap;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{GatewaySettings, Result, TesRouteError};

use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct SettingsRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateSettingsInput {
    pub strict_proxy_mode: Option<bool>,
    pub default_free_ip_window_ms: Option<i64>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub default_free_ip_max_requests: Option<Option<i64>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub default_free_ip_max_tokens: Option<Option<i64>>,
    pub session_ttl_ms: Option<i64>,
    pub proxy_test_interval_sec: Option<u64>,
    pub exhaustive_failover: Option<bool>,
}

impl SettingsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_settings(&self) -> Result<GatewaySettings> {
        let rows = sqlx::query("SELECT key, value FROM settings")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        let mut map = HashMap::new();
        for row in rows {
            let key: String = row.try_get("key").map_err(map_sqlx_err)?;
            let value: String = row.try_get("value").map_err(map_sqlx_err)?;
            map.insert(key, value);
        }

        let mut out = GatewaySettings::default();
        if let Some(v) = map.get("strict_proxy_mode") {
            out.strict_proxy_mode =
                matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1");
        }
        if let Some(v) = map.get("default_free_ip_window_ms")
            && let Ok(n) = v.trim().parse::<i64>()
            && n > 0
        {
            out.default_free_ip_window_ms = n;
        }
        if let Some(v) = map.get("default_free_ip_max_requests") {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                out.default_free_ip_max_requests = None;
            } else if let Ok(n) = trimmed.parse::<i64>() {
                out.default_free_ip_max_requests = Some(n);
            }
        }
        if let Some(v) = map.get("default_free_ip_max_tokens") {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                out.default_free_ip_max_tokens = None;
            } else if let Ok(n) = trimmed.parse::<i64>() {
                out.default_free_ip_max_tokens = Some(n);
            }
        }
        if let Some(v) = map.get("session_ttl_ms")
            && let Ok(n) = v.trim().parse::<i64>()
            && n > 0
        {
            out.session_ttl_ms = n;
        }
        if let Some(v) = map.get("proxy_test_interval_sec")
            && let Ok(n) = v.trim().parse::<u64>()
        {
            out.proxy_test_interval_sec = n.clamp(5, 3600);
        }
        if let Some(v) = map.get("exhaustive_failover") {
            out.exhaustive_failover =
                matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1");
        }

        Ok(out)
    }

    pub async fn update_settings(&self, input: UpdateSettingsInput) -> Result<GatewaySettings> {
        let now = now_ms();
        if let Some(strict) = input.strict_proxy_mode {
            self.set_raw("strict_proxy_mode", if strict { "true" } else { "false" }, now)
                .await?;
        }
        if let Some(win) = input.default_free_ip_window_ms
            && win > 0
        {
            self.set_raw("default_free_ip_window_ms", &win.to_string(), now)
                .await?;
        }
        if let Some(max_req) = input.default_free_ip_max_requests {
            let val = max_req
                .filter(|&v| v > 0)
                .map(|v| v.to_string())
                .unwrap_or_default();
            self.set_raw("default_free_ip_max_requests", &val, now)
                .await?;
        }
        if let Some(max_tok) = input.default_free_ip_max_tokens {
            let val = max_tok
                .filter(|&v| v > 0)
                .map(|v| v.to_string())
                .unwrap_or_default();
            self.set_raw("default_free_ip_max_tokens", &val, now)
                .await?;
        }
        if let Some(ttl) = input.session_ttl_ms
            && ttl > 0
        {
            self.set_raw("session_ttl_ms", &ttl.to_string(), now).await?;
        }
        if let Some(interval_sec) = input.proxy_test_interval_sec {
            let clamped = interval_sec.clamp(5, 3600);
            self.set_raw("proxy_test_interval_sec", &clamped.to_string(), now)
                .await?;
        }
        if let Some(failover) = input.exhaustive_failover {
            self.set_raw("exhaustive_failover", if failover { "true" } else { "false" }, now)
                .await?;
        }

        let updated = self.get_settings().await?;
        // Mirror settings updates onto `opencode` provider_configs row when proxy/quota settings changed
        if input.strict_proxy_mode.is_some()
            || input.default_free_ip_window_ms.is_some()
            || input.default_free_ip_max_requests.is_some()
            || input.default_free_ip_max_tokens.is_some()
            || input.session_ttl_ms.is_some()
        {
            let oc_mode = if updated.strict_proxy_mode {
                "pool_strict"
            } else {
                "pool_with_direct"
            };
            sqlx::query(
                "INSERT INTO provider_configs (
                    provider_id, enabled, proxy_mode, window_duration_ms,
                    max_requests_per_window, max_tokens_per_window, session_ttl_ms, updated_at
                 ) VALUES ('opencode', 1, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(provider_id) DO UPDATE SET
                    proxy_mode = ifnull(?, provider_configs.proxy_mode),
                    window_duration_ms = excluded.window_duration_ms,
                    max_requests_per_window = excluded.max_requests_per_window,
                    max_tokens_per_window = excluded.max_tokens_per_window,
                    session_ttl_ms = excluded.session_ttl_ms,
                    updated_at = excluded.updated_at",
            )
            .bind(oc_mode)
            .bind(updated.default_free_ip_window_ms)
            .bind(updated.default_free_ip_max_requests)
            .bind(updated.default_free_ip_max_tokens)
            .bind(updated.session_ttl_ms)
            .bind(now)
            .bind(input.strict_proxy_mode.map(|_| oc_mode))
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        }

        Ok(updated)
    }

    /// Returns the stored credential row verbatim. Callers that need to compare against a
    /// user-supplied secret must go through [`verify_admin_password`] instead — the value may
    /// be an Argon2id PHC string, and legacy installs may still hold a plaintext password.
    pub async fn get_admin_password(&self) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = 'admin_password'")
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(row
            .and_then(|r| r.try_get::<String, _>("value").ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    pub async fn has_admin_password(&self) -> Result<bool> {
        Ok(self.get_admin_password().await?.is_some())
    }

    pub async fn set_admin_password(&self, password: Option<&str>) -> Result<()> {
        let now = now_ms();
        let clean = password.map(str::trim).filter(|s| !s.is_empty());
        match clean {
            Some(pw) => {
                self.set_raw("admin_password", &hash_admin_password(pw)?, now)
                    .await
            }
            None => {
                sqlx::query("DELETE FROM settings WHERE key = 'admin_password'")
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
                Ok(())
            }
        }
    }

    /// Constant-time check of a candidate against the stored credential.
    ///
    /// Legacy plaintext rows are compared in constant time and then re-hashed in place, so a
    /// successful unlock is the last time that password exists in cleartext on disk.
    pub async fn verify_admin_password(&self, candidate: &str) -> Result<bool> {
        let Some(stored) = self.get_admin_password().await? else {
            return Ok(false);
        };

        if !stored.starts_with("$argon2") {
            if !ct_eq(stored.as_bytes(), candidate.as_bytes()) {
                return Ok(false);
            }
            let _ = self
                .set_raw("admin_password", &hash_admin_password(candidate)?, now_ms())
                .await;
            return Ok(true);
        }

        Ok(verify_phc(&stored, candidate))
    }

    async fn set_raw(&self, key: &str, value: &str, now: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }
}

/// Argon2id with the crate defaults (m=19456 KiB, t=2, p=1), a fresh random salt per call.
pub fn hash_admin_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| TesRouteError::BadRequest(format!("failed to hash admin password: {e}")))
}

fn verify_phc(phc: &str, candidate: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(candidate.as_bytes(), &parsed)
        .is_ok()
}

/// Branch-free comparison so a legacy plaintext credential cannot be recovered by timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}
