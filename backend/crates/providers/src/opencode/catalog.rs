use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;
use tesie_core::{
    AntigravityQuotaSnapshot, ProviderKind, canonical_antigravity_model,
    curated_models_for_provider, display_name_for_model, is_excluded_antigravity_model_key,
    model_level_capabilities,
};
use tesie_proxy::{MihomoController, RelayRequest};
use tesie_store::{Store, now_ms};

const CACHE_TTL_MS: i64 = 10 * 60 * 1000;
const CAPABILITIES_URL: &str = "https://models.opencode.ai/api.json";

#[derive(Debug, Clone, Serialize)]
pub struct OpenAiModelEntry {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_levels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<String>>,
}

fn infer_model_capabilities(
    id: &str,
    live_attachment: Option<bool>,
    live_reasoning: Option<bool>,
    live_modalities: Option<Vec<String>>,
    live_tool_call: Option<bool>,
) -> (Option<bool>, Option<bool>, Option<Vec<String>>, Option<bool>) {
    let lower = id.to_ascii_lowercase();
    let is_gemini = lower.contains("gemini");
    let is_claude = lower.contains("claude") || lower.contains("sonnet") || lower.contains("opus");
    let is_gpt_vis = lower.contains("gpt-4o") || lower.contains("o1") || lower.contains("o3") || lower.contains("gpt-4.5");
    let is_vl = lower.contains("-vl") || lower.contains("vision") || lower.contains("internvl") || lower.contains("qwen-vl");

    let attachment = live_attachment.or_else(|| {
        if is_gemini || is_claude || is_gpt_vis || is_vl {
            Some(true)
        } else {
            None
        }
    });

    let reasoning = live_reasoning.or_else(|| {
        if lower.contains("thinking")
            || lower.contains("reasoning")
            || lower.contains("reasoner")
            || lower.contains("r1")
            || lower.contains("qwq")
            || lower.contains("o1")
            || lower.contains("o3")
            || lower.contains("beam")
        {
            Some(true)
        } else if is_gemini && lower.contains("3.8") {
            Some(true)
        } else {
            None
        }
    });

    let modalities = live_modalities.or_else(|| {
        let mut mods = vec!["text".to_string()];
        if attachment == Some(true) {
            mods.push("image".to_string());
        }
        if is_gemini || lower.contains("audio") || lower.contains("whisper") {
            mods.push("audio".to_string());
        }
        Some(mods)
    });

    let tool_call = live_tool_call.or(Some(true));

    (attachment, reasoning, modalities, tool_call)
}

#[derive(Debug, Default)]
pub struct ModelCatalogCache {
    live_free: Mutex<Option<(i64, Vec<OpenAiModelEntry>)>>,
}

