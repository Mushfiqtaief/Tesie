use std::collections::HashSet;

use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use tesie_core::{ProviderConfig, ProxyNode, Result, TesRouteError};
use url::Url;
use uuid::Uuid;

use crate::quotas::QuotasRepo;
use crate::settings::SettingsRepo;
use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct ProxiesRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct CreateProxyInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub proxy_type: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub raw_outbound_json: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateProxyInput {
    pub name: Option<String>,
    pub proxy_type: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub username: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub password: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub proxy_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub raw_outbound_json: Option<Option<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedProxyEndpoint {
    pub proxy_type: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl ProxiesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_proxies(&self) -> Result<Vec<ProxyNode>> {
        let rows = sqlx::query(
            "SELECT id, name, proxy_type, proxy_url, raw_outbound_json, is_direct, enabled, alive,
                    egress_ip, last_delay_ms, last_probed_at, consecutive_failures,
                    created_at, updated_at
             FROM proxy_nodes",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let mut nodes: Vec<ProxyNode> = rows
            .into_iter()
            .map(|r| row_to_proxy(&r))
            .collect::<Result<_>>()?;
        sort_proxies_naturally(&mut nodes);
        Ok(nodes)
    }

    pub async fn list_enabled(&self, strict_proxy_mode: bool) -> Result<Vec<ProxyNode>> {
        let mode = if strict_proxy_mode {
            "pool_strict"
        } else {
            "pool_with_direct"
        };
        self.list_for_mode(mode).await
    }

    pub async fn list_for_mode(&self, proxy_mode: &str) -> Result<Vec<ProxyNode>> {
        let all = self.list_proxies().await?;
        let external_only: Vec<ProxyNode> = all
            .iter()
            .filter(|p| p.enabled && !p.is_direct)
            .cloned()
            .collect();
        let direct_only: Vec<ProxyNode> = all
            .iter()
            .filter(|p| p.enabled && p.is_direct)
            .cloned()
            .collect();

        match proxy_mode {
            "pool_strict" => Ok(external_only),
            "all" => {
                if external_only.is_empty() {
                    Ok(direct_only)
                } else {
                    Ok(external_only)
                }
            }
            "direct" | "none" => Ok(direct_only),
            _ => Ok(all.into_iter().filter(|p| p.enabled).collect()),
        }
    }

    /// Resolves the active proxy list for a provider configuration:
    /// - `"all"`: all external proxies in the global pool (falls back to `DIRECT` if pool is empty)
    /// - `"selected"`: only proxies in `cfg.selected_proxy_ids` (0 selected => `DIRECT`)
    /// - `"direct"` / `"none"`: `DIRECT` outbound only (no proxy)
    /// - Legacy `"pool_strict"` / `"pool_with_direct"`: preserved for backwards compatibility
    pub async fn list_for_provider_config(&self, cfg: &ProviderConfig) -> Result<Vec<ProxyNode>> {
        let all = self.list_proxies().await?;
        let direct_nodes: Vec<ProxyNode> = all
            .iter()
            .filter(|p| p.enabled && p.is_direct)
            .cloned()
            .collect();

        match cfg.proxy_mode.as_str() {
            "direct" | "none" => Ok(direct_nodes),
            "selected" => {
                if cfg.selected_proxy_ids.is_empty() {
                    return Ok(direct_nodes);
                }
                let wanted: HashSet<&str> =
                    cfg.selected_proxy_ids.iter().map(String::as_str).collect();
                let selected: Vec<ProxyNode> = all
                    .into_iter()
                    .filter(|p| p.enabled && !p.is_direct && wanted.contains(p.id.as_str()))
                    .collect();
                if selected.is_empty() {
                    Ok(direct_nodes)
                } else {
                    Ok(selected)
                }
            }
            "pool_strict" => Ok(all
                .into_iter()
                .filter(|p| p.enabled && !p.is_direct)
                .collect()),
            "pool_with_direct" | "pool" => Ok(all.into_iter().filter(|p| p.enabled).collect()),
            // Default / "all"
            _ => {
                let external: Vec<ProxyNode> = all
                    .into_iter()
                    .filter(|p| p.enabled && !p.is_direct)
                    .collect();
                if external.is_empty() {
                    Ok(direct_nodes)
                } else {
                    Ok(external)
                }
            }
        }
    }

    pub async fn get_proxy(&self, id_or_name: &str) -> Result<Option<ProxyNode>> {
        let row = sqlx::query(
            "SELECT id, name, proxy_type, proxy_url, raw_outbound_json, is_direct, enabled, alive,
                    egress_ip, last_delay_ms, last_probed_at, consecutive_failures,
                    created_at, updated_at
             FROM proxy_nodes
             WHERE id = ? OR name = ?",
        )
        .bind(id_or_name)
        .bind(id_or_name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        row.as_ref().map(row_to_proxy).transpose()
    }

    /// Computes the next sequential `NODE - <N>` number from existing external proxies.
    pub async fn next_node_index(&self) -> Result<usize> {
        let rows = sqlx::query("SELECT name FROM proxy_nodes WHERE is_direct = 0")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        let mut max_num = 0usize;
        for r in rows {
            if let Ok(name) = r.try_get::<String, _>("name")
                && let Some(n) = parse_node_number(&name)
            {
                max_num = max_num.max(n);
            }
        }
        Ok(max_num + 1)
    }

    pub async fn upsert_proxy(&self, input: CreateProxyInput) -> Result<ProxyNode> {
        let name = if input.name.trim().is_empty() {
            let idx = self.next_node_index().await?;
            format!("NODE - {idx}")
        } else {
            input.name.trim().to_string()
        };

        if name.eq_ignore_ascii_case("DIRECT") {
            return Err(TesRouteError::BadRequest(
                "DIRECT is reserved for the built-in direct outbound".into(),
            ));
        }

        let built_url = if let (Some(host), Some(port)) = (
            input.host.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            input.port.filter(|&p| p > 0),
        ) {
            let ptype = input.proxy_type.as_deref().unwrap_or("socks5");
            Some(build_proxy_url(
                ptype,
                host,
                port,
                input.username.as_deref(),
                input.password.as_deref(),
            )?)
        } else {
            input
                .proxy_url
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        let raw_json = input
            .raw_outbound_json
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if built_url.is_none() && raw_json.is_none() {
            return Err(TesRouteError::BadRequest(
                "either host+port, proxy_url, or raw_outbound_json is required".into(),
            ));
        }

        let parsed = parse_proxy_endpoint(false, built_url.as_deref(), raw_json.as_deref());
        let now = now_ms();
        let enabled = true;
        let id = Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO proxy_nodes (
                id, name, proxy_type, proxy_url, raw_outbound_json, is_direct, enabled, alive,
                consecutive_failures, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, 0, ?, 1, 0, ?, ?)
             ON CONFLICT(name) DO UPDATE SET
                proxy_type = excluded.proxy_type,
                proxy_url = excluded.proxy_url,
                raw_outbound_json = excluded.raw_outbound_json,
                enabled = excluded.enabled,
                alive = 1,
                egress_ip = NULL,
                last_delay_ms = NULL,
                updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&name)
        .bind(&parsed.proxy_type)
        .bind(&built_url)
        .bind(&raw_json)
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let node = self
            .get_proxy(&name)
            .await?
            .ok_or_else(|| TesRouteError::Store("upserted proxy missing".into()))?;

        let settings = SettingsRepo::new(self.pool.clone()).get_settings().await?;
        QuotasRepo::new(self.pool.clone())
            .ensure_ip_quota_row(&node.ip_key(), &settings, now)
            .await?;

        Ok(node)
    }

    /// Batch-imports proxy URLs and assigns uniform sequential names `NODE - 1`, `NODE - 2`, ...
    /// continuing seamlessly from the highest existing `NODE - k` in the pool.
    pub async fn bulk_import_proxy_urls(&self, urls_text: &str) -> Result<Vec<ProxyNode>> {
        let existing = self.list_proxies().await?;
        let mut by_url = std::collections::HashMap::new();
        for p in &existing {
            if !p.is_direct
                && let Some(u) = p.proxy_url.as_deref().map(str::trim)
                && !u.is_empty()
            {
                by_url.insert(strip_url_fragment(u).to_string(), p.clone());
            }
        }

        let mut next_idx = self.next_node_index().await?;
        let mut out = Vec::new();
        for raw_line in urls_text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let clean_url = strip_url_fragment(line);
            if let Some(already) = by_url.get(clean_url) {
                out.push(already.clone());
                continue;
            }

            let name = format!("NODE - {next_idx}");
            next_idx += 1;
            let node = self
                .upsert_proxy(CreateProxyInput {
                    name,
                    proxy_url: Some(clean_url.to_string()),
                    enabled: Some(true),
                    ..Default::default()
                })
                .await?;
            by_url.insert(clean_url.to_string(), node.clone());
            out.push(node);
        }
        Ok(out)
    }

    pub async fn update_proxy(&self, id: &str, input: UpdateProxyInput) -> Result<ProxyNode> {
        let existing = self
            .get_proxy(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("proxy {id} not found")))?;

        let next_name = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&existing.name)
            .to_string();

        if existing.is_direct && next_name != "DIRECT" {
            return Err(TesRouteError::BadRequest(
                "cannot rename built-in DIRECT outbound".into(),
            ));
        }
        if !existing.is_direct && next_name.eq_ignore_ascii_case("DIRECT") {
            return Err(TesRouteError::BadRequest(
                "DIRECT is reserved for the built-in direct outbound".into(),
            ));
        }

        // Check name collision with another proxy
        if !next_name.eq_ignore_ascii_case(&existing.name)
            && let Some(other) = self.get_proxy(&next_name).await?
            && other.id != existing.id
        {
            return Err(TesRouteError::Conflict(format!(
                "another proxy named '{next_name}' already exists"
            )));
        }

        let has_structured_edit = input.host.is_some()
            || input.port.is_some()
            || input.proxy_type.is_some()
            || input.username.is_some()
            || input.password.is_some();

        let next_url = if has_structured_edit {
            let ptype = input
                .proxy_type
                .as_deref()
                .unwrap_or(&existing.proxy_type);
            let host = input
                .host
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(existing.host.as_deref())
                .or(existing.ip.as_deref())
                .ok_or_else(|| TesRouteError::BadRequest("proxy host/IP cannot be empty".into()))?;
            let port = input
                .port
                .filter(|&p| p > 0)
                .or(existing.port)
                .ok_or_else(|| TesRouteError::BadRequest("valid proxy port (1-65535) is required".into()))?;
            let username = match &input.username {
                Some(opt) => opt.as_deref(),
                None => existing.username.as_deref(),
            };
            let password = match &input.password {
                Some(opt) => opt.as_deref(),
                None => existing.password.as_deref(),
            };
            Some(build_proxy_url(ptype, host, port, username, password)?)
        } else {
            input.proxy_url.unwrap_or_else(|| existing.proxy_url.clone())
        };

        let next_raw = if has_structured_edit && input.raw_outbound_json.is_none() {
            None
        } else {
            input
                .raw_outbound_json
                .unwrap_or_else(|| existing.raw_outbound_json.clone())
        };

        let endpoint_changed = next_url != existing.proxy_url || next_raw != existing.raw_outbound_json;
        let parsed = parse_proxy_endpoint(
            existing.is_direct,
            next_url.as_deref(),
            next_raw.as_deref(),
        );
        let now = now_ms();

        if endpoint_changed {
            sqlx::query(
                "UPDATE proxy_nodes
                 SET name = ?, proxy_type = ?, proxy_url = ?, raw_outbound_json = ?,
                     enabled = 1, alive = 1, egress_ip = NULL, last_delay_ms = NULL,
                     consecutive_failures = 0, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&next_name)
            .bind(&parsed.proxy_type)
            .bind(&next_url)
            .bind(&next_raw)
            .bind(now)
            .bind(&existing.id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        } else {
            sqlx::query(
                "UPDATE proxy_nodes
                 SET name = ?, proxy_type = ?, proxy_url = ?, raw_outbound_json = ?, enabled = 1, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&next_name)
            .bind(&parsed.proxy_type)
            .bind(&next_url)
            .bind(&next_raw)
            .bind(now)
            .bind(&existing.id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        }

        if existing.name != next_name {
            let old_unprobed = format!("unprobed:{}", existing.name);
            let _ = sqlx::query("DELETE FROM ip_quotas WHERE ip_key = ?")
                .bind(&old_unprobed)
                .execute(&self.pool)
                .await;
        }

        let updated = self.get_proxy(&existing.id).await?.ok_or_else(|| {
            TesRouteError::NotFound(format!("proxy {} missing after update", existing.id))
        })?;

        let settings = SettingsRepo::new(self.pool.clone()).get_settings().await?;
        QuotasRepo::new(self.pool.clone())
            .ensure_ip_quota_row(&updated.ip_key(), &settings, now)
            .await?;

        Ok(updated)
    }

    pub async fn delete_proxy(&self, id: &str) -> Result<()> {
        let existing = self
            .get_proxy(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("proxy {id} not found")))?;
        if existing.is_direct {
            return Err(TesRouteError::BadRequest(
                "cannot delete built-in DIRECT outbound".into(),
            ));
        }
        self.delete_proxies_batch(&[existing.id]).await?;
        Ok(())
    }

    pub async fn delete_proxies_batch(&self, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx_err)?;
        let mut deleted = 0usize;

        for raw_id in ids {
            let id = raw_id.trim();
            if id.is_empty() {
                continue;
            }
            let row = sqlx::query("SELECT id, name, is_direct FROM proxy_nodes WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
            let Some(row) = row else {
                continue;
            };
            let is_direct = row.try_get::<i64, _>("is_direct").map_err(map_sqlx_err)? != 0;
            if is_direct {
                continue;
            }
            let name: String = row.try_get("name").map_err(map_sqlx_err)?;

            let res = sqlx::query("DELETE FROM proxy_nodes WHERE id = ? AND is_direct = 0")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
            if res.rows_affected() > 0 {
                deleted += 1;
                let unprobed_key = format!("unprobed:{name}");
                sqlx::query("DELETE FROM ip_quotas WHERE ip_key = ?")
                    .bind(&unprobed_key)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_sqlx_err)?;
            }
        }

        tx.commit().await.map_err(map_sqlx_err)?;
        Ok(deleted)
    }

    /// Renames all external proxies sequentially to `NODE - 1`, `NODE - 2`, ..., `NODE - N`
    /// preserving their existing pool order.
    pub async fn normalize_all_external_names_to_nodes(&self) -> Result<usize> {
        let mut external: Vec<ProxyNode> = self
            .list_proxies()
            .await?
            .into_iter()
            .filter(|p| !p.is_direct)
            .collect();
        if external.is_empty() {
            return Ok(0);
        }

        // Preserve legacy proxy-1, proxy-2 or existing NODE - 1, NODE - 2 order
        external.sort_by_key(|p| {
            let legacy_idx = parse_node_number(&p.name).or_else(|| parse_legacy_proxy_index(&p.name));
            (
                legacy_idx.unwrap_or(usize::MAX),
                p.created_at,
                p.name.clone(),
            )
        });

        let now = now_ms();
        let mut tx = self.pool.begin().await.map_err(map_sqlx_err)?;

        // Phase 1: move to unique temporary names to avoid UNIQUE(name) collisions
        for p in &external {
            let tmp = format!("__tesroute_tmp_{}", p.id);
            sqlx::query("UPDATE proxy_nodes SET name = ? WHERE id = ?")
                .bind(&tmp)
                .bind(&p.id)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;
        }

        // Phase 2: assign sequential `NODE - 1`, `NODE - 2`, ...
        let mut renamed = 0usize;
        for (i, p) in external.iter().enumerate() {
            let target_name = format!("NODE - {}", i + 1);
            sqlx::query("UPDATE proxy_nodes SET name = ?, updated_at = ? WHERE id = ?")
                .bind(&target_name)
                .bind(now)
                .bind(&p.id)
                .execute(&mut *tx)
                .await
                .map_err(map_sqlx_err)?;

            if p.name != target_name {
                renamed += 1;
                let old_unprobed = format!("unprobed:{}", p.name);
                let new_unprobed = format!("unprobed:{target_name}");
                let _ = sqlx::query("UPDATE OR IGNORE ip_quotas SET ip_key = ? WHERE ip_key = ?")
                    .bind(&new_unprobed)
                    .bind(&old_unprobed)
                    .execute(&mut *tx)
                    .await;
                let _ = sqlx::query("DELETE FROM ip_quotas WHERE ip_key = ?")
                    .bind(&old_unprobed)
                    .execute(&mut *tx)
                    .await;
            }
        }

        tx.commit().await.map_err(map_sqlx_err)?;
        Ok(renamed)
    }

    pub async fn update_proxy_probe(
        &self,
        id: &str,
        egress_ip: &str,
        delay_ms: i64,
        now: i64,
    ) -> Result<ProxyNode> {
        let existing = self
            .get_proxy(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("proxy {id} not found")))?;
        let clean_ip = egress_ip.trim();
        let next_delay = if delay_ms > 0 {
            Some(delay_ms)
        } else {
            existing.last_delay_ms
        };
        sqlx::query(
            "UPDATE proxy_nodes
             SET egress_ip = ?, last_delay_ms = ?, alive = 1, last_probed_at = ?,
                 consecutive_failures = 0, updated_at = ?
             WHERE id = ?",
        )
        .bind(clean_ip)
        .bind(next_delay)
        .bind(now)
        .bind(now)
        .bind(&existing.id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        let unprobed_key = format!("unprobed:{}", existing.name);
        sqlx::query("DELETE FROM ip_quotas WHERE ip_key = ?")
            .bind(&unprobed_key)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;

        let settings = SettingsRepo::new(self.pool.clone()).get_settings().await?;
        QuotasRepo::new(self.pool.clone())
            .ensure_ip_quota_row(clean_ip, &settings, now)
            .await?;

        self.get_proxy(&existing.id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("proxy {id} not found")))
    }

    pub async fn sync_mihomo_delay(
        &self,
        proxy_name: &str,
        alive: bool,
        delay_ms: Option<i64>,
        now: i64,
    ) -> Result<()> {
        if alive {
            sqlx::query(
                "UPDATE proxy_nodes
                 SET alive = 1,
                     last_delay_ms = COALESCE(?, last_delay_ms),
                     last_probed_at = ?,
                     consecutive_failures = 0,
                     updated_at = ?
                 WHERE name = ?",
            )
            .bind(delay_ms.filter(|&d| d > 0))
            .bind(now)
            .bind(now)
            .bind(proxy_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        } else {
            sqlx::query(
                "UPDATE proxy_nodes
                 SET alive = 0,
                     last_delay_ms = NULL,
                     last_probed_at = ?,
                     consecutive_failures = consecutive_failures + 1,
                     updated_at = ?
                 WHERE name = ?",
            )
            .bind(now)
            .bind(now)
            .bind(proxy_name)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        }
        Ok(())
    }

    pub async fn record_proxy_failure(&self, id: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE proxy_nodes
             SET consecutive_failures = consecutive_failures + 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn reset_proxy_failures(&self, id: &str, now: i64) -> Result<()> {
        sqlx::query(
            "UPDATE proxy_nodes
             SET consecutive_failures = 0, alive = 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }
}

pub fn sort_proxies_naturally(nodes: &mut [ProxyNode]) {
    nodes.sort_by(|a, b| {
        a.is_direct
            .cmp(&b.is_direct)
            .then_with(|| {
                let na = parse_node_number(&a.name);
                let nb = parse_node_number(&b.name);
                match (na, nb) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.created_at.cmp(&b.created_at),
                }
            })
            .then_with(|| a.name.cmp(&b.name))
    });
}

pub fn parse_node_number(name: &str) -> Option<usize> {
    let trimmed = name.trim();
    let rest = trimmed
        .strip_prefix("NODE")
        .or_else(|| trimmed.strip_prefix("node"))
        .or_else(|| trimmed.strip_prefix("Node"))?;
    let digits = rest.trim_start_matches([' ', '-', '_', '#']);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<usize>().ok()
}

fn parse_legacy_proxy_index(name: &str) -> Option<usize> {
    let rest = name.trim().strip_prefix("proxy-")?;
    let num_part = rest.split('-').next()?;
    num_part.parse::<usize>().ok()
}

fn strip_url_fragment(raw: &str) -> &str {
    raw.split('#').next().unwrap_or(raw).trim()
}

fn percent_decode_str(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) =
                ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16))
        {
            out.push(((hi << 4) | lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

pub fn build_proxy_url(
    raw_type: &str,
    raw_host: &str,
    port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<String> {
    let scheme = match raw_type.trim().to_ascii_lowercase().as_str() {
        "socks4" | "socks4a" => "socks4",
        "socks5" | "socks5h" | "socks" => "socks5",
        "http" => "http",
        "https" => "https",
        "ss" | "shadowsocks" => "ss",
        other => {
            return Err(TesRouteError::BadRequest(format!(
                "unsupported proxy type '{other}' (expected socks4, socks5, http, https, or ss)"
            )));
        }
    };

    let host = raw_host.trim().trim_matches(['[', ']']);
    if host.is_empty() {
        return Err(TesRouteError::BadRequest("proxy IP/host cannot be empty".into()));
    }
    let formatted_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };

    if scheme == "ss" {
        let cipher = username
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("aes-256-gcm");
        let pass = password.map(str::trim).unwrap_or("");
        let userinfo = URL_SAFE_NO_PAD.encode(format!("{cipher}:{pass}"));
        return Ok(format!("ss://{userinfo}@{formatted_host}:{port}"));
    }

    let mut base = Url::parse(&format!("{scheme}://{formatted_host}:{port}"))
        .map_err(|e| TesRouteError::BadRequest(format!("invalid proxy host/port: {e}")))?;

    if let Some(u) = username.map(str::trim).filter(|s| !s.is_empty()) {
        base.set_username(u)
            .map_err(|_| TesRouteError::BadRequest("invalid proxy username".into()))?;
        if let Some(p) = password.filter(|s| !s.is_empty()) {
            base.set_password(Some(p))
                .map_err(|_| TesRouteError::BadRequest("invalid proxy password".into()))?;
        }
    }

    Ok(base.to_string().trim_end_matches('/').to_string())
}

pub fn infer_proxy_type_and_host(
    is_direct: bool,
    proxy_url: Option<&str>,
    raw_outbound_json: Option<&str>,
) -> (String, Option<String>) {
    let parsed = parse_proxy_endpoint(is_direct, proxy_url, raw_outbound_json);
    (parsed.proxy_type, parsed.host)
}

pub fn parse_proxy_endpoint(
    is_direct: bool,
    proxy_url: Option<&str>,
    raw_outbound_json: Option<&str>,
) -> ParsedProxyEndpoint {
    if is_direct {
        return ParsedProxyEndpoint {
            proxy_type: "direct".to_string(),
            ..Default::default()
        };
    }

    if let Some(raw) = raw_outbound_json.map(str::trim).filter(|s| !s.is_empty())
        && let Ok(val) = serde_json::from_str::<Value>(raw)
        && let Some(obj) = val.as_object()
    {
        let proxy_type = obj
            .get("type")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "custom".to_string());
        let host = obj
            .get("server")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let port = obj
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok());
        let username = obj
            .get("username")
            .or_else(|| obj.get("cipher"))
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let password = obj
            .get("password")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        return ParsedProxyEndpoint {
            proxy_type,
            host,
            port,
            username,
            password,
        };
    }

    if let Some(raw_url) = proxy_url.map(str::trim).filter(|s| !s.is_empty()) {
        if raw_url.to_ascii_lowercase().starts_with("ss://") {
            return parse_ss_endpoint(raw_url);
        }
        if let Ok(parsed) = Url::parse(raw_url) {
            let proxy_type = match parsed.scheme().to_ascii_lowercase().as_str() {
                "socks4" | "socks4a" => "socks4".to_string(),
                "socks" | "socks5" | "socks5h" => "socks5".to_string(),
                "http" => "http".to_string(),
                "https" => "https".to_string(),
                other => other.to_string(),
            };
            let host = parsed
                .host_str()
                .map(|s| s.trim_matches(['[', ']']).to_string());
            let port = parsed.port_or_known_default().or_else(|| {
                if proxy_type.starts_with("socks") {
                    Some(1080)
                } else {
                    None
                }
            });
            let username = if parsed.username().is_empty() {
                None
            } else {
                Some(percent_decode_str(parsed.username()))
            };
            let password = parsed.password().map(percent_decode_str);
            return ParsedProxyEndpoint {
                proxy_type,
                host,
                port,
                username,
                password,
            };
        }
    }

    ParsedProxyEndpoint {
        proxy_type: "socks5".to_string(),
        ..Default::default()
    }
}

fn parse_ss_endpoint(raw: &str) -> ParsedProxyEndpoint {
    let mut out = ParsedProxyEndpoint {
        proxy_type: "ss".to_string(),
        ..Default::default()
    };
    let Some(without_scheme) = raw.get(5..) else {
        return out;
    };
    let without_frag = without_scheme.split('#').next().unwrap_or(without_scheme);
    let body = without_frag.split('?').next().unwrap_or(without_frag);
    let (userinfo, host_port) = if let Some((left, right)) = body.rsplit_once('@') {
        let decoded_left = URL_SAFE_NO_PAD
            .decode(left)
            .or_else(|_| URL_SAFE.decode(left))
            .or_else(|_| STANDARD.decode(left))
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_else(|| left.to_string());
        (Some(decoded_left), right.to_string())
    } else if let Some(decoded) = URL_SAFE_NO_PAD
        .decode(body)
        .or_else(|_| URL_SAFE.decode(body))
        .or_else(|_| STANDARD.decode(body))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        && let Some((left, right)) = decoded.rsplit_once('@')
    {
        (Some(left.to_string()), right.to_string())
    } else {
        return out;
    };

    if let Some(ui) = userinfo
        && let Some((cipher, pass)) = ui.split_once(':')
    {
        if !cipher.is_empty() {
            out.username = Some(cipher.to_string());
        }
        if !pass.is_empty() {
            out.password = Some(pass.to_string());
        }
    }

    if let Ok(parsed) = Url::parse(&format!("http://{host_port}")) {
        out.host = parsed
            .host_str()
            .map(|s| s.trim_matches(['[', ']']).to_string());
        out.port = parsed.port();
    }
    out
}

pub(crate) fn row_to_proxy(row: &sqlx::sqlite::SqliteRow) -> Result<ProxyNode> {
    let is_direct = row.try_get::<i64, _>("is_direct").map_err(map_sqlx_err)? != 0;
    let proxy_url: Option<String> = row.try_get("proxy_url").map_err(map_sqlx_err)?;
    let raw_outbound_json: Option<String> =
        row.try_get("raw_outbound_json").map_err(map_sqlx_err)?;
    let egress_ip: Option<String> = row.try_get("egress_ip").map_err(map_sqlx_err)?;
    let last_delay_ms: Option<i64> = row.try_get("last_delay_ms").map_err(map_sqlx_err)?;
    let db_type: Option<String> = row.try_get("proxy_type").ok();
    let alive = row
        .try_get::<i64, _>("alive")
        .map(|v| v != 0)
        .unwrap_or(true);

    let parsed = parse_proxy_endpoint(is_direct, proxy_url.as_deref(), raw_outbound_json.as_deref());
    let proxy_type = db_type
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or(parsed.proxy_type);
    let ip = egress_ip
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| parsed.host.clone());

    Ok(ProxyNode {
        id: row.try_get("id").map_err(map_sqlx_err)?,
        name: row.try_get("name").map_err(map_sqlx_err)?,
        proxy_type,
        ip,
        host: parsed.host,
        port: parsed.port,
        username: parsed.username,
        password: parsed.password,
        latency_ms: if alive { last_delay_ms } else { None },
        alive,
        proxy_url,
        raw_outbound_json,
        is_direct,
        enabled: row.try_get::<i64, _>("enabled").map_err(map_sqlx_err)? != 0,
        egress_ip,
        last_delay_ms: if alive { last_delay_ms } else { None },
        last_probed_at: row.try_get("last_probed_at").map_err(map_sqlx_err)?,
        consecutive_failures: row.try_get("consecutive_failures").map_err(map_sqlx_err)?,
        created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx_err)?,
    })
}
