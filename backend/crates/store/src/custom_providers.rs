use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{CustomProvider, Result, TesRouteError};
use url::Url;

use crate::provider_configs::ProviderConfigsRepo;
use crate::{map_sqlx_err, now_ms};

const RESERVED_PREFIXES: &[&str] = &["oc", "opencode", "ag", "antigravity", "combo"];

#[derive(Clone, Debug)]
pub struct CustomProvidersRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateCustomProviderInput {
    pub name: String,
    pub prefix: String,
    #[serde(default = "default_openai_protocol")]
    pub protocol: String,
    #[serde(default = "default_chat_api_type")]
    pub api_type: String,
    pub base_url: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub custom_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub favicon_url: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateCustomProviderInput {
    pub name: Option<String>,
    pub prefix: Option<String>,
    pub protocol: Option<String>,
    pub api_type: Option<String>,
    pub base_url: Option<String>,
    pub api_keys: Option<Vec<String>>,
    pub models: Option<Vec<String>>,
    pub custom_headers: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "crate::deserialize_double_option")]
    pub favicon_url: Option<Option<String>>,
    pub color: Option<String>,
    pub enabled: Option<bool>,
}

fn default_openai_protocol() -> String {
    "openai".to_string()
}

fn default_chat_api_type() -> String {
    "chat".to_string()
}

impl CustomProvidersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<CustomProvider>> {
        let rows = sqlx::query(
            "SELECT id, name, prefix, protocol, api_type, base_url,
                    api_keys_json, models_json, custom_headers_json,
                    favicon_url, color, enabled, created_at, updated_at
             FROM custom_providers
             ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        rows.iter().map(row_to_custom_provider).collect()
    }

    pub async fn list_enabled(&self) -> Result<Vec<CustomProvider>> {
        let rows = sqlx::query(
            "SELECT id, name, prefix, protocol, api_type, base_url,
                    api_keys_json, models_json, custom_headers_json,
                    favicon_url, color, enabled, created_at, updated_at
             FROM custom_providers
             WHERE enabled = 1
             ORDER BY created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        rows.iter().map(row_to_custom_provider).collect()
    }

    pub async fn get(&self, id: &str) -> Result<Option<CustomProvider>> {
        let row = sqlx::query(
            "SELECT id, name, prefix, protocol, api_type, base_url,
                    api_keys_json, models_json, custom_headers_json,
                    favicon_url, color, enabled, created_at, updated_at
             FROM custom_providers
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        row.as_ref().map(row_to_custom_provider).transpose()
    }

    pub async fn get_by_prefix(&self, prefix: &str) -> Result<Option<CustomProvider>> {
        let clean = prefix.trim().to_ascii_lowercase();
        let row = sqlx::query(
            "SELECT id, name, prefix, protocol, api_type, base_url,
                    api_keys_json, models_json, custom_headers_json,
                    favicon_url, color, enabled, created_at, updated_at
             FROM custom_providers
             WHERE LOWER(prefix) = ?",
        )
        .bind(&clean)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        row.as_ref().map(row_to_custom_provider).transpose()
    }

    pub async fn create(&self, input: CreateCustomProviderInput) -> Result<CustomProvider> {
        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(TesRouteError::BadRequest(
                "custom provider name cannot be empty".into(),
            ));
        }
        let prefix = validate_prefix(&input.prefix)?;
        if self.get_by_prefix(&prefix).await?.is_some() {
            return Err(TesRouteError::Conflict(format!(
                "prefix '{prefix}' is already in use by another custom provider"
            )));
        }

        let base_url = validate_base_url(&input.base_url)?;
        let (protocol, api_type) = normalize_protocol_and_type(&input.protocol, &input.api_type);
        let api_keys = sanitize_list(input.api_keys);
        let models = sanitize_list(input.models);
        let custom_headers = sanitize_headers(input.custom_headers);
        let favicon_url = input
            .favicon_url
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let color = input
            .color
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if protocol == "anthropic" {
                    "#0284C7".to_string()
                } else {
                    "#0EA5E9".to_string()
                }
            });
        let enabled = input.enabled.unwrap_or(true);

