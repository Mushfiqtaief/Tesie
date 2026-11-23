pub mod aggregator;
pub mod bridge;
pub mod cache_engine;

pub use cache_engine::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde_json::{json, Value};
use tesie_core::{apply_token_saver_to_request, Result, TesRouteError, WireFormat};
use tesie_providers::{ModelCatalogCache, OpenCodeEngine, ProviderRequest};
use tesie_proxy::MihomoController;
use tesie_store::Store;
use uuid::Uuid;

use aggregator::{drain_and_aggregate, sse_error_payload};
use bridge::bridge_request_if_needed;

static COMBO_RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub struct GatewayState {
    pub store: Store,
    pub engine: Arc<OpenCodeEngine>,
    pub mihomo: Arc<MihomoController>,
    pub catalog: Arc<ModelCatalogCache>,
    pub require_api_key: bool,
    pub fetch_live_models: bool,
    pub memory_cache: MemoryCache,
}

pub fn gateway_router(state: GatewayState) -> Router {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/responses", post(handle_responses))
        .route("/v1/messages", post(handle_messages))
        .route("/v1/messages/count_tokens", post(handle_count_tokens))
        // Compatibility aliases when downstream CLIs (e.g. Claude Code) append `/v1/*` onto a `/v1` base URL
        .route("/v1/v1/models", get(list_models))
        .route("/v1/v1/chat/completions", post(handle_chat_completions))
        .route("/v1/v1/responses", post(handle_responses))
        .route("/v1/v1/messages", post(handle_messages))
        .route("/v1/v1/messages/count_tokens", post(handle_count_tokens))
        // Antigravity Native / MITM Proxy Endpoints for Google Cloud Code & IDE Interception
        .route("/v1internal:loadCodeAssist", post(handle_antigravity_load_code_assist).get(handle_antigravity_load_code_assist))
        .route("/v1internal/loadCodeAssist", post(handle_antigravity_load_code_assist).get(handle_antigravity_load_code_assist))
        .route("/v1internal:fetchUserInfo", post(handle_antigravity_fetch_user_info).get(handle_antigravity_fetch_user_info))
        .route("/v1internal/fetchUserInfo", post(handle_antigravity_fetch_user_info).get(handle_antigravity_fetch_user_info))
        .route("/v1internal:onboardUser", post(handle_antigravity_onboard_user).get(handle_antigravity_onboard_user))
        .route("/v1internal/onboardUser", post(handle_antigravity_onboard_user).get(handle_antigravity_onboard_user))
        .route("/v1internal:listCloudAICompanionProjects", post(handle_antigravity_list_companion_projects).get(handle_antigravity_list_companion_projects))
        .route("/v1internal/listCloudAICompanionProjects", post(handle_antigravity_list_companion_projects).get(handle_antigravity_list_companion_projects))
        .route("/v1internal:getCodeAssistGlobalUserSetting", post(handle_antigravity_user_settings).get(handle_antigravity_user_settings))
        .route("/v1internal/getCodeAssistGlobalUserSetting", post(handle_antigravity_user_settings).get(handle_antigravity_user_settings))
        .route("/v1internal:setCodeAssistGlobalUserSetting", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/setCodeAssistGlobalUserSetting", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:fetchAdminControls", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/fetchAdminControls", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:fetchCodeCustomizationState", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/fetchCodeCustomizationState", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:listExperiments", post(handle_antigravity_list_experiments).get(handle_antigravity_list_experiments))
        .route("/v1internal/listExperiments", post(handle_antigravity_list_experiments).get(handle_antigravity_list_experiments))
        .route("/v1internal:cascadeNuxes", post(handle_antigravity_list_cascade_nuxes).get(handle_antigravity_list_cascade_nuxes))
        .route("/v1internal/cascadeNuxes", post(handle_antigravity_list_cascade_nuxes).get(handle_antigravity_list_cascade_nuxes))
        .route("/v1internal:buildWithGooglePlugins", post(handle_antigravity_list_build_with_google_plugins).get(handle_antigravity_list_build_with_google_plugins))
        .route("/v1internal/buildWithGooglePlugins", post(handle_antigravity_list_build_with_google_plugins).get(handle_antigravity_list_build_with_google_plugins))
        .route("/v1internal:listModelConfigs", post(handle_antigravity_list_model_configs).get(handle_antigravity_list_model_configs))
        .route("/v1internal/listModelConfigs", post(handle_antigravity_list_model_configs).get(handle_antigravity_list_model_configs))
        .route("/v1internal:fetchAvailableModels", post(handle_antigravity_fetch_available_models).get(handle_antigravity_fetch_available_models))
        .route("/v1internal/fetchAvailableModels", post(handle_antigravity_fetch_available_models).get(handle_antigravity_fetch_available_models))
        .route("/v1internal:retrieveUserQuotaSummary", post(handle_antigravity_retrieve_user_quota_summary).get(handle_antigravity_retrieve_user_quota_summary))
        .route("/v1internal/retrieveUserQuotaSummary", post(handle_antigravity_retrieve_user_quota_summary).get(handle_antigravity_retrieve_user_quota_summary))
        .route("/v1internal:retrieveUserQuota", post(handle_antigravity_retrieve_user_quota_summary).get(handle_antigravity_retrieve_user_quota_summary))
        .route("/v1internal/retrieveUserQuota", post(handle_antigravity_retrieve_user_quota_summary).get(handle_antigravity_retrieve_user_quota_summary))
        .route("/v1internal:recordClientEvent", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/recordClientEvent", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:recordCodeAssistMetrics", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/recordCodeAssistMetrics", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:recordTrajectoryAnalytics", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/recordTrajectoryAnalytics", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:writeTrajectoryAcls", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal/writeTrajectoryAcls", post(handle_antigravity_generic_ok).get(handle_antigravity_generic_ok))
        .route("/v1internal:streamGenerateContent", post(handle_antigravity_stream_generate_content))
        .route("/v1internal/streamGenerateContent", post(handle_antigravity_stream_generate_content))
        .route("/v1internal:generateContent", post(handle_antigravity_generate_content))
        .route("/v1internal/generateContent", post(handle_antigravity_generate_content))
        .with_state(state)
}

async fn authorize_client(state: &GatewayState, headers: &HeaderMap) -> Result<Option<String>> {
    let has_keys = state.store.keys().has_any_enabled_keys().await?;
    if !state.require_api_key && !has_keys {
        return Ok(None);
    }

    let token = extract_bearer_or_api_key(headers).ok_or_else(|| {
        TesRouteError::Unauthorized("missing API key in Authorization or x-api-key header".into())
    })?;
    let rec = state.store.keys().verify_and_check_quota(token).await?;
    Ok(Some(rec.id))
}

fn extract_bearer_or_api_key(headers: &HeaderMap) -> Option<&str> {
    if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        let trimmed = auth.trim();
        if let Some(rest) = trimmed
            .strip_prefix("Bearer ")
            .or_else(|| trimmed.strip_prefix("bearer "))
        {
            let tok = rest.trim();
            if !tok.is_empty() {
                return Some(tok);
            }
        }
    }
    if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let tok = key.trim();
        if !tok.is_empty() {
            return Some(tok);
        }
    }
    None
}

