use chrono::{Duration as ChronoDuration, Local, TimeZone};
use serde::Serialize;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use tesie_core::{Result, UsageLogInput, UsageLogRecord};

use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct UsageRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Default)]
pub struct LogQueryFilter {
    pub limit: i64,
    pub status: Option<String>,
    pub proxy: Option<String>,
    pub model: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyUsageAggregate {
    pub proxy_name: String,
    pub last_egress_ip: Option<String>,
    pub total_requests: i64,
    pub success_requests: i64,
    pub rate_limited_requests: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub avg_tokens_per_sec: f64,
    pub peak_tokens_per_sec: f64,
    pub avg_ttft_ms: f64,
    pub avg_latency_ms: f64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IpUsageAggregate {
    pub egress_ip: String,
    pub total_requests: i64,
    pub success_requests: i64,
    pub total_tokens: i64,
    pub rate_limited_requests: i64,
    pub avg_tokens_per_sec: f64,
    pub avg_ttft_ms: f64,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelUsageAggregate {
    pub model: String,
    pub provider_id: String,
    pub total_requests: i64,
    pub success_requests: i64,
    pub rate_limited_requests: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub avg_tokens_per_sec: f64,
    pub peak_tokens_per_sec: f64,
    pub avg_ttft_ms: f64,
    pub avg_latency_ms: f64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderUsageAggregate {
    pub provider_id: String,
    pub total_requests: i64,
    pub success_requests: i64,
    pub rate_limited_requests: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub avg_tokens_per_sec: f64,
    pub avg_ttft_ms: f64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyUsageAggregate {
    pub key_name: String,
    pub key_prefix: Option<String>,
    pub total_requests: i64,
    pub success_requests: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageTimeBucket {
    pub timestamp: i64,
    pub label: String,
    pub requests: i64,
    pub success_requests: i64,
    pub rate_limited_requests: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub avg_tokens_per_sec: f64,
    pub avg_ttft_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageSummary {
    pub period: String,
    pub total_requests: i64,
    pub success_requests: i64,
    pub rate_limited_requests: i64,
    pub error_requests: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub avg_tokens_per_sec: f64,
    pub peak_tokens_per_sec: f64,
    pub avg_ttft_ms: f64,
    pub avg_latency_ms: f64,
    pub timeseries: Vec<UsageTimeBucket>,
    pub by_provider: Vec<ProviderUsageAggregate>,
    pub by_api_key: Vec<ApiKeyUsageAggregate>,
    pub by_proxy: Vec<ProxyUsageAggregate>,
    pub by_ip: Vec<IpUsageAggregate>,
    pub by_model: Vec<ModelUsageAggregate>,
    pub recent: Vec<UsageLogRecord>,
}

const LOG_SELECT_COLS: &str = "id, request_id, api_key_id, provider_id, model, proxy_name, egress_ip,
    status_code, prompt_tokens, cached_tokens, completion_tokens, total_tokens,
    latency_ms, ttft_ms, tokens_per_sec, error_kind, error_detail,
    upstream_url, COALESCE(attempt_count, 1) AS attempt_count, attempt_trace, session_id, wire_format,
    created_at";

fn round2(val: f64) -> f64 {
    if val.is_finite() {
        (val * 100.0).round() / 100.0
    } else {
        0.0
    }
}

fn period_bounds(period_raw: &str, now: i64) -> (String, Option<i64>) {
    let norm = period_raw.trim().to_ascii_lowercase();
    match norm.as_str() {
        "today" => {
            let today = Local::now().date_naive();
            let start = today
                .and_hms_opt(0, 0, 0)
                .and_then(|dt| Local.from_local_datetime(&dt).earliest())
                .map(|dt| dt.timestamp_millis())
                .unwrap_or(now - 86_400_000);
            ("today".into(), Some(start))
        }
        "24h" => ("24h".into(), Some(now.saturating_sub(86_400_000))),
        "7d" => ("7d".into(), Some(now.saturating_sub(7 * 86_400_000))),
        "30d" => ("30d".into(), Some(now.saturating_sub(30 * 86_400_000))),
        _ => ("all".into(), None),
    }
}

fn push_since_filter<'a>(qb: &mut QueryBuilder<'a, Sqlite>, prefix: &str, since: Option<i64>) {
    if let Some(ts) = since {
        qb.push(prefix);
        qb.push("created_at >= ");
        qb.push_bind(ts);
    }
}

impl UsageRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn insert_log(&self, input: UsageLogInput) -> Result<i64> {
        let now = now_ms();
        let attempt_count = input.attempt_count.max(1);
        let res = sqlx::query(
            "INSERT INTO usage_logs (
                request_id, api_key_id, provider_id, model, proxy_name, egress_ip,
                status_code, prompt_tokens, cached_tokens, completion_tokens, total_tokens,
                latency_ms, ttft_ms, tokens_per_sec, error_kind, error_detail,
                upstream_url, attempt_count, attempt_trace, session_id, wire_format, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&input.request_id)
        .bind(&input.api_key_id)
        .bind(&input.provider_id)
        .bind(&input.model)
        .bind(&input.proxy_name)
        .bind(&input.egress_ip)
        .bind(input.status_code)
        .bind(input.prompt_tokens)
        .bind(input.cached_tokens)
        .bind(input.completion_tokens)
        .bind(input.total_tokens)
        .bind(input.latency_ms)
        .bind(input.ttft_ms)
        .bind(input.tokens_per_sec)
        .bind(&input.error_kind)
        .bind(&input.error_detail)
        .bind(&input.upstream_url)
        .bind(attempt_count)
        .bind(&input.attempt_trace)
        .bind(&input.session_id)
        .bind(&input.wire_format)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        Ok(res.last_insert_rowid())
    }

    pub async fn list_recent_logs(&self, limit: i64) -> Result<Vec<UsageLogRecord>> {
        self.list_recent_logs_since(limit, None).await
    }

    async fn list_recent_logs_since(
        &self,
        limit: i64,
        since: Option<i64>,
    ) -> Result<Vec<UsageLogRecord>> {
        let clamped = limit.clamp(1, 500);
        // Dashboard `Recent requests` must ONLY show succeeded (2xx) completions;
        // failed/rate-limited requests remain accessible in the dedicated Live Logs view.
        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(format!(
            "SELECT {LOG_SELECT_COLS} FROM usage_logs WHERE status_code >= 200 AND status_code < 300"
        ));
        push_since_filter(&mut qb, " AND ", since);
        qb.push(" ORDER BY id DESC LIMIT ");
        qb.push_bind(clamped);

        let rows = qb.build().fetch_all(&self.pool).await.map_err(map_sqlx_err)?;
        rows.iter().map(row_to_usage).collect()
    }

    pub async fn query_logs(&self, filter: LogQueryFilter) -> Result<Vec<UsageLogRecord>> {
        let clamped = if filter.limit <= 0 {
            100
        } else {
            filter.limit.clamp(1, 500)
        };

        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new(format!("SELECT {LOG_SELECT_COLS} FROM usage_logs WHERE 1=1"));

        if let Some(status_raw) = filter.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            match status_raw.to_ascii_lowercase().as_str() {
                "2xx" | "success" | "ok" => {
                    qb.push(" AND status_code >= 200 AND status_code < 300");
                }
                "429" | "ratelimit" | "rate_limit" => {
                    qb.push(" AND status_code = 429");
                }
                "error" | "errors" | "err" => {
                    qb.push(" AND (status_code < 200 OR status_code >= 300)");
                }
                other => {
                    if let Ok(code) = other.parse::<i64>() {
                        qb.push(" AND status_code = ");
                        qb.push_bind(code);
                    }
                }
            }
        }

        if let Some(proxy) = filter.proxy.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            qb.push(" AND proxy_name = ");
            qb.push_bind(proxy);
        }

        if let Some(model) = filter.model.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            qb.push(" AND model = ");
            qb.push_bind(model);
        }

        if let Some(search) = filter.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let pattern = format!("%{search}%");
            qb.push(" AND (request_id LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR provider_id LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR model LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR proxy_name LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR COALESCE(egress_ip, '') LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR COALESCE(error_kind, '') LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR COALESCE(error_detail, '') LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR COALESCE(session_id, '') LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR COALESCE(upstream_url, '') LIKE ");
            qb.push_bind(pattern.clone());
            qb.push(" OR COALESCE(attempt_trace, '') LIKE ");
            qb.push_bind(pattern);
            qb.push(")");
        }

        qb.push(" ORDER BY id DESC LIMIT ");
        qb.push_bind(clamped);

        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        rows.iter().map(row_to_usage).collect()
    }

    pub async fn clear_logs(&self) -> Result<u64> {
        let res = sqlx::query("DELETE FROM usage_logs")
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let _ = sqlx::query("PRAGMA incremental_vacuum(100)")
            .execute(&self.pool)
            .await;
        Ok(res.rows_affected())
    }

    pub async fn count_logs(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(1) AS cnt FROM usage_logs")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let count: i64 = row.try_get("cnt").map_err(map_sqlx_err)?;
        Ok(count)
    }

    pub async fn prune_logs(
        &self,
        retention_days: Option<i64>,
        max_entries: Option<i64>,
    ) -> Result<u64> {
        let mut deleted_total = 0u64;

        if let Some(days) = retention_days {
            if days > 0 {
                let cutoff_ms = now_ms() - (days * 86_400 * 1000);
                let res = sqlx::query("DELETE FROM usage_logs WHERE created_at < ?")
                    .bind(cutoff_ms)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
                deleted_total += res.rows_affected();
            }
        }

        if let Some(limit) = max_entries {
            if limit > 0 {
                let total = self.count_logs().await?;
                if total > limit {
                    let excess = total - limit;
                    let res = sqlx::query(
                        "DELETE FROM usage_logs WHERE id IN (
                            SELECT id FROM usage_logs ORDER BY created_at ASC LIMIT ?
                        )",
                    )
                    .bind(excess)
                    .execute(&self.pool)
                    .await
                    .map_err(map_sqlx_err)?;
                    deleted_total += res.rows_affected();
                }
            }
        }

        if deleted_total > 0 {
            let _ = sqlx::query("PRAGMA incremental_vacuum(50)")
                .execute(&self.pool)
                .await;
        }

        Ok(deleted_total)
    }

