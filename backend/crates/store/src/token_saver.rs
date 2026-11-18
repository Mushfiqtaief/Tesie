use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{RequestSavingsReport, Result, TokenSaverConfig};

use crate::{map_sqlx_err, now_ms};

const TOKEN_SAVER_CONFIG_KEY: &str = "token_saver_config";

#[derive(Clone, Debug)]
pub struct TokenSaverRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenSaverCategoryStat {
    pub category: String,
    pub requests: i64,
    pub saved_tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSaverEvent {
    pub id: i64,
    pub created_at: i64,
    pub provider_id: String,
    pub model: String,
    pub category: String,
    pub original_tokens: i64,
    pub compressed_tokens: i64,
    pub saved_tokens: i64,
    pub style_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenSaverStats {
    pub total_saved_tokens: i64,
    pub total_original_tokens: i64,
    pub compressed_requests: i64,
    pub style_applied_requests: i64,
    pub avg_saved_pct: f64,
    pub by_category: Vec<TokenSaverCategoryStat>,
    pub recent: Vec<TokenSaverEvent>,
}

impl TokenSaverRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_config(&self) -> Result<TokenSaverConfig> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(TOKEN_SAVER_CONFIG_KEY)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        if let Some(r) = row {
            let raw: String = r.try_get("value").map_err(map_sqlx_err)?;
            if let Ok(cfg) = serde_json::from_str::<TokenSaverConfig>(&raw) {
                return Ok(cfg);
            }
        }
        Ok(TokenSaverConfig::default())
    }

    pub async fn save_config(&self, cfg: &TokenSaverConfig) -> Result<TokenSaverConfig> {
        let now = now_ms();
        let json_str = serde_json::to_string(cfg).unwrap_or_else(|_| "{}".to_string());
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(TOKEN_SAVER_CONFIG_KEY)
        .bind(&json_str)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(cfg.clone())
    }

    pub async fn record_savings(
        &self,
        provider_id: &str,
        model: &str,
        report: &RequestSavingsReport,
        style_mode: &str,
    ) -> Result<()> {
        if report.saved_tokens == 0 && !report.style_applied {
            return Ok(());
        }
        let now = now_ms();
        let cat = if report.saved_tokens > 0 {
            report.primary_category.as_str()
        } else {
            "style"
        };
        let active_style = if report.style_applied {
            style_mode
        } else {
            "off"
        };

        sqlx::query(
            "INSERT INTO token_saver_logs (
                created_at, provider_id, model, category,
                original_tokens, compressed_tokens, saved_tokens, style_mode
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(now)
        .bind(provider_id)
        .bind(model)
        .bind(cat)
        .bind(report.original_tokens as i64)
        .bind(report.compressed_tokens as i64)
        .bind(report.saved_tokens as i64)
        .bind(active_style)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn get_stats(&self) -> Result<TokenSaverStats> {
        let totals = sqlx::query(
            "SELECT
                COALESCE(SUM(saved_tokens), 0) AS saved,
                COALESCE(SUM(original_tokens), 0) AS orig,
                COALESCE(SUM(CASE WHEN saved_tokens > 0 THEN 1 ELSE 0 END), 0) AS comp_cnt,
                COALESCE(SUM(CASE WHEN style_mode != 'off' THEN 1 ELSE 0 END), 0) AS style_cnt
             FROM token_saver_logs",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let total_saved_tokens: i64 = totals.try_get("saved").map_err(map_sqlx_err)?;
        let total_original_tokens: i64 = totals.try_get("orig").map_err(map_sqlx_err)?;
        let compressed_requests: i64 = totals.try_get("comp_cnt").map_err(map_sqlx_err)?;
        let style_applied_requests: i64 = totals.try_get("style_cnt").map_err(map_sqlx_err)?;

        let avg_saved_pct = if total_original_tokens > 0 && total_saved_tokens > 0 {
            ((total_saved_tokens as f64 / total_original_tokens as f64) * 1000.0).round() / 10.0
        } else {
            0.0
        };

        let cat_rows = sqlx::query(
            "SELECT category, COUNT(*) AS reqs, COALESCE(SUM(saved_tokens), 0) AS saved
             FROM token_saver_logs
             WHERE saved_tokens > 0
             GROUP BY category
             ORDER BY saved DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let mut by_category = Vec::with_capacity(cat_rows.len());
        for row in cat_rows {
            by_category.push(TokenSaverCategoryStat {
                category: row.try_get("category").map_err(map_sqlx_err)?,
                requests: row.try_get("reqs").map_err(map_sqlx_err)?,
                saved_tokens: row.try_get("saved").map_err(map_sqlx_err)?,
            });
        }

        let recent_rows = sqlx::query(
            "SELECT id, created_at, provider_id, model, category,
                    original_tokens, compressed_tokens, saved_tokens, style_mode
             FROM token_saver_logs
             ORDER BY id DESC
             LIMIT 20",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let mut recent = Vec::with_capacity(recent_rows.len());
        for row in recent_rows {
            recent.push(TokenSaverEvent {
                id: row.try_get("id").map_err(map_sqlx_err)?,
                created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
                provider_id: row.try_get("provider_id").map_err(map_sqlx_err)?,
                model: row.try_get("model").map_err(map_sqlx_err)?,
                category: row.try_get("category").map_err(map_sqlx_err)?,
                original_tokens: row.try_get("original_tokens").map_err(map_sqlx_err)?,
                compressed_tokens: row.try_get("compressed_tokens").map_err(map_sqlx_err)?,
                saved_tokens: row.try_get("saved_tokens").map_err(map_sqlx_err)?,
                style_mode: row.try_get("style_mode").map_err(map_sqlx_err)?,
            });
        }

        Ok(TokenSaverStats {
            total_saved_tokens,
            total_original_tokens,
            compressed_requests,
            style_applied_requests,
            avg_saved_pct,
            by_category,
            recent,
        })
    }

    pub async fn clear_stats(&self) -> Result<u64> {
        let res = sqlx::query("DELETE FROM token_saver_logs")
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(res.rows_affected())
    }
}