        let mut id = format!("cp-{prefix}");
        if self.get(&id).await?.is_some() {
            id = format!("cp-{prefix}-{}", &uuid::Uuid::new_v4().to_string()[..6]);
        }

        let now = now_ms();
        self.insert_or_replace_raw(CustomProvider {
            id: id.clone(),
            name,
            prefix,
            protocol,
            api_type,
            base_url,
            api_keys,
            models,
            custom_headers,
            favicon_url,
            color,
            enabled,
            created_at: now,
            updated_at: now,
        })
        .await?;

        // Seed default provider_configs row (Direct connection + Fallback routing)
        let _ = ProviderConfigsRepo::new(self.pool.clone())
            .get_or_default(&id)
            .await;

        self.get(&id)
            .await?
            .ok_or_else(|| TesRouteError::Store("created custom provider missing".into()))
    }

    pub async fn insert_or_replace_raw(&self, cp: CustomProvider) -> Result<()> {
        let keys_json = serde_json::to_string(&cp.api_keys).unwrap_or_else(|_| "[]".to_string());
        let models_json = serde_json::to_string(&cp.models).unwrap_or_else(|_| "[]".to_string());
        let headers_json =
            serde_json::to_string(&cp.custom_headers).unwrap_or_else(|_| "{}".to_string());

        sqlx::query(
            "INSERT INTO custom_providers (
                id, name, prefix, protocol, api_type, base_url,
                api_keys_json, models_json, custom_headers_json,
                favicon_url, color, enabled, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                prefix = excluded.prefix,
                protocol = excluded.protocol,
                api_type = excluded.api_type,
                base_url = excluded.base_url,
                api_keys_json = excluded.api_keys_json,
                models_json = excluded.models_json,
                custom_headers_json = excluded.custom_headers_json,
                favicon_url = excluded.favicon_url,
                color = excluded.color,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
        )
        .bind(&cp.id)
        .bind(&cp.name)
        .bind(&cp.prefix)
        .bind(&cp.protocol)
        .bind(&cp.api_type)
        .bind(&cp.base_url)
        .bind(&keys_json)
        .bind(&models_json)
        .bind(&headers_json)
        .bind(&cp.favicon_url)
        .bind(&cp.color)
        .bind(if cp.enabled { 1_i64 } else { 0_i64 })
        .bind(cp.created_at)
        .bind(cp.updated_at)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;
        Ok(())
    }

    pub async fn update(&self, id: &str, input: UpdateCustomProviderInput) -> Result<CustomProvider> {
        let current = self
            .get(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("custom provider {id} not found")))?;

        let name = match input.name {
            Some(n) => {
                let clean = n.trim().to_string();
                if clean.is_empty() {
                    return Err(TesRouteError::BadRequest(
                        "custom provider name cannot be empty".into(),
                    ));
                }
                clean
            }
            None => current.name,
        };

        let prefix = match input.prefix {
            Some(p) => {
                let clean = validate_prefix(&p)?;
                if let Some(other) = self.get_by_prefix(&clean).await?
                    && other.id != id
                {
                    return Err(TesRouteError::Conflict(format!(
                        "prefix '{clean}' is already in use"
                    )));
                }
                clean
            }
            None => current.prefix,
        };

        let base_url = match input.base_url {
            Some(u) => validate_base_url(&u)?,
            None => current.base_url,
        };

        let next_proto = input.protocol.unwrap_or(current.protocol);
        let next_api_type = input.api_type.unwrap_or(current.api_type);
        let (protocol, api_type) = normalize_protocol_and_type(&next_proto, &next_api_type);
        let api_keys = match input.api_keys {
            Some(v) => sanitize_list(v),
            None => current.api_keys,
        };
        let models = match input.models {
            Some(v) => sanitize_list(v),
            None => current.models,
        };
        let custom_headers = match input.custom_headers {
            Some(h) => sanitize_headers(h),
            None => current.custom_headers,
        };
        let favicon_url = match input.favicon_url {
            Some(opt) => opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            None => current.favicon_url,
        };
        let color = input
            .color
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(current.color);
        let enabled = input.enabled.unwrap_or(current.enabled);
        let now = now_ms();

        self.insert_or_replace_raw(CustomProvider {
            id: id.to_string(),
            name,
            prefix,
            protocol,
            api_type,
            base_url,
            api_keys,
            models,
            custom_headers,
            favicon_url,
            color,
            enabled,
            created_at: current.created_at,
            updated_at: now,
        })
        .await?;

        self.get(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("custom provider {id} not found")))
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM custom_providers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if res.rows_affected() == 0 {
            return Err(TesRouteError::NotFound(format!(
                "custom provider {id} not found"
            )));
        }
        let _ = ProviderConfigsRepo::new(self.pool.clone())
            .delete_config(id)
            .await;
        Ok(())
    }
}