    pub async fn summarize_usage(&self, recent_limit: i64) -> Result<UsageSummary> {
        self.summarize_usage_for_period("all", recent_limit).await
    }

    pub async fn summarize_usage_for_period(
        &self,
        period_raw: &str,
        recent_limit: i64,
    ) -> Result<UsageSummary> {
        let now = now_ms();
        let (period, since) = period_bounds(period_raw, now);

        let mut tot_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT COUNT(1) AS cnt,
                    COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END), 0) AS ok_cnt,
                    COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0) AS rl_cnt,
                    COALESCE(SUM(CASE WHEN (status_code < 200 OR status_code >= 300) AND status_code != 429 THEN 1 ELSE 0 END), 0) AS err_cnt,
                    COALESCE(SUM(prompt_tokens), 0) AS ptoks,
                    COALESCE(SUM(cached_tokens), 0) AS cachetoks,
                    COALESCE(SUM(completion_tokens), 0) AS ctoks,
                    COALESCE(SUM(total_tokens), 0) AS ttoks,
                    CAST(COALESCE(AVG(CASE WHEN tokens_per_sec IS NOT NULL AND tokens_per_sec > 0 THEN tokens_per_sec END), 0.0) AS REAL) AS avg_tps,
                    CAST(COALESCE(MAX(tokens_per_sec), 0.0) AS REAL) AS peak_tps,
                    CAST(COALESCE(AVG(CASE WHEN ttft_ms IS NOT NULL THEN ttft_ms END), 0.0) AS REAL) AS avg_ttft,
                    CAST(COALESCE(AVG(latency_ms), 0.0) AS REAL) AS avg_lat
             FROM usage_logs",
        );
        push_since_filter(&mut tot_qb, " WHERE ", since);
        let totals_row = tot_qb
            .build()
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        let total_requests: i64 = totals_row.try_get("cnt").map_err(map_sqlx_err)?;
        let success_requests: i64 = totals_row.try_get("ok_cnt").map_err(map_sqlx_err)?;
        let rate_limited_requests: i64 = totals_row.try_get("rl_cnt").map_err(map_sqlx_err)?;
        let error_requests: i64 = totals_row.try_get("err_cnt").map_err(map_sqlx_err)?;
        let prompt_tokens: i64 = totals_row.try_get("ptoks").map_err(map_sqlx_err)?;
        let cached_tokens: i64 = totals_row.try_get("cachetoks").map_err(map_sqlx_err)?;
        let completion_tokens: i64 = totals_row.try_get("ctoks").map_err(map_sqlx_err)?;
        let total_tokens: i64 = totals_row.try_get("ttoks").map_err(map_sqlx_err)?;
        let avg_tokens_per_sec = round2(totals_row.try_get("avg_tps").map_err(map_sqlx_err)?);
        let peak_tokens_per_sec = round2(totals_row.try_get("peak_tps").map_err(map_sqlx_err)?);
        let avg_ttft_ms = round2(totals_row.try_get("avg_ttft").map_err(map_sqlx_err)?);
        let avg_latency_ms = round2(totals_row.try_get("avg_lat").map_err(map_sqlx_err)?);

        // 1. Time series (in Rust over local boundaries so zero timezone mismatches)
        let timeseries = self.build_timeseries(&period, since, now).await?;

        // 2. Provider breakdown
        let mut prov_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT provider_id,
                    COUNT(1) AS cnt,
                    COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END), 0) AS ok_cnt,
                    COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0) AS rl_cnt,
                    COALESCE(SUM(prompt_tokens), 0) AS ptoks,
                    COALESCE(SUM(cached_tokens), 0) AS cachetoks,
                    COALESCE(SUM(completion_tokens), 0) AS ctoks,
                    COALESCE(SUM(total_tokens), 0) AS ttoks,
                    CAST(COALESCE(AVG(CASE WHEN tokens_per_sec IS NOT NULL AND tokens_per_sec > 0 THEN tokens_per_sec END), 0.0) AS REAL) AS avg_tps,
                    CAST(COALESCE(AVG(CASE WHEN ttft_ms IS NOT NULL THEN ttft_ms END), 0.0) AS REAL) AS avg_ttft,
                    MAX(created_at) AS last_used
             FROM usage_logs",
        );
        push_since_filter(&mut prov_qb, " WHERE ", since);
        prov_qb.push(" GROUP BY provider_id ORDER BY ttoks DESC, cnt DESC");
        let prov_rows = prov_qb.build().fetch_all(&self.pool).await.map_err(map_sqlx_err)?;
        let mut by_provider = Vec::with_capacity(prov_rows.len());
        for row in prov_rows {
            by_provider.push(ProviderUsageAggregate {
                provider_id: row.try_get("provider_id").map_err(map_sqlx_err)?,
                total_requests: row.try_get("cnt").map_err(map_sqlx_err)?,
                success_requests: row.try_get("ok_cnt").map_err(map_sqlx_err)?,
                rate_limited_requests: row.try_get("rl_cnt").map_err(map_sqlx_err)?,
                prompt_tokens: row.try_get("ptoks").map_err(map_sqlx_err)?,
                cached_tokens: row.try_get("cachetoks").map_err(map_sqlx_err)?,
                completion_tokens: row.try_get("ctoks").map_err(map_sqlx_err)?,
                total_tokens: row.try_get("ttoks").map_err(map_sqlx_err)?,
                avg_tokens_per_sec: round2(row.try_get("avg_tps").map_err(map_sqlx_err)?),
                avg_ttft_ms: round2(row.try_get("avg_ttft").map_err(map_sqlx_err)?),
                last_used_at: row.try_get("last_used").map_err(map_sqlx_err)?,
            });
        }

        // 3. API Key breakdown
        let mut key_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT COALESCE(k.name, 'Direct / Keyless') AS kname,
                    k.key_prefix AS kprefix,
                    COUNT(1) AS cnt,
                    COALESCE(SUM(CASE WHEN u.status_code >= 200 AND u.status_code < 300 THEN 1 ELSE 0 END), 0) AS ok_cnt,
                    COALESCE(SUM(u.prompt_tokens), 0) AS ptoks,
                    COALESCE(SUM(u.cached_tokens), 0) AS cachetoks,
                    COALESCE(SUM(u.completion_tokens), 0) AS ctoks,
                    COALESCE(SUM(u.total_tokens), 0) AS ttoks,
                    MAX(u.created_at) AS last_used
             FROM usage_logs u
             LEFT JOIN api_keys k ON u.api_key_id = k.id",
        );
        if let Some(ts) = since {
            key_qb.push(" WHERE u.created_at >= ");
            key_qb.push_bind(ts);
        }
        key_qb.push(" GROUP BY COALESCE(k.id, '__keyless__') ORDER BY ttoks DESC, cnt DESC");
        let key_rows = key_qb.build().fetch_all(&self.pool).await.map_err(map_sqlx_err)?;
        let mut by_api_key = Vec::with_capacity(key_rows.len());
        for row in key_rows {
            by_api_key.push(ApiKeyUsageAggregate {
                key_name: row.try_get("kname").map_err(map_sqlx_err)?,
                key_prefix: row.try_get("kprefix").map_err(map_sqlx_err)?,
                total_requests: row.try_get("cnt").map_err(map_sqlx_err)?,
                success_requests: row.try_get("ok_cnt").map_err(map_sqlx_err)?,
                prompt_tokens: row.try_get("ptoks").map_err(map_sqlx_err)?,
                cached_tokens: row.try_get("cachetoks").map_err(map_sqlx_err)?,
                completion_tokens: row.try_get("ctoks").map_err(map_sqlx_err)?,
                total_tokens: row.try_get("ttoks").map_err(map_sqlx_err)?,
                last_used_at: row.try_get("last_used").map_err(map_sqlx_err)?,
            });
        }

        // 4. Proxy breakdown
        let mut px_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT proxy_name,
                    MAX(egress_ip) AS last_ip,
                    COUNT(1) AS cnt,
                    COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END), 0) AS ok_cnt,
                    COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0) AS rl_cnt,
                    COALESCE(SUM(prompt_tokens), 0) AS ptoks,
                    COALESCE(SUM(cached_tokens), 0) AS cachetoks,
                    COALESCE(SUM(completion_tokens), 0) AS ctoks,
                    COALESCE(SUM(total_tokens), 0) AS ttoks,
                    CAST(COALESCE(AVG(CASE WHEN tokens_per_sec IS NOT NULL AND tokens_per_sec > 0 THEN tokens_per_sec END), 0.0) AS REAL) AS avg_tps,
                    CAST(COALESCE(MAX(tokens_per_sec), 0.0) AS REAL) AS peak_tps,
                    CAST(COALESCE(AVG(CASE WHEN ttft_ms IS NOT NULL THEN ttft_ms END), 0.0) AS REAL) AS avg_ttft,
                    CAST(COALESCE(AVG(latency_ms), 0.0) AS REAL) AS avg_lat,
                    MAX(created_at) AS last_used
             FROM usage_logs",
        );
        push_since_filter(&mut px_qb, " WHERE ", since);
        px_qb.push(" GROUP BY proxy_name ORDER BY cnt DESC, proxy_name ASC");
        let proxy_rows = px_qb.build().fetch_all(&self.pool).await.map_err(map_sqlx_err)?;

        let mut by_proxy = Vec::with_capacity(proxy_rows.len());
        for row in proxy_rows {
            by_proxy.push(ProxyUsageAggregate {
                proxy_name: row.try_get("proxy_name").map_err(map_sqlx_err)?,
                last_egress_ip: row.try_get("last_ip").map_err(map_sqlx_err)?,
                total_requests: row.try_get("cnt").map_err(map_sqlx_err)?,
                success_requests: row.try_get("ok_cnt").map_err(map_sqlx_err)?,
                rate_limited_requests: row.try_get("rl_cnt").map_err(map_sqlx_err)?,
                prompt_tokens: row.try_get("ptoks").map_err(map_sqlx_err)?,
                cached_tokens: row.try_get("cachetoks").map_err(map_sqlx_err)?,
                completion_tokens: row.try_get("ctoks").map_err(map_sqlx_err)?,
                total_tokens: row.try_get("ttoks").map_err(map_sqlx_err)?,
                avg_tokens_per_sec: round2(row.try_get("avg_tps").map_err(map_sqlx_err)?),
                peak_tokens_per_sec: round2(row.try_get("peak_tps").map_err(map_sqlx_err)?),
                avg_ttft_ms: round2(row.try_get("avg_ttft").map_err(map_sqlx_err)?),
                avg_latency_ms: round2(row.try_get("avg_lat").map_err(map_sqlx_err)?),
                last_used_at: row.try_get("last_used").map_err(map_sqlx_err)?,
            });
        }

        // 5. Egress IP breakdown
        let mut ip_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT COALESCE(egress_ip, proxy_name) AS ip,
                    COUNT(1) AS cnt,
                    COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END), 0) AS ok_cnt,
                    COALESCE(SUM(total_tokens), 0) AS toks,
                    COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0) AS rl,
                    CAST(COALESCE(AVG(CASE WHEN tokens_per_sec IS NOT NULL AND tokens_per_sec > 0 THEN tokens_per_sec END), 0.0) AS REAL) AS avg_tps,
                    CAST(COALESCE(AVG(CASE WHEN ttft_ms IS NOT NULL THEN ttft_ms END), 0.0) AS REAL) AS avg_ttft,
                    CAST(COALESCE(AVG(latency_ms), 0.0) AS REAL) AS avg_lat
             FROM usage_logs",
        );
        push_since_filter(&mut ip_qb, " WHERE ", since);
        ip_qb.push(" GROUP BY COALESCE(egress_ip, proxy_name) ORDER BY cnt DESC");
        let ip_rows = ip_qb.build().fetch_all(&self.pool).await.map_err(map_sqlx_err)?;

        let mut by_ip = Vec::with_capacity(ip_rows.len());
        for row in ip_rows {
            by_ip.push(IpUsageAggregate {
                egress_ip: row.try_get("ip").map_err(map_sqlx_err)?,
                total_requests: row.try_get("cnt").map_err(map_sqlx_err)?,
                success_requests: row.try_get("ok_cnt").map_err(map_sqlx_err)?,
                total_tokens: row.try_get("toks").map_err(map_sqlx_err)?,
                rate_limited_requests: row.try_get("rl").map_err(map_sqlx_err)?,
                avg_tokens_per_sec: round2(row.try_get("avg_tps").map_err(map_sqlx_err)?),
                avg_ttft_ms: round2(row.try_get("avg_ttft").map_err(map_sqlx_err)?),
                avg_latency_ms: round2(row.try_get("avg_lat").map_err(map_sqlx_err)?),
            });
        }

        // 6. Model breakdown
        let mut model_qb: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT model,
                    MAX(provider_id) AS provider_id,
                    COUNT(1) AS cnt,
                    COALESCE(SUM(CASE WHEN status_code >= 200 AND status_code < 300 THEN 1 ELSE 0 END), 0) AS ok_cnt,
                    COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0) AS rl_cnt,
                    COALESCE(SUM(prompt_tokens), 0) AS ptoks,
                    COALESCE(SUM(cached_tokens), 0) AS cachetoks,
                    COALESCE(SUM(completion_tokens), 0) AS ctoks,
                    COALESCE(SUM(total_tokens), 0) AS toks,
                    CAST(COALESCE(AVG(CASE WHEN tokens_per_sec IS NOT NULL AND tokens_per_sec > 0 THEN tokens_per_sec END), 0.0) AS REAL) AS avg_tps,
                    CAST(COALESCE(MAX(tokens_per_sec), 0.0) AS REAL) AS peak_tps,
                    CAST(COALESCE(AVG(CASE WHEN ttft_ms IS NOT NULL THEN ttft_ms END), 0.0) AS REAL) AS avg_ttft,
                    CAST(COALESCE(AVG(latency_ms), 0.0) AS REAL) AS avg_lat,
                    MAX(created_at) AS last_used
             FROM usage_logs",
        );
        push_since_filter(&mut model_qb, " WHERE ", since);
        model_qb.push(" GROUP BY model ORDER BY cnt DESC, model ASC");
        let model_rows = model_qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        let mut by_model = Vec::with_capacity(model_rows.len());
        for row in model_rows {
            by_model.push(ModelUsageAggregate {
                model: row.try_get("model").map_err(map_sqlx_err)?,
                provider_id: row.try_get("provider_id").map_err(map_sqlx_err)?,
                total_requests: row.try_get("cnt").map_err(map_sqlx_err)?,
                success_requests: row.try_get("ok_cnt").map_err(map_sqlx_err)?,
                rate_limited_requests: row.try_get("rl_cnt").map_err(map_sqlx_err)?,
                prompt_tokens: row.try_get("ptoks").map_err(map_sqlx_err)?,
                cached_tokens: row.try_get("cachetoks").map_err(map_sqlx_err)?,
                completion_tokens: row.try_get("ctoks").map_err(map_sqlx_err)?,
                total_tokens: row.try_get("toks").map_err(map_sqlx_err)?,
                avg_tokens_per_sec: round2(row.try_get("avg_tps").map_err(map_sqlx_err)?),
                peak_tokens_per_sec: round2(row.try_get("peak_tps").map_err(map_sqlx_err)?),
                avg_ttft_ms: round2(row.try_get("avg_ttft").map_err(map_sqlx_err)?),
                avg_latency_ms: round2(row.try_get("avg_lat").map_err(map_sqlx_err)?),
                last_used_at: row.try_get("last_used").map_err(map_sqlx_err)?,
            });
        }

        let recent = self.list_recent_logs_since(recent_limit, since).await?;

        Ok(UsageSummary {
            period,
            total_requests,
            success_requests,
            rate_limited_requests,
            error_requests,
            prompt_tokens,
            cached_tokens,
            completion_tokens,
            total_tokens,
            avg_tokens_per_sec,
            peak_tokens_per_sec,
            avg_ttft_ms,
            avg_latency_ms,
            timeseries,
            by_provider,
            by_api_key,
            by_proxy,
            by_ip,
            by_model,
            recent,
        })
    }

    async fn build_timeseries(
        &self,
        period: &str,
        since: Option<i64>,
        now: i64,
    ) -> Result<Vec<UsageTimeBucket>> {
        let local_now = Local::now();
        let (bucket_ms, slots): (i64, Vec<(i64, String)>) = match period {
            "today" => {
                let today_start = since.unwrap_or(now - 86_400_000);
                let list = (0..24)
                    .map(|h| {
                        let ts = today_start + h * 3_600_000;
                        let lbl = format!("{h:02}:00");
                        (ts, lbl)
                    })
                    .collect();
                (3_600_000, list)
            }
            "24h" => {
                let cur_hour_ts = (now / 3_600_000) * 3_600_000;
                let start = cur_hour_ts - 23 * 3_600_000;
                let list = (0..24)
                    .map(|h| {
                        let ts = start + h * 3_600_000;
                        let lbl = Local
                            .timestamp_millis_opt(ts)
                            .earliest()
                            .map(|dt| dt.format("%H:00").to_string())
                            .unwrap_or_else(|| format!("{h:02}:00"));
                        (ts, lbl)
                    })
                    .collect();
                (3_600_000, list)
            }
            "7d" => {
                let today = local_now.date_naive();
                let list = (0..7)
                    .map(|idx| {
                        let d = today - ChronoDuration::days(6 - idx);
                        let ts = d
                            .and_hms_opt(0, 0, 0)
                            .and_then(|dt| Local.from_local_datetime(&dt).earliest())
                            .map(|dt| dt.timestamp_millis())
                            .unwrap_or(now);
                        (ts, d.format("%b %d").to_string())
                    })
                    .collect();
                (86_400_000, list)
            }
            _ => {
                // "30d" or "all": last 30 local days
                let today = local_now.date_naive();
                let list = (0..30)
                    .map(|idx| {
                        let d = today - ChronoDuration::days(29 - idx);
                        let ts = d
                            .and_hms_opt(0, 0, 0)
                            .and_then(|dt| Local.from_local_datetime(&dt).earliest())
                            .map(|dt| dt.timestamp_millis())
                            .unwrap_or(now);
                        (ts, d.format("%b %d").to_string())
                    })
                    .collect();
                (86_400_000, list)
            }
        };

        let first_ts = slots.first().map(|(ts, _)| *ts).unwrap_or(now);
        let query_since = since.map(|s| s.max(first_ts)).unwrap_or(first_ts);

        let rows = sqlx::query(
            "SELECT created_at, status_code, prompt_tokens, cached_tokens, completion_tokens,
                    total_tokens, tokens_per_sec, ttft_ms
             FROM usage_logs
             WHERE created_at >= ?",
        )
        .bind(query_since)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        #[derive(Default)]
        struct Acc {
            requests: i64,
            success: i64,
            rl: i64,
            prompt: i64,
            cached: i64,
            completion: i64,
            total: i64,
            tps_sum: f64,
            tps_cnt: i64,
            ttft_sum: i64,
            ttft_cnt: i64,
        }

        let mut accs = vec![(); slots.len()]
            .into_iter()
            .map(|_| Acc::default())
            .collect::<Vec<_>>();

        for row in rows {
            let created_at: i64 = row.try_get("created_at").map_err(map_sqlx_err)?;
            let idx = ((created_at - first_ts) / bucket_ms) as isize;
            if idx < 0 || idx as usize >= accs.len() {
                continue;
            }
            let a = &mut accs[idx as usize];
            let status: i64 = row.try_get("status_code").map_err(map_sqlx_err)?;
            let ptok: i64 = row.try_get("prompt_tokens").map_err(map_sqlx_err)?;
            let cachetok: i64 = row.try_get("cached_tokens").map_err(map_sqlx_err)?;
            let ctok: i64 = row.try_get("completion_tokens").map_err(map_sqlx_err)?;
            let ttok: i64 = row.try_get("total_tokens").map_err(map_sqlx_err)?;
            let tps: Option<f64> = row.try_get("tokens_per_sec").map_err(map_sqlx_err)?;
            let ttft: Option<i64> = row.try_get("ttft_ms").map_err(map_sqlx_err)?;

            a.requests += 1;
            if (200..300).contains(&status) {
                a.success += 1;
            } else if status == 429 {
                a.rl += 1;
            }
            a.prompt += ptok;
            a.cached += cachetok;
            a.completion += ctok;
            a.total += ttok;
            if let Some(v) = tps
                && v > 0.0
            {
                a.tps_sum += v;
                a.tps_cnt += 1;
            }
            if let Some(t) = ttft
                && t > 0
            {
                a.ttft_sum += t;
                a.ttft_cnt += 1;
            }
        }

        Ok(slots
            .into_iter()
            .zip(accs)
            .map(|((ts, label), a)| UsageTimeBucket {
                timestamp: ts,
                label,
                requests: a.requests,
                success_requests: a.success,
                rate_limited_requests: a.rl,
                prompt_tokens: a.prompt,
                cached_tokens: a.cached,
                completion_tokens: a.completion,
                total_tokens: a.total,
                avg_tokens_per_sec: if a.tps_cnt > 0 {
                    round2(a.tps_sum / a.tps_cnt as f64)
                } else {
                    0.0
                },
                avg_ttft_ms: if a.ttft_cnt > 0 {
                    round2(a.ttft_sum as f64 / a.ttft_cnt as f64)
                } else {
                    0.0
                },
            })
            .collect())
    }
}

