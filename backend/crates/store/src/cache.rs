use sqlx::{Row, SqlitePool};
use tesie_core::{ResponseCacheRecord, ResponseCacheStats, Result};

use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct ResponseCacheRepo {
    pool: SqlitePool,
}

impl ResponseCacheRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, cache_key: &str) -> Result<Option<ResponseCacheRecord>> {
        let now = now_ms();
        let row = sqlx::query(
            "SELECT cache_key, model, source_format, response_payload,
                    prompt_tokens, completion_tokens, total_tokens,
                    created_at, expires_at, hit_count, last_hit_at
             FROM response_cache
             WHERE cache_key = ? AND expires_at > ?",
        )
        .bind(cache_key)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        if let Some(r) = row {
            let record = ResponseCacheRecord {
                cache_key: r.try_get("cache_key").map_err(map_sqlx_err)?,
                model: r.try_get("model").map_err(map_sqlx_err)?,
                source_format: r.try_get("source_format").map_err(map_sqlx_err)?,
                response_payload: r.try_get("response_payload").map_err(map_sqlx_err)?,
                prompt_tokens: r.try_get("prompt_tokens").map_err(map_sqlx_err)?,
                completion_tokens: r.try_get("completion_tokens").map_err(map_sqlx_err)?,
                total_tokens: r.try_get("total_tokens").map_err(map_sqlx_err)?,
                created_at: r.try_get("created_at").map_err(map_sqlx_err)?,
                expires_at: r.try_get("expires_at").map_err(map_sqlx_err)?,
                hit_count: r.try_get("hit_count").map_err(map_sqlx_err)?,
                last_hit_at: r.try_get("last_hit_at").map_err(map_sqlx_err)?,
            };

            let _ = self.record_hit(cache_key, record.total_tokens).await;

            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    pub async fn record_hit(&self, cache_key: &str, tokens_saved: i64) -> Result<()> {
        let now = now_ms();
        let _ = sqlx::query(
            "UPDATE response_cache SET hit_count = hit_count + 1, last_hit_at = ? WHERE cache_key = ?",
        )
        .bind(now)
        .bind(cache_key)
        .execute(&self.pool)
        .await;

        let _ = self.increment_counter("cache_hit_count").await;
        if tokens_saved > 0 {
            let _ = self.add_to_counter("cache_tokens_saved", tokens_saved).await;
        }
        Ok(())
    }

    pub async fn set(&self, record: &ResponseCacheRecord, max_entries: i64) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO response_cache (
                cache_key, model, source_format, response_payload,
                prompt_tokens, completion_tokens, total_tokens,
                created_at, expires_at, hit_count, last_hit_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(cache_key) DO UPDATE SET
                response_payload = excluded.response_payload,
                prompt_tokens = excluded.prompt_tokens,
                completion_tokens = excluded.completion_tokens,
                total_tokens = excluded.total_tokens,
                expires_at = excluded.expires_at",
        )
        .bind(&record.cache_key)
        .bind(&record.model)
        .bind(&record.source_format)
        .bind(&record.response_payload)
        .bind(record.prompt_tokens)
        .bind(record.completion_tokens)
        .bind(record.total_tokens)
        .bind(record.created_at)
        .bind(record.expires_at)
        .bind(record.hit_count)
        .bind(record.last_hit_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        // Prune oldest if total exceeds max_entries
        if max_entries > 0 {
            let total = self.count().await?;
            if total > max_entries {
                let excess = total - max_entries;
                let _ = sqlx::query(
                    "DELETE FROM response_cache WHERE cache_key IN (
                        SELECT cache_key FROM response_cache
                        ORDER BY last_hit_at ASC, created_at ASC
                        LIMIT ?
                    )",
                )
                .bind(excess)
                .execute(&self.pool)
                .await;
            }
        }

        // Opportunistic cleanup of expired rows
        let _ = sqlx::query("DELETE FROM response_cache WHERE expires_at < ?")
            .bind(now)
            .execute(&self.pool)
            .await;

        Ok(())
    }

    pub async fn record_miss(&self) -> Result<()> {
        let _ = self.increment_counter("cache_miss_count").await;
        Ok(())
    }

    pub async fn count(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(1) AS cnt FROM response_cache")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let count: i64 = row.try_get("cnt").map_err(map_sqlx_err)?;
        Ok(count)
    }

    pub async fn prune_expired(&self) -> Result<u64> {
        let now = now_ms();
        let res = sqlx::query("DELETE FROM response_cache WHERE expires_at < ?")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        Ok(res.rows_affected())
    }

    pub async fn purge_all(&self) -> Result<u64> {
        let res = sqlx::query("DELETE FROM response_cache")
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let _ = sqlx::query("PRAGMA incremental_vacuum(50)")
            .execute(&self.pool)
            .await;
        Ok(res.rows_affected())
    }

    pub async fn get_stats(&self) -> Result<ResponseCacheStats> {
        let now = now_ms();
        let row = sqlx::query(
            "SELECT COUNT(1) AS total,
                    COALESCE(SUM(hit_count), 0) AS hits,
                    COALESCE(SUM(hit_count * total_tokens), 0) AS tokens_saved
             FROM response_cache
             WHERE expires_at > ?",
        )
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let total_entries: i64 = row.try_get("total").unwrap_or(0);
        let hits_from_cache: i64 = row.try_get("hits").unwrap_or(0);
        let tokens_saved_from_cache: i64 = row.try_get("tokens_saved").unwrap_or(0);

        let global_hits = self.get_counter("cache_hit_count").await.unwrap_or(0);
        let global_misses = self.get_counter("cache_miss_count").await.unwrap_or(0);
        let global_tokens_saved = self.get_counter("cache_tokens_saved").await.unwrap_or(0);

        Ok(ResponseCacheStats {
            total_entries,
            hit_count: std::cmp::max(hits_from_cache, global_hits),
            miss_count: global_misses,
            tokens_saved: std::cmp::max(tokens_saved_from_cache, global_tokens_saved),
            memory_entries: 0,
        })
    }

    async fn increment_counter(&self, key: &str) -> Result<()> {
        self.add_to_counter(key, 1).await
    }

    async fn add_to_counter(&self, key: &str, amount: i64) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET
                value = CAST((CAST(settings.value AS INTEGER) + excluded.value) AS TEXT),
                updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(amount.to_string())
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    async fn get_counter(&self, key: &str) -> Result<i64> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if let Some(r) = row {
            let val: String = r.try_get("value").map_err(map_sqlx_err)?;
            Ok(val.trim().parse::<i64>().unwrap_or(0))
        } else {
            Ok(0)
        }
    }
}