pub fn validate_prefix(raw: &str) -> Result<String> {
    let clean = raw.trim().to_ascii_lowercase();
    if clean.is_empty() || clean.len() > 24 {
        return Err(TesRouteError::BadRequest(
            "prefix must be 1..24 characters (lowercase letters, digits, '-' or '_')".into(),
        ));
    }
    if RESERVED_PREFIXES.contains(&clean.as_str()) {
        return Err(TesRouteError::BadRequest(format!(
            "prefix '{clean}' is reserved"
        )));
    }
    let mut chars = clean.chars();
    let Some(first) = chars.next() else {
        return Err(TesRouteError::BadRequest("prefix cannot be empty".into()));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(TesRouteError::BadRequest(
            "prefix must start with an alphanumeric character".into(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(TesRouteError::BadRequest(
            "prefix can only contain lowercase letters, digits, '-' or '_'".into(),
        ));
    }
    Ok(clean)
}

pub fn validate_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    let parsed = Url::parse(&trimmed)
        .map_err(|e| TesRouteError::BadRequest(format!("invalid base_url '{raw}': {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(TesRouteError::BadRequest(
            "base_url must use http:// or https://".into(),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(TesRouteError::BadRequest(
            "base_url must contain a valid hostname".into(),
        ));
    }
    Ok(trimmed)
}

fn normalize_protocol_and_type(protocol: &str, api_type: &str) -> (String, String) {
    let p = match protocol.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "anthropic-compatible" | "claude" => "anthropic",
        _ => "openai",
    };
    let t = if p == "anthropic" {
        "messages".to_string()
    } else if api_type.trim().eq_ignore_ascii_case("responses") {
        "responses".to_string()
    } else {
        "chat".to_string()
    };
    (p.to_string(), t)
}

fn sanitize_list(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

fn sanitize_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .filter_map(|(k, v)| {
            let clean_k = k.trim().to_string();
            let clean_v = v.trim().to_string();
            if clean_k.is_empty() || clean_v.is_empty() {
                None
            } else {
                Some((clean_k, clean_v))
            }
        })
        .collect()
}

fn row_to_custom_provider(row: &sqlx::sqlite::SqliteRow) -> Result<CustomProvider> {
    let raw_keys: String = row
        .try_get("api_keys_json")
        .unwrap_or_else(|_| "[]".to_string());
    let raw_models: String = row
        .try_get("models_json")
        .unwrap_or_else(|_| "[]".to_string());
    let raw_headers: String = row
        .try_get("custom_headers_json")
        .unwrap_or_else(|_| "{}".to_string());

    Ok(CustomProvider {
        id: row.try_get("id").map_err(map_sqlx_err)?,
        name: row.try_get("name").map_err(map_sqlx_err)?,
        prefix: row.try_get("prefix").map_err(map_sqlx_err)?,
        protocol: row.try_get("protocol").map_err(map_sqlx_err)?,
        api_type: row.try_get("api_type").map_err(map_sqlx_err)?,
        base_url: row.try_get("base_url").map_err(map_sqlx_err)?,
        api_keys: serde_json::from_str(&raw_keys).unwrap_or_default(),
        models: serde_json::from_str(&raw_models).unwrap_or_default(),
        custom_headers: serde_json::from_str(&raw_headers).unwrap_or_default(),
        favicon_url: row.try_get("favicon_url").map_err(map_sqlx_err)?,
        color: row.try_get("color").map_err(map_sqlx_err)?,
        enabled: row.try_get::<i64, _>("enabled").map_err(map_sqlx_err)? != 0,
        created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx_err)?,
    })
}