async fn list_models(
    State(state): State<GatewayState>,
    headers: HeaderMap,
) -> std::result::Result<Json<Value>, Response> {
    if let Err(err) = authorize_client(&state, &headers).await {
        return Err(error_to_response(&err));
    }
    let models = state
        .catalog
        .list_models(
            &state.store,
            Some(&state.mihomo),
            &state.engine.config().zen_base_url,
            state.fetch_live_models,
        )
        .await;
    Ok(Json(json!({ "object": "list", "data": models })))
}

async fn handle_count_tokens(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(err) = authorize_client(&state, &headers).await {
        return error_to_response(&err);
    }
    let text_len = serde_json::to_string(&json!({
        "system": body.get("system"),
        "messages": body.get("messages"),
        "tools": body.get("tools"),
    }))
    .map(|s| s.len())
    .unwrap_or(4);
    let est_tokens = (text_len / 4).max(1);
    (StatusCode::OK, Json(json!({ "input_tokens": est_tokens }))).into_response()
}

async fn handle_chat_completions(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    handle_gateway_request(state, headers, body, WireFormat::OpenAiChat).await
}

async fn handle_responses(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    handle_gateway_request(state, headers, body, WireFormat::OpenAiResponses).await
}

async fn handle_messages(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    handle_gateway_request(state, headers, body, WireFormat::AnthropicMessages).await
}

async fn handle_antigravity_load_code_assist(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
    _body: axum::body::Bytes,
) -> Response {
    let body = json!({
        "cloudaicompanionProject": "projects/tesie-companion",
        "currentTier": {
            "id": "g1-pro-tier",
            "name": "Google AI Pro"
        },
        "paidTier": {
            "id": "g1-pro-tier",
            "name": "Google AI Pro"
        },
        "allowedTiers": [
            {
                "id": "g1-pro-tier",
                "name": "Google AI Pro",
                "isDefault": true
            }
        ],
        "ineligibleTiers": []
    });
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(body),
    )
        .into_response()
}

async fn handle_antigravity_fetch_user_info(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
    _body: axum::body::Bytes,
) -> Response {
    let body = json!({
        "userSettings": {
            "userTier": {
                "id": "g1-pro-tier",
                "name": "Google AI Pro"
            }
        },
        "userTags": [],
        "regionCode": "US"
    });
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(body),
    )
        .into_response()
}

async fn handle_antigravity_onboard_user(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
    _body: axum::body::Bytes,
) -> Response {
    let body = json!({
        "done": true,
        "response": {
            "@type": "type.googleapis.com/google.internal.cloud.code.v1internal.OnboardUserResponse",
            "cloudaicompanionProject": {
                "id": "projects/tesie-companion",
                "name": "Tesie Companion Project"
            }
        }
    });
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(body),
    )
        .into_response()
}

async fn handle_antigravity_list_companion_projects(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
    _body: axum::body::Bytes,
) -> Response {
    let body = json!({
        "projects": [
            {
                "id": "projects/tesie-companion",
                "name": "Tesie Companion Project"
            }
        ]
    });
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(body),
    )
        .into_response()
}

async fn handle_antigravity_user_settings(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
    _body: axum::body::Bytes,
) -> Response {
    let body = json!({
        "cloudaicompanionProject": "projects/tesie-companion"
    });
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(body),
    )
        .into_response()
}

async fn handle_antigravity_list_experiments(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({ "experiments": [] })),
    )
        .into_response()
}

async fn handle_antigravity_list_model_configs(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({ "modelConfigs": [] })),
    )
        .into_response()
}

/// `ListCascadeNuxesResponse`: IDE polls every 2 minutes, empty list is a valid no-op.
async fn handle_antigravity_list_cascade_nuxes(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({ "cascadeNuxes": [] })),
    )
        .into_response()
}

/// `ListBuildWithGooglePluginsResponse`: startup plugin-upgrade check, empty means none.
async fn handle_antigravity_list_build_with_google_plugins(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({ "buildWithGooglePlugins": [] })),
    )
        .into_response()
}

async fn handle_antigravity_generic_ok(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({ "status": "ok" })),
    )
        .into_response()
}