impl ModelCatalogCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn list_models(
        &self,
        store: &Store,
        mihomo: Option<&Arc<MihomoController>>,
        zen_base_url: &str,
        fetch_live: bool,
    ) -> Vec<OpenAiModelEntry> {
        let now = now_ms();
        let created_sec = now / 1000;
        let mut seen = HashSet::new();
        let mut out = Vec::new();

        if let Ok(combos) = store.combos().list_enabled().await {
            for c in combos {
                if seen.insert(c.name.clone()) {
                    let (attachment, reasoning, modalities, tool_call) =
                        infer_model_capabilities(&c.name, None, None, None, None);
                    out.push(OpenAiModelEntry {
                        display_name: c.name.clone(),
                        id: c.name,
                        object: "model",
                        created: created_sec,
                        owned_by: "combo".to_string(),
                        supported_levels: Vec::new(),
                        default_level: None,
                        attachment,
                        reasoning,
                        modalities,
                        tool_call,
                    });
                }
            }
        }

        let oc_enabled = store
            .provider_configs()
            .get_or_default("opencode")
            .await
            .map(|c| c.enabled)
            .unwrap_or(true);

        if oc_enabled {
            let mut pushed_live = false;
            if fetch_live {
                let live = self.get_or_fetch_live_free(mihomo, zen_base_url, now).await;
                if !live.is_empty() {
                    pushed_live = true;
                    for entry in live {
                        if seen.insert(entry.id.clone()) {
                            out.push(entry);
                        }
                    }
                }
            }
            if !pushed_live {
                for m in curated_models_for_provider(ProviderKind::OpenCodeFree) {
                    if seen.insert(m.id.to_string()) {
                        let (supported_levels, default_level) = model_level_capabilities(
                            ProviderKind::OpenCodeFree,
                            m.id,
                            Some(true),
                            None,
                        );
                        let (attachment, reasoning, modalities, tool_call) =
                            infer_model_capabilities(m.id, None, None, None, None);
                        out.push(OpenAiModelEntry {
                            id: m.id.to_string(),
                            object: "model",
                            created: created_sec,
                            owned_by: "opencode".to_string(),
                            display_name: m.name.to_string(),
                            supported_levels,
                            default_level,
                            attachment,
                            reasoning,
                            modalities,
                            tool_call,
                        });
                    }
                }
            }
        }

        // Include ONLY unlocked/available merged Antigravity models when at least 1 enabled account exists
        let ag_enabled = store
            .provider_configs()
            .get_or_default("antigravity")
            .await
            .map(|c| c.enabled)
            .unwrap_or(true);
        if ag_enabled
            && let Ok(conns) = store.connections().list_for_provider("antigravity").await
        {
            let enabled_conns: Vec<_> = conns
                .into_iter()
                .filter(|c| {
                    c.enabled
                        && !(c.cooldown_until.is_some_and(|u| u > now)
                            && c.last_error.as_deref().is_some_and(|e| {
                                let l = e.to_ascii_lowercase();
                                l.contains("service disabled") || l.contains("terms of service")
                            }))
                })
                .collect();
            if !enabled_conns.is_empty() {
                let mut unlocked_ids = Vec::new();
                let mut any_snapshot_loaded = false;
                for conn in &enabled_conns {
                    if let Some(raw) = conn.quota_json.as_deref()
                        && let Ok(snap) = serde_json::from_str::<AntigravityQuotaSnapshot>(raw)
                        && snap.fetched_at > 0
                    {
                        any_snapshot_loaded = true;
                        for entry in snap.models {
                            if !entry.unlocked || is_excluded_antigravity_model_key(&entry.model) {
                                continue;
                            }
                            let (canon_id, _) = canonical_antigravity_model(&entry.model);
                            if !is_excluded_antigravity_model_key(&canon_id)
                                && !unlocked_ids.contains(&canon_id)
                            {
                                unlocked_ids.push(canon_id);
                            }
                        }
                    }
                }

                if any_snapshot_loaded {
                    for id in unlocked_ids {
                        if seen.insert(id.clone()) {
                            let (supported_levels, default_level) = model_level_capabilities(
                                ProviderKind::Antigravity,
                                &id,
                                None,
                                None,
                            );
                            let (attachment, reasoning, modalities, tool_call) =
                                infer_model_capabilities(&id, None, None, None, None);
                            out.push(OpenAiModelEntry {
                                display_name: display_name_for_model(ProviderKind::Antigravity, &id),
                                id,
                                object: "model",
                                created: created_sec,
                                owned_by: "antigravity".to_string(),
                                supported_levels,
                                default_level,
                                attachment,
                                reasoning,
                                modalities,
                                tool_call,
                            });
                        }
                    }
                } else {
                    for m in curated_models_for_provider(ProviderKind::Antigravity) {
                        if seen.insert(m.id.to_string()) {
                            let (supported_levels, default_level) = model_level_capabilities(
                                ProviderKind::Antigravity,
                                m.id,
                                None,
                                None,
                            );
                            let (attachment, reasoning, modalities, tool_call) =
                                infer_model_capabilities(m.id, None, None, None, None);
                            out.push(OpenAiModelEntry {
                                id: m.id.to_string(),
                                object: "model",
                                created: created_sec,
                                owned_by: "antigravity".to_string(),
                                display_name: m.name.to_string(),
                                supported_levels,
                                default_level,
                                attachment,
                                reasoning,
                                modalities,
                                tool_call,
                            });
                        }
                    }
                }
            }
        }

        // Include configured Custom Providers (OpenAI/Anthropic compatible)
        if let Ok(custom_list) = store.custom_providers().list_enabled().await {
            for cp in custom_list {
                let cfg_enabled = store
                    .provider_configs()
                    .get_or_default(&cp.id)
                    .await
                    .map(|c| c.enabled)
                    .unwrap_or(true);
                if !cfg_enabled {
                    continue;
                }
                let levels = vec![
                    "auto".to_string(),
                    "none".to_string(),
                    "low".to_string(),
                    "medium".to_string(),
                    "high".to_string(),
                ];
                for m in cp.models {
                    let full_id = if m.contains('/') && m.starts_with(&format!("{}/", cp.prefix)) {
                        m.clone()
                    } else {
                        format!("{}/{}", cp.prefix, m)
                    };
                    if seen.insert(full_id.clone()) {
                        let (attachment, reasoning, modalities, tool_call) =
                            infer_model_capabilities(&m, None, None, None, None);
                        out.push(OpenAiModelEntry {
                            id: full_id,
                            object: "model",
                            created: created_sec,
                            owned_by: cp.id.clone(),
                            display_name: m,
                            supported_levels: levels.clone(),
                            default_level: Some("auto".to_string()),
                            attachment,
                            reasoning,
                            modalities,
                            tool_call,
                        });
                    }
                }
            }
        }

        out
    }

    async fn get_or_fetch_live_free(
        &self,
        mihomo: Option<&Arc<MihomoController>>,
        _zen_base_url: &str,
        now: i64,
    ) -> Vec<OpenAiModelEntry> {
        {
            let guard = self.live_free.lock().unwrap();
            if let Some((expires_at, cached)) = guard.as_ref()
                && now < *expires_at
            {
                return cached.clone();
            }
        }

        let Some(ctl) = mihomo else {
            return Vec::new();
        };

        // Query OpenCode's authoritative capability & deprecation catalog (models.opencode.ai/api.json)
        let resp = ctl
            .relay(RelayRequest {
                target_url: CAPABILITIES_URL.to_string(),
                proxy_name: "DIRECT".to_string(),
                method: "GET".to_string(),
                fingerprint: "opencode".to_string(),
                timeout_ms: 8000,
                upstream_authorization: None,
                headers: vec![
                    ("Accept".into(), "application/json".into()),
                    ("User-Agent".into(), "opencode/latest/2.1.1/cli".into()),
                ],
                body: Bytes::new(),
            })
            .await;

        let Ok(r) = resp else {
            return Vec::new();
        };
        if !r.status().is_success() {
            return Vec::new();
        }
        let Ok(json) = r.json::<Value>().await else {
            return Vec::new();
        };

        let created_sec = now / 1000;
        let mut filtered = Vec::new();
        if let Some(models_map) = json
            .get("opencode")
            .and_then(|p| p.get("models"))
            .and_then(Value::as_object)
        {
            let mut ids: Vec<&String> = models_map.keys().collect();
            ids.sort();
            for key in ids {
                let Some(m) = models_map.get(key) else {
                    continue;
                };
                let status = m.get("status").and_then(Value::as_str).unwrap_or("active");
                if status == "deprecated" || status == "disabled" || status == "retired" {
                    continue;
                }
                let id = m
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(key.as_str())
                    .trim()
                    .to_string();
                let cost = m.get("cost");
                let zero_cost = cost.is_some_and(|c| {
                    c.get("input").and_then(Value::as_f64) == Some(0.0)
                        && c.get("output").and_then(Value::as_f64) == Some(0.0)
                });
                let is_free = id.to_ascii_lowercase().contains("free") || zero_cost;
                if is_free && !id.is_empty() {
                    let live_reasoning = m.get("reasoning").and_then(Value::as_bool);
                    let live_attachment = m.get("attachment").and_then(Value::as_bool);
                    let live_tool_call = m.get("tool_call").and_then(Value::as_bool);
                    let live_modalities: Option<Vec<String>> = m
                        .get("modalities")
                        .and_then(|mod_val| mod_val.get("input"))
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        });
                    let live_variants: Vec<String> = m
                        .get("variants")
                        .and_then(Value::as_object)
                        .map(|obj| obj.keys().cloned().collect())
                        .unwrap_or_default();
                    let (supported_levels, default_level) = model_level_capabilities(
                        ProviderKind::OpenCodeFree,
                        &id,
                        live_reasoning,
                        (!live_variants.is_empty()).then_some(live_variants.as_slice()),
                    );
                    let (attachment, reasoning, modalities, tool_call) = infer_model_capabilities(
                        &id,
                        live_attachment,
                        live_reasoning,
                        live_modalities,
                        live_tool_call,
                    );
                    let display_name = m
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| display_name_for_model(ProviderKind::OpenCodeFree, &id));
                    filtered.push(OpenAiModelEntry {
                        id,
                        object: "model",
                        created: created_sec,
                        owned_by: "opencode".to_string(),
                        display_name,
                        supported_levels,
                        default_level,
                        attachment,
                        reasoning,
                        modalities,
                        tool_call,
                    });
                }
            }
        }

        if !filtered.is_empty() {
            let mut guard = self.live_free.lock().unwrap();
            *guard = Some((now + CACHE_TTL_MS, filtered.clone()));
        }
        filtered
    }
}