fn row_to_usage(row: &sqlx::sqlite::SqliteRow) -> Result<UsageLogRecord> {
    Ok(UsageLogRecord {
        id: row.try_get("id").map_err(map_sqlx_err)?,
        request_id: row.try_get("request_id").map_err(map_sqlx_err)?,
        api_key_id: row.try_get("api_key_id").map_err(map_sqlx_err)?,
        provider_id: row.try_get("provider_id").map_err(map_sqlx_err)?,
        model: row.try_get("model").map_err(map_sqlx_err)?,
        proxy_name: row.try_get("proxy_name").map_err(map_sqlx_err)?,
        egress_ip: row.try_get("egress_ip").map_err(map_sqlx_err)?,
        status_code: row.try_get("status_code").map_err(map_sqlx_err)?,
        prompt_tokens: row.try_get("prompt_tokens").map_err(map_sqlx_err)?,
        cached_tokens: row.try_get("cached_tokens").map_err(map_sqlx_err)?,
        completion_tokens: row.try_get("completion_tokens").map_err(map_sqlx_err)?,
        total_tokens: row.try_get("total_tokens").map_err(map_sqlx_err)?,
        latency_ms: row.try_get("latency_ms").map_err(map_sqlx_err)?,
        ttft_ms: row.try_get("ttft_ms").map_err(map_sqlx_err)?,
        tokens_per_sec: row.try_get("tokens_per_sec").map_err(map_sqlx_err)?,
        error_kind: row.try_get("error_kind").map_err(map_sqlx_err)?,
        error_detail: row.try_get("error_detail").map_err(map_sqlx_err)?,
        upstream_url: row.try_get("upstream_url").map_err(map_sqlx_err)?,
        attempt_count: row.try_get("attempt_count").map_err(map_sqlx_err)?,
        attempt_trace: row.try_get("attempt_trace").map_err(map_sqlx_err)?,
        session_id: row.try_get("session_id").map_err(map_sqlx_err)?,
        wire_format: row.try_get("wire_format").map_err(map_sqlx_err)?,
        created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
    })
}