async fn handle_antigravity_retrieve_user_quota_summary(
    State(_state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    let body = json!({
        "groups": [
            {
                "displayName": "Antigravity Models",
                "buckets": [
                    {
                        "bucketId": "antigravity-weekly",
                        "displayName": "Weekly Limit",
                        "window": "weekly",
                        "remainingFraction": 1.0,
                        "resetTime": "2030-01-01T00:00:00Z"
                    },
                    {
                        "bucketId": "antigravity-5h",
                        "displayName": "Session Limit",
                        "window": "5h",
                        "remainingFraction": 1.0,
                        "resetTime": "2030-01-01T00:00:00Z"
                    }
                ]
            }
        ]
    });
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(body),
    )
        .into_response()
}

/// Required by `v1internal.ModelDetails.model`: the IDE drops entries whose
/// `exa.codeium_common_pb.Model` enum is unset.
fn antigravity_model_enum(id: &str) -> (&'static str, &'static str, &'static str) {
    let l = id.to_ascii_lowercase();
    if l.contains("claude") {
        let model = if l.contains("haiku") {
            "MODEL_CLAUDE_4_5_HAIKU"
        } else if l.contains("opus") {
            "MODEL_CLAUDE_4_OPUS"
        } else {
            "MODEL_CLAUDE_4_SONNET"
        };
        (
            model,
            "API_PROVIDER_ANTHROPIC_VERTEX",
            "MODEL_PROVIDER_ANTHROPIC",
        )
    } else if l.contains("gpt") || l.contains("oss") {
        (
            "MODEL_OPENAI_GPT_OSS_120B_MEDIUM",
            "API_PROVIDER_OPENAI_VERTEX",
            "MODEL_PROVIDER_OPENAI",
        )
    } else if l.contains("deepseek") {
        (
            "MODEL_DEEPSEEK_V3",
            "API_PROVIDER_INTERNAL",
            "MODEL_PROVIDER_DEEPSEEK",
        )
    } else if l.contains("grok") {
        (
            "MODEL_XAI_GROK_3",
            "API_PROVIDER_INTERNAL",
            "MODEL_PROVIDER_XAI",
        )
    } else if l.contains("lite") {
        (
            "MODEL_GOOGLE_GEMINI_2_5_FLASH_LITE",
            "API_PROVIDER_GOOGLE_GEMINI",
            "MODEL_PROVIDER_GOOGLE",
        )
    } else if l.contains("flash") {
        (
            "MODEL_GOOGLE_GEMINI_2_5_FLASH",
            "API_PROVIDER_GOOGLE_GEMINI",
            "MODEL_PROVIDER_GOOGLE",
        )
    } else {
        (
            "MODEL_GOOGLE_GEMINI_2_5_PRO",
            "API_PROVIDER_GOOGLE_GEMINI",
            "MODEL_PROVIDER_GOOGLE",
        )
    }
}

fn antigravity_model_details(id: &str, display: &str) -> serde_json::Value {
    let (model, api_provider, model_provider) = antigravity_model_enum(id);
    json!({
        "displayName": display,
        "model": model,
        "apiProvider": api_provider,
        "modelProvider": model_provider,
        "maxTokens": 1048576,
        "maxOutputTokens": 65536,
        "supportsImages": true,
        "supportsThinking": true,
        "recommended": false,
        "quotaInfo": {
            "remainingFraction": 1.0,
            "resetTime": "2030-01-01T00:00:00Z"
        }
    })
}

async fn handle_antigravity_fetch_available_models(
    State(state): State<GatewayState>,
    _headers: HeaderMap,
) -> Response {
    fn add(
        map: &mut serde_json::Map<String, serde_json::Value>,
        model_ids: &mut Vec<String>,
        id: String,
        display: String,
    ) {
        if !map.contains_key(&id) {
            model_ids.push(id.clone());
            map.insert(id.clone(), antigravity_model_details(&id, &display));
        }
    }

    let mut models_map = serde_json::Map::new();
    let mut model_ids: Vec<String> = Vec::new();
    let mut default_model_id = "gemini-3.8-flash".to_string();

    if let Ok(upstream) =
        tesie_providers::fetch_available_models_raw(&state.store, &state.mihomo).await
        && let Some(models) = upstream.get("models").and_then(Value::as_object)
    {
        for (id, val) in models {
            model_ids.push(id.clone());
            models_map.insert(id.clone(), val.clone());
        }
        if let Some(def_id) = upstream.get("defaultAgentModelId").and_then(Value::as_str) {
            default_model_id = def_id.to_string();
        }
    }
    for m in tesie_core::curated_models_for_provider(tesie_core::ProviderKind::Antigravity) {
        add(&mut models_map, &mut model_ids, m.id.to_string(), m.name.to_string());
    }
    let well_known = [
        ("gemini-3.8-flash", "Gemini 3.8 Flash"),
        ("gemini-4-argon", "Gemini 4 Argon"),
        ("gemini-4-hydrogen", "Gemini 4 Hydrogen"),
        ("gemini-4-neon", "Gemini 4 Neon"),
        ("gemini-4-carbon", "Gemini 4 Carbon"),
        ("claude-opus-5-5", "Claude Opus 5.5"),
        ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
        ("claude-opus-4-6-thinking", "Claude Opus 4.6"),
        ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        ("gpt-oss-120b-medium", "GPT-OSS 120B"),
        ("gemini-3-flash", "Gemini 3 Flash"),
        ("gemini-3.5-flash-lite", "Gemini 3.5 Flash Lite"),
        ("gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite"),
        ("gemini-2.5-flash", "Gemini 2.5 Flash"),
        ("gemini-2.5-flash-lite", "Gemini 2.5 Flash Lite"),
    ];
    for (id, name) in well_known {
        add(&mut models_map, &mut model_ids, id.to_string(), name.to_string());
    }

    // Populate active Combos so they are directly discoverable in IDE model pickers
    if let Ok(combos) = state.store.combos().list_combos().await {
        for c in combos {
            if c.enabled {
                add(
                    &mut models_map,
                    &mut model_ids,
                    c.name.clone(),
                    format!("Combo: {}", c.name),
                );
            }
        }
    }

    // Populate OVH and OpenCode models so IDEs can select them directly
    for m in tesie_core::curated_models_for_provider(tesie_core::ProviderKind::Ovh) {
        add(
            &mut models_map,
            &mut model_ids,
            m.id.to_string(),
            format!("OVH: {}", m.name),
        );
    }
    for m in tesie_core::curated_models_for_provider(tesie_core::ProviderKind::OpenCodeFree) {
        add(
            &mut models_map,
            &mut model_ids,
            m.id.to_string(),
            format!("OpenCode: {}", m.name),
        );
    }

    // Populate any configured MITM model aliases
    if let Ok(mitm_cfg) = state.store.mitm().get_config().await {
        for (src, dst) in &mitm_cfg.model_aliases {
            add(
                &mut models_map,
                &mut model_ids,
                src.clone(),
                format!("{src} (-> {dst})"),
            );
        }
    }

    (
        StatusCode::OK,
        [(CONTENT_TYPE, "application/json; charset=utf-8")],
        Json(json!({
            "models": models_map,
            "defaultAgentModelId": default_model_id,
            "agentModelSorts": [{
                "displayName": "Tesie",
                "groups": [{
                    "displayName": "Tesie Models",
                    "modelIds": model_ids
                }]
            }]
        })),
    )
        .into_response()
}

