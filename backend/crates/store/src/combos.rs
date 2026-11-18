use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tesie_core::{ComboRecord, Result, TesRouteError};
use uuid::Uuid;

use crate::{map_sqlx_err, now_ms};

#[derive(Clone, Debug)]
pub struct CombosRepo {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateComboInput {
    pub name: String,
    pub strategy: Option<String>,
    pub models: Vec<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UpdateComboInput {
    pub name: Option<String>,
    pub strategy: Option<String>,
    pub models: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

impl CombosRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn list_combos(&self) -> Result<Vec<ComboRecord>> {
        let rows = sqlx::query(
            "SELECT id, name, strategy, models_json, enabled, created_at, updated_at
             FROM combos
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        rows.iter().map(row_to_combo).collect()
    }

    pub async fn list_enabled(&self) -> Result<Vec<ComboRecord>> {
        let rows = sqlx::query(
            "SELECT id, name, strategy, models_json, enabled, created_at, updated_at
             FROM combos
             WHERE enabled = 1
             ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        rows.iter().map(row_to_combo).collect()
    }

    pub async fn get_by_id(&self, id: &str) -> Result<Option<ComboRecord>> {
        let row = sqlx::query(
            "SELECT id, name, strategy, models_json, enabled, created_at, updated_at
             FROM combos
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        row.as_ref().map(row_to_combo).transpose()
    }

    pub async fn get_by_name(&self, name: &str) -> Result<Option<ComboRecord>> {
        let clean = name.trim();
        let row = sqlx::query(
            "SELECT id, name, strategy, models_json, enabled, created_at, updated_at
             FROM combos
             WHERE LOWER(name) = LOWER(?)",
        )
        .bind(clean)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        row.as_ref().map(row_to_combo).transpose()
    }

    pub async fn create_combo(&self, input: CreateComboInput) -> Result<ComboRecord> {
        let name = validate_combo_name(&input.name)?;
        let models = sanitize_models(&input.models)?;
        let strategy = normalize_strategy(input.strategy.as_deref().unwrap_or("fallback"));
        let enabled = input.enabled.unwrap_or(true);

        if self.get_by_name(&name).await?.is_some() {
            return Err(TesRouteError::Conflict(format!(
                "combo '{name}' already exists"
            )));
        }

        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        let models_json = serde_json::to_string(&models)
            .map_err(|e| TesRouteError::Internal(format!("serialize combo models: {e}")))?;

        sqlx::query(
            "INSERT INTO combos (id, name, strategy, models_json, enabled, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&name)
        .bind(&strategy)
        .bind(&models_json)
        .bind(if enabled { 1_i64 } else { 0_i64 })
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_by_id(&id)
            .await?
            .ok_or_else(|| TesRouteError::Store("created combo missing".into()))
    }

    pub async fn update_combo(&self, id: &str, input: UpdateComboInput) -> Result<ComboRecord> {
        let existing = self
            .get_by_id(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("combo {id} not found")))?;

        let next_name = match input.name {
            Some(n) => {
                let valid = validate_combo_name(&n)?;
                if !valid.eq_ignore_ascii_case(&existing.name)
                    && self.get_by_name(&valid).await?.is_some()
                {
                    return Err(TesRouteError::Conflict(format!(
                        "combo '{valid}' already exists"
                    )));
                }
                valid
            }
            None => existing.name.clone(),
        };

        let next_strategy = input
            .strategy
            .as_deref()
            .map(normalize_strategy)
            .unwrap_or(existing.strategy);

        let next_models = match input.models {
            Some(ms) => sanitize_models(&ms)?,
            None => existing.models,
        };

        let next_enabled = input.enabled.unwrap_or(existing.enabled);
        let now = now_ms();
        let models_json = serde_json::to_string(&next_models)
            .map_err(|e| TesRouteError::Internal(format!("serialize combo models: {e}")))?;

        sqlx::query(
            "UPDATE combos
             SET name = ?, strategy = ?, models_json = ?, enabled = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&next_name)
        .bind(&next_strategy)
        .bind(&models_json)
        .bind(if next_enabled { 1_i64 } else { 0_i64 })
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx_err)?;

        self.get_by_id(id)
            .await?
            .ok_or_else(|| TesRouteError::NotFound(format!("combo {id} not found")))
    }

    pub async fn delete_combo(&self, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM combos WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx_err)?;
        if res.rows_affected() == 0 {
            return Err(TesRouteError::NotFound(format!("combo {id} not found")));
        }
        Ok(())
    }
}

fn validate_combo_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(TesRouteError::BadRequest(
            "combo name cannot be empty".into(),
        ));
    }
    if trimmed.len() > 64 {
        return Err(TesRouteError::BadRequest(
            "combo name must be 64 characters or fewer".into(),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(TesRouteError::BadRequest(
            "combo name may only contain letters, digits, '-', '_', or '.'".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn sanitize_models(raw: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for m in raw {
        let trimmed = m.trim();
        if !trimmed.is_empty() && !out.contains(&trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    if out.is_empty() {
        return Err(TesRouteError::BadRequest(
            "combo must include at least one target model".into(),
        ));
    }
    Ok(out)
}

fn normalize_strategy(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "round-robin" | "round_robin" | "rr" => "round-robin".to_string(),
        "random" => "random".to_string(),
        _ => "fallback".to_string(),
    }
}

fn row_to_combo(row: &sqlx::sqlite::SqliteRow) -> Result<ComboRecord> {
    let models_str: String = row.try_get("models_json").map_err(map_sqlx_err)?;
    let models: Vec<String> = serde_json::from_str(&models_str).unwrap_or_default();
    Ok(ComboRecord {
        id: row.try_get("id").map_err(map_sqlx_err)?,
        name: row.try_get("name").map_err(map_sqlx_err)?,
        strategy: row.try_get("strategy").map_err(map_sqlx_err)?,
        models,
        enabled: row.try_get::<i64, _>("enabled").map_err(map_sqlx_err)? != 0,
        created_at: row.try_get("created_at").map_err(map_sqlx_err)?,
        updated_at: row.try_get("updated_at").map_err(map_sqlx_err)?,
    })
}
