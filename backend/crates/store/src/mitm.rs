use sqlx::{Row, SqlitePool};
use tesie_core::{AntigravityMitmConfig, Result};

use crate::map_sqlx_err;

const MITM_CONFIG_KEY: &str = "antigravity_mitm_config";

#[derive(Clone, Debug)]
pub struct MitmRepo {
    pool: SqlitePool,
}

impl MitmRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_config(&self) -> Result<AntigravityMitmConfig> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(MITM_CONFIG_KEY)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        if let Some(r) = row {
            let raw: String = r.try_get("value").map_err(map_sqlx_err)?;
            if let Ok(cfg) = serde_json::from_str::<AntigravityMitmConfig>(&raw) {
                return Ok(cfg);
            }
        }
        Ok(AntigravityMitmConfig::default())
    }

    pub async fn save_config(&self, cfg: &AntigravityMitmConfig) -> Result<()> {
        let now = crate::now_ms();
        let raw = serde_json::to_string(cfg)
            .map_err(|e| tesie_core::TesRouteError::Store(e.to_string()))?;
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(MITM_CONFIG_KEY)
        .bind(raw)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }
}