async fn handle_antigravity_stream_generate_content(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(raw_body): Json<Value>,
) -> Response {
    let mitm_cfg = state.store.mitm().get_config().await.ok();
    let combos = state.store.combos().list_combos().await.ok();
    let (_, openai_body) = normalize_antigravity_request_to_openai(
        &raw_body,
        true,
        mitm_cfg.as_ref(),
        combos.as_deref(),
    );
    let resp = handle_gateway_request_with_auth(state, headers, openai_body, WireFormat::OpenAiChat, true).await;

    let is_sse = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_sse {
        let (parts, body) = resp.into_parts();
        let mapped_stream = body.into_data_stream().map(|res| {
            res.map(map_openai_sse_to_gemini)
                .map_err(|e| std::io::Error::other(e.to_string()))
        });
        Response::from_parts(parts, Body::from_stream(mapped_stream))
    } else {
        resp
    }
}

async fn handle_antigravity_generate_content(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(raw_body): Json<Value>,
) -> Response {
    let mitm_cfg = state.store.mitm().get_config().await.ok();
    let combos = state.store.combos().list_combos().await.ok();
    let (_, openai_body) = normalize_antigravity_request_to_openai(
        &raw_body,
        false,
        mitm_cfg.as_ref(),
        combos.as_deref(),
    );
    let resp = handle_gateway_request_with_auth(state, headers, openai_body, WireFormat::OpenAiChat, true).await;
    resp
}

fn normalize_antigravity_request_to_openai(
    raw_body: &Value,
    is_stream: bool,
    mitm_config: Option<&tesie_core::AntigravityMitmConfig>,
    combos: Option<&[tesie_core::ComboRecord]>,
) -> (String, Value) {
    let raw_model = raw_body
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| {
            raw_body
                .get("request")
                .and_then(|r| r.get("model"))
                .and_then(Value::as_str)
        })
        .unwrap_or("gemini-3.8-flash");

    let clean_model_raw = raw_model.strip_prefix("models/").unwrap_or(raw_model).trim();

    // 1. Resolve tiered thinking models (e.g. gemini-3.8-flash-tiered -> gemini-3.8-flash-medium)
    let tiered_model = if clean_model_raw.ends_with("-tiered") {
        let level = raw_body
            .get("request")
            .and_then(|r| r.get("generationConfig"))
            .or_else(|| raw_body.get("generationConfig"))
            .and_then(|gc| gc.get("thinkingConfig"))
            .and_then(|tc| tc.get("thinkingLevel"))
            .and_then(Value::as_str)
            .unwrap_or("medium")
            .to_lowercase();
        let base = clean_model_raw.strip_suffix("-tiered").unwrap_or(clean_model_raw);
        format!("{base}-{level}")
    } else {
        clean_model_raw.to_string()
    };

    // 2. Resolve synonyms (matching 9router canonical aliases)
    let clean_model_str = match tiered_model.as_str() {
        "gemini-default" => "gemini-3.8-flash-medium",
        "claude-sonnet-5.5" => "claude-sonnet-5-5",
        "claude-opus-5.5" => "claude-opus-5-5",
        "claude-opus-4.6" => "claude-opus-4-6-thinking",
        other => other,
    };

    let is_tab_autocomplete = clean_model_str.starts_with("tab_");

    let mut resolved_target: Option<String> = None;

    if !is_tab_autocomplete
        && let Some(cfg) = mitm_config
        && cfg.enabled
    {
        // 1. Check explicit per-model alias mapping (exact or tiered)
        if let Some(target) = cfg.model_aliases.get(clean_model_str).filter(|s| !s.trim().is_empty()) {
            resolved_target = Some(target.clone());
        } else if let Some(target) = cfg.model_aliases.get(clean_model_raw).filter(|s| !s.trim().is_empty()) {
            resolved_target = Some(target.clone());
        }
        // 2. Check if clean_model matches a combo name directly
        if resolved_target.is_none()
            && let Some(c_list) = combos
            && c_list.iter().any(|c| c.name.eq_ignore_ascii_case(clean_model_str))
        {
            resolved_target = Some(format!("combo/{clean_model_str}"));
        }
        // 3. Fallback to default_target_model, except for explicitly routed ids
        let has_explicit_prefix = clean_model_str.starts_with("antigravity/")
            || clean_model_str.starts_with("oc/")
            || clean_model_str.starts_with("ovh/")
            || clean_model_str.starts_with("combo/");
        if resolved_target.is_none()
            && !has_explicit_prefix
            && let Some(def) = cfg
                .default_target_model
                .as_deref()
                .filter(|s| !s.trim().is_empty())
        {
            resolved_target = Some(def.to_string());
        }
    }

    let target_model = resolved_target.unwrap_or_else(|| {
        if clean_model_str.starts_with("antigravity/")
            || clean_model_str.starts_with("oc/")
            || clean_model_str.starts_with("ovh/")
            || clean_model_str.starts_with("combo/")
        {
            clean_model_str.to_string()
        } else if let Some(c_list) = combos
            && c_list.iter().any(|c| c.name.eq_ignore_ascii_case(clean_model_str))
        {
            format!("combo/{clean_model_str}")
        } else {
            format!("antigravity/{clean_model_str}")
        }
    });

    let mut messages: Vec<Value> = Vec::new();

    let sys_val = raw_body
        .get("request")
        .and_then(|r| r.get("systemInstruction"))
        .or_else(|| raw_body.get("systemInstruction"));
    if let Some(sys) = sys_val {
        if let Some(parts) = sys.get("parts").and_then(Value::as_array) {
            let mut text = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    text.push_str(t);
                }
            }
            if !text.is_empty() {
                messages.push(json!({
                    "role": "system",
                    "content": text
                }));
            }
        }
    }

    if let Some(existing_msgs) = raw_body.get("messages").and_then(Value::as_array) {
        messages.extend(existing_msgs.iter().cloned());
    } else {
        let contents = raw_body
            .get("request")
            .and_then(|r| r.get("contents"))
            .or_else(|| raw_body.get("contents"))
            .and_then(Value::as_array);

        if let Some(conts) = contents {
            for c in conts {
                let role = match c.get("role").and_then(Value::as_str).unwrap_or("user") {
                    "model" => "assistant",
                    other => other,
                };
                let mut text = String::new();
                let mut tool_calls = Vec::new();

                if let Some(parts) = c.get("parts").and_then(Value::as_array) {
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(Value::as_str) {
                            text.push_str(t);
                        }
                        if let Some(fc) = p.get("functionCall") {
                            let name = fc.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let args = fc.get("args").cloned().unwrap_or(json!({}));
                            let id = fc.get("id").and_then(Value::as_str).unwrap_or("call_1");
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(&args).unwrap_or_default()
                                }
                            }));
                        }
                    }
                }

                let mut msg_obj = serde_json::Map::new();
                msg_obj.insert("role".into(), Value::String(role.into()));
                msg_obj.insert("content".into(), Value::String(text));
                if !tool_calls.is_empty() {
                    msg_obj.insert("tool_calls".into(), Value::Array(tool_calls));
                }
                messages.push(Value::Object(msg_obj));
            }
        }
    }

    if messages.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": ""
        }));
    }

    let mut openai_body = json!({
        "model": target_model,
        "messages": messages,
        "stream": is_stream,
    });

    let gen_config = raw_body
        .get("request")
        .and_then(|r| r.get("generationConfig"))
        .or_else(|| raw_body.get("generationConfig"));

    if let Some(gc) = gen_config {
        if let Some(temp) = gc.get("temperature").and_then(Value::as_f64) {
            openai_body["temperature"] = json!(temp);
        }
        if let Some(top_p) = gc.get("topP").and_then(Value::as_f64) {
            openai_body["top_p"] = json!(top_p);
        }
        if let Some(max_tokens) = gc.get("maxOutputTokens").and_then(Value::as_u64) {
            openai_body["max_tokens"] = json!(max_tokens);
        }
    }

    (target_model, openai_body)
}

fn map_openai_sse_to_gemini(chunk: bytes::Bytes) -> bytes::Bytes {
    let Ok(s) = std::str::from_utf8(&chunk) else {
        return chunk;
    };
    let mut out = String::new();
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed == "data: [DONE]" {
            out.push_str("data: {\"response\":{\"candidates\":[{\"finishReason\":\"STOP\"}]}}\n\n");
            continue;
        }
        if let Some(data_str) = trimmed.strip_prefix("data:") {
            let data_str = data_str.trim();
            if let Ok(v) = serde_json::from_str::<Value>(data_str) {
                // If it already has the outer response wrapper, forward as-is
                if v.get("response").is_some() {
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                    continue;
                }
                // If it has candidates directly, wrap in response envelope
                if v.get("candidates").is_some() {
                    let wrapped = json!({ "response": v });
                    if let Ok(encoded) = serde_json::to_string(&wrapped) {
                        out.push_str(&format!("data: {encoded}\n\n"));
                        continue;
                    }
                }
                if let Some(choices) = v.get("choices").and_then(Value::as_array)
                    && let Some(choice) = choices.first()
                {
                    // 1. Thinking / reasoning content (DeepSeek, Qwen, etc.)
                    if let Some(thought) = choice
                        .get("delta")
                        .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                    {
                        let thought_chunk = json!({
                            "response": {
                                "candidates": [{
                                    "content": {
                                        "role": "model",
                                        "parts": [{
                                            "thought": true,
                                            "text": thought
                                        }]
                                    }
                                }]
                            }
                        });
                        if let Ok(encoded) = serde_json::to_string(&thought_chunk) {
                            out.push_str(&format!("data: {encoded}\n\n"));
                        }
                    }

                    // 2. Standard response text content
                    if let Some(content) = choice
                        .get("delta")
                        .and_then(|d| d.get("content"))
                        .and_then(Value::as_str)
                    {
                        let gemini_chunk = json!({
                            "response": {
                                "candidates": [{
                                    "content": {
                                        "role": "model",
                                        "parts": [{ "text": content }]
                                    }
                                }]
                            }
                        });
                        if let Ok(encoded) = serde_json::to_string(&gemini_chunk) {
                            out.push_str(&format!("data: {encoded}\n\n"));
                            continue;
                        }
                    }

                    // 3. Finish reason mapping
                    if let Some(finish) = choice.get("finish_reason").and_then(Value::as_str) {
                        let reason = match finish {
                            "stop" => "STOP",
                            "length" => "MAX_TOKENS",
                            _ => "STOP",
                        };
                        out.push_str(&format!(
                            "data: {{\"response\":{{\"candidates\":[{{\"finishReason\":\"{reason}\"}}]}}}}\n\n"
                        ));
                        continue;
                    }

                    // Done with this OpenAI choice delta chunk (e.g. role announcement or empty delta)
                    continue;
                }

                // Any other OpenAI top-level chunk (e.g. usage only) â€” do not echo raw OpenAI JSON
                if v.get("choices").is_some() || v.get("object").is_some() {
                    continue;
                }
            }
        }
        if !trimmed.is_empty() {
            out.push_str(trimmed);
            out.push_str("\n\n");
        }
    }
    if out.is_empty() {
        // If the chunk was an OpenAI event that produced no Gemini output, drop it rather than echoing raw OpenAI wire
        if s.contains("chat.completion") || s.contains("\"choices\":") || s.contains("\"object\":") {
            bytes::Bytes::new()
        } else {
            chunk
        }
    } else {
        bytes::Bytes::from(out)
    }
}

fn order_combo_models(models: &[String], strategy: &str) -> Vec<String> {
    let len = models.len();
    if len <= 1 {
        return models.to_vec();
    }
    let start = match strategy {
        "round-robin" => COMBO_RR_COUNTER.fetch_add(1, Ordering::Relaxed) % len,
        "random" => (Uuid::new_v4().as_u128() as usize) % len,
        _ => 0,
    };
    let mut ordered = Vec::with_capacity(len);
    ordered.extend_from_slice(&models[start..]);
    ordered.extend_from_slice(&models[..start]);
    ordered
}

async fn handle_gateway_request(
    state: GatewayState,
    headers: HeaderMap,
    body: Value,
    source_format: WireFormat,
) -> Response {
    handle_gateway_request_with_auth(state, headers, body, source_format, false).await
}

async fn handle_gateway_request_with_auth(
    state: GatewayState,
    headers: HeaderMap,
    mut body: Value,
    source_format: WireFormat,
    bypass_auth: bool,
) -> Response {
    let api_key_id = if bypass_auth {
        None
    } else {
        match authorize_client(&state, &headers).await {
            Ok(id) => id,
            Err(err) => return error_to_response(&err),
        }
    };

    let Some(raw_model) = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        return error_to_response(&TesRouteError::BadRequest(
            "missing required 'model' string field".into(),
        ));
    };

    let client_wants_stream = body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let header_pairs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_string(), val.to_string()))
        })
        .collect();

    let ts_cfg = state
        .store
        .token_saver()
        .get_config()
        .await
        .unwrap_or_default();
    let ts_report =
        apply_token_saver_to_request(&mut body, source_format, &header_pairs, &ts_cfg);

    let settings = state
        .store
        .settings()
        .get_settings()
        .await
        .unwrap_or_default();
    let is_cacheable = is_request_cacheable(&body, &headers, &settings);
    let cache_key = if is_cacheable {
        compute_cache_key(&raw_model, &body, source_format)
    } else {
        None
    };

    if let Some(ref key) = cache_key {
        let now = tesie_store::now_ms();
        let cached_opt = if let Some(mem_rec) = state.memory_cache.get(key, now) {
            let store_clone = state.store.clone();
            let key_str = key.clone();
            let tokens = mem_rec.total_tokens;
            tokio::spawn(async move {
                let _ = store_clone.response_cache().record_hit(&key_str, tokens).await;
            });
            Some(mem_rec)
        } else if let Ok(Some(db_rec)) = state.store.response_cache().get(key).await {
            state.memory_cache.put(db_rec.clone());
            Some(db_rec)
        } else {
            None
        };

        if let Some(cached) = cached_opt {
            if let Ok(payload) = serde_json::from_str::<Value>(&cached.response_payload) {
                if client_wants_stream {
                    let stream = replay_cached_as_sse_stream(source_format, &raw_model, &payload);
                    let mut resp = Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "text/event-stream; charset=utf-8")
                        .header(CACHE_CONTROL, "no-cache")
                        .header(HeaderName::from_static("x-tesie-cache"), HeaderValue::from_static("HIT"))
                        .header(HeaderName::from_static("x-tesie-proxy"), HeaderValue::from_static("CACHE"))
                        .body(Body::from_stream(stream))
                        .unwrap();
                    if let Ok(m_val) = HeaderValue::from_str(&cached.model) {
                        resp.headers_mut().insert(HeaderName::from_static("x-tesie-model"), m_val);
                    }
                    if let Ok(t_val) = HeaderValue::from_str(&cached.total_tokens.to_string()) {
                        resp.headers_mut().insert(HeaderName::from_static("x-tesie-tokens-saved"), t_val);
                    }
                    return resp;
                } else {
                    let mut resp = (StatusCode::OK, Json(payload)).into_response();
                    resp.headers_mut().insert(HeaderName::from_static("x-tesie-cache"), HeaderValue::from_static("HIT"));
                    resp.headers_mut().insert(HeaderName::from_static("x-tesie-proxy"), HeaderValue::from_static("CACHE"));
                    if let Ok(m_val) = HeaderValue::from_str(&cached.model) {
                        resp.headers_mut().insert(HeaderName::from_static("x-tesie-model"), m_val);
                    }
                    if let Ok(t_val) = HeaderValue::from_str(&cached.total_tokens.to_string()) {
                        resp.headers_mut().insert(HeaderName::from_static("x-tesie-tokens-saved"), t_val);
                    }
                    return resp;
                }
            }
        } else {
            let _ = state.store.response_cache().record_miss().await;
        }
    }

    // Check if the requested model matches a Combo (`<name>` or `combo/<name>`)
    let combo_lookup = raw_model.strip_prefix("combo/").unwrap_or(&raw_model);
    let maybe_combo = state
        .store
        .combos()
        .get_by_name(combo_lookup)
        .await
        .ok()
        .flatten();

    // Multimodal Bridge configuration
    let bridge_cfg = state
        .store
        .bridge()
        .get_config()
        .await
        .unwrap_or_default();

    let (provider_resp, resolved_combo_name) = if let Some(combo) = maybe_combo {
        if !combo.enabled {
            return error_to_response(&TesRouteError::BadRequest(format!(
                "combo '{}' is currently disabled",
                combo.name
            )));
        }
        let ordered = order_combo_models(&combo.models, &combo.strategy);
        if ordered.is_empty() {
            return error_to_response(&TesRouteError::BadRequest(format!(
                "combo '{}' has no models configured",
                combo.name
            )));
        }

        let mut last_err: Option<TesRouteError> = None;
        let mut success = None;
        for candidate_model in ordered {
            let mut candidate_body = body.clone();
            if let Some(obj) = candidate_body.as_object_mut() {
                obj.insert("model".into(), Value::String(candidate_model.clone()));
            }

            // Transcribe images/audio ONLY if this specific target candidate model lacks native capability
            let _ = bridge_request_if_needed(
                &state.engine,
                &mut candidate_body,
                source_format,
                &candidate_model,
                &header_pairs,
                &api_key_id,
                &bridge_cfg,
            )
            .await;

            match state
                .engine
                .execute_chat(ProviderRequest {
                    request_id: Uuid::new_v4().to_string(),
                    source_format,
                    raw_model: candidate_model,
                    body: candidate_body,
                    headers: header_pairs.clone(),
                    api_key_id: api_key_id.clone(),
                })
                .await
            {
                Ok(r) => {
                    success = Some(r);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        match success {
            Some(r) => (r, Some(combo.name)),
            None => {
                let err = last_err.unwrap_or_else(|| {
                    TesRouteError::Internal("all models in combo failed".into())
                });
                return error_to_response(&err);
            }
        }
    } else {
        // Direct model: transcribe images/audio ONLY if this target model lacks native capability
        let _ = bridge_request_if_needed(
            &state.engine,
            &mut body,
            source_format,
            &raw_model,
            &header_pairs,
            &api_key_id,
            &bridge_cfg,
        )
        .await;

        match state
            .engine
            .execute_chat(ProviderRequest {
                request_id: Uuid::new_v4().to_string(),
                source_format,
                raw_model,
                body,
                headers: header_pairs,
                api_key_id,
            })
            .await
        {
            Ok(r) => (r, None),
            Err(err) => return error_to_response(&err),
        }
    };

    let provider_id = provider_resp.provider_id.clone();
    let proxy_name = provider_resp.proxy_name.clone();
    let egress_ip = provider_resp.egress_ip.clone();
    let model = provider_resp.model.clone();

    if ts_report.saved_tokens > 0 || ts_report.style_applied {
        let _ = state
            .store
            .token_saver()
            .record_savings(&provider_id, &model, &ts_report, &ts_cfg.style_mode)
            .await;
    }

    if client_wants_stream {
        let stream_key = cache_key.clone();
        let stream_model = model.clone();
        let stream_format = source_format;
        let stream_ttl = settings.response_cache_ttl_sec;
        let stream_max = settings.response_cache_max_entries;
        let store_clone = state.store.clone();
        let mem_clone = state.memory_cache.clone();

        let mapped_stream: Body = if let Some(key_str) = stream_key {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<bytes::Bytes>();
            tokio::spawn(async move {
                let mut chunks = Vec::new();
                while let Some(b) = rx.recv().await {
                    chunks.push(b);
                }
                let byte_stream = futures::stream::iter(chunks.into_iter().map(Ok));
                if let Ok(payload) = drain_and_aggregate(stream_format, &stream_model, byte_stream).await {
                    let (prompt_tok, comp_tok, tot_tok) = extract_usage_tokens(&payload);
                    let now = tesie_store::now_ms();
                    let record = tesie_core::ResponseCacheRecord {
                        cache_key: key_str,
                        model: stream_model,
                        source_format: format!("{:?}", stream_format),
                        response_payload: payload.to_string(),
                        prompt_tokens: prompt_tok,
                        completion_tokens: comp_tok,
                        total_tokens: tot_tok,
                        created_at: now,
                        expires_at: now + (stream_ttl * 1000),
                        hit_count: 0,
                        last_hit_at: None,
                    };
                    mem_clone.put(record.clone());
                    let _ = store_clone.response_cache().set(&record, stream_max).await;
                }
            });

            let stream_with_tap = provider_resp.stream.map(move |item| {
                match item {
                    Ok(b) => {
                        let _ = tx.send(b.clone());
                        Ok(b)
                    }
                    Err(e) => Err(std::io::Error::other(e.to_string())),
                }
            });
            Body::from_stream(stream_with_tap)
        } else {
            let mapped = provider_resp
                .stream
                .map(|item| item.map_err(|e| std::io::Error::other(e.to_string())));
            Body::from_stream(mapped)
        };

        let mut resp = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(CACHE_CONTROL, "no-cache")
            .header(HeaderName::from_static("x-tesie-cache"), HeaderValue::from_static("MISS"))
            .body(mapped_stream)
            .unwrap();
        attach_proxy_headers(
            resp.headers_mut(),
            &proxy_name,
            egress_ip.as_deref(),
            &model,
            resolved_combo_name.as_deref(),
            ts_report.saved_tokens,
        );
        return resp;
    }

    match drain_and_aggregate(source_format, &model, provider_resp.stream).await {
        Ok(payload) => {
            if let Some(key_str) = cache_key {
                let (prompt_tok, comp_tok, tot_tok) = extract_usage_tokens(&payload);
                let now = tesie_store::now_ms();
                let record = tesie_core::ResponseCacheRecord {
                    cache_key: key_str,
                    model: model.clone(),
                    source_format: format!("{:?}", source_format),
                    response_payload: payload.to_string(),
                    prompt_tokens: prompt_tok,
                    completion_tokens: comp_tok,
                    total_tokens: tot_tok,
                    created_at: now,
                    expires_at: now + (settings.response_cache_ttl_sec * 1000),
                    hit_count: 0,
                    last_hit_at: None,
                };
                state.memory_cache.put(record.clone());
                let store_clone = state.store.clone();
                let max_entries = settings.response_cache_max_entries;
                tokio::spawn(async move {
                    let _ = store_clone.response_cache().set(&record, max_entries).await;
                });
            }

            let mut resp = (StatusCode::OK, Json(payload)).into_response();
            resp.headers_mut().insert(HeaderName::from_static("x-tesie-cache"), HeaderValue::from_static("MISS"));
            attach_proxy_headers(
                resp.headers_mut(),
                &proxy_name,
                egress_ip.as_deref(),
                &model,
                resolved_combo_name.as_deref(),
                ts_report.saved_tokens,
            );
            resp
        }
        Err(err) => error_to_response(&err),
    }
}

fn attach_proxy_headers(
    headers: &mut HeaderMap,
    proxy_name: &str,
    egress_ip: Option<&str>,
    resolved_model: &str,
    combo_name: Option<&str>,
    tokens_saved: usize,
) {
    if let Ok(val) = HeaderValue::from_str(proxy_name) {
        headers.insert(HeaderName::from_static("x-tesie-proxy"), val.clone());
        headers.insert(HeaderName::from_static("x-tesroute-proxy"), val);
    }
    if let Some(ip) = egress_ip
        && !ip.is_empty()
        && let Ok(val) = HeaderValue::from_str(ip)
    {
        headers.insert(HeaderName::from_static("x-tesie-egress-ip"), val.clone());
        headers.insert(HeaderName::from_static("x-tesroute-egress-ip"), val);
    }
    if let Ok(val) = HeaderValue::from_str(resolved_model) {
        headers.insert(HeaderName::from_static("x-tesie-model"), val.clone());
        headers.insert(HeaderName::from_static("x-tesroute-model"), val);
    }
    if let Some(combo) = combo_name
        && let Ok(val) = HeaderValue::from_str(combo)
    {
        headers.insert(HeaderName::from_static("x-tesie-combo"), val.clone());
        headers.insert(HeaderName::from_static("x-tesroute-combo"), val);
    }
    if tokens_saved > 0
        && let Ok(val) = HeaderValue::from_str(&tokens_saved.to_string())
    {
        headers.insert(HeaderName::from_static("x-tesie-tokens-saved"), val);
    }
}

fn error_to_response(err: &TesRouteError) -> Response {
    let (status, retry_after, body) = sse_error_payload(err);
    let mut resp = (status, Json(body)).into_response();
    if let Some(secs) = retry_after
        && let Ok(val) = HeaderValue::from_str(&secs.to_string())
    {
        resp.headers_mut().insert(RETRY_AFTER, val);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mitm(default_target_model: Option<&str>) -> tesie_core::AntigravityMitmConfig {
        tesie_core::AntigravityMitmConfig {
            enabled: true,
            default_target_model: default_target_model.map(str::to_string),
            model_aliases: Default::default(),
        }
    }

    fn resolved(
        model: &str,
        cfg: Option<&tesie_core::AntigravityMitmConfig>,
        combos: Option<&[tesie_core::ComboRecord]>,
    ) -> String {
        let body = json!({
            "model": model,
            "contents": [{ "role": "user", "parts": [{ "text": "hi" }] }]
        });
        normalize_antigravity_request_to_openai(&body, true, cfg, combos).0
    }

    #[test]
    fn default_target_model_does_not_hijack_explicitly_routed_ids() {
        let cfg = mitm(Some("antigravity/gemini-2.5-flash"));
        for id in [
            "combo/GoldFish",
            "ovh/DeepSeek-V4.1-Flash-1017",
            "oc/some-model",
            "antigravity/gemini-2.5-flash",
        ] {
            assert_eq!(
                resolved(id, Some(&cfg), None),
                id,
                "explicit id {id} was replaced by default_target_model"
            );
        }
    }

    #[test]
    fn default_target_model_applies_to_unprefixed_ids() {
        let cfg = mitm(Some("antigravity/gemini-2.5-flash"));
        assert_eq!(
            resolved("gemini-4-carbon", Some(&cfg), None),
            "antigravity/gemini-2.5-flash"
        );
    }

    #[test]
    fn unprefixed_ids_fall_back_to_antigravity_without_a_default() {
        assert_eq!(
            resolved("gemini-4-carbon", None, None),
            "antigravity/gemini-4-carbon"
        );
    }

    #[test]
    fn explicit_alias_beats_the_default() {
        let mut cfg = mitm(Some("antigravity/gemini-2.5-flash"));
        cfg.model_aliases
            .insert("custom-code-model".into(), "combo/GoldFish".into());
        assert_eq!(
            resolved("custom-code-model", Some(&cfg), None),
            "combo/GoldFish"
        );
    }
}
