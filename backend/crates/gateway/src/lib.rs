pub mod aggregator;
pub mod bridge;

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
    mut body: Value,
    source_format: WireFormat,
) -> Response {
    let api_key_id = match authorize_client(&state, &headers).await {
        Ok(id) => id,
        Err(err) => return error_to_response(&err),
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

    // Multimodal Bridge: Transcribe images/audio if target model lacks native capability
    let bridge_cfg = state
        .store
        .bridge()
        .get_config()
        .await
        .unwrap_or_default();
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

    // Check if the requested model matches a Combo (`<name>` or `combo/<name>`)
    let combo_lookup = raw_model.strip_prefix("combo/").unwrap_or(&raw_model);
    let maybe_combo = state
        .store
        .combos()
        .get_by_name(combo_lookup)
        .await
        .ok()
        .flatten();

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
        let mapped_stream = provider_resp
            .stream
            .map(|item| item.map_err(|e| std::io::Error::other(e.to_string())));
        let mut resp = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(CACHE_CONTROL, "no-cache")
            .body(Body::from_stream(mapped_stream))
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
            let mut resp = (StatusCode::OK, Json(payload)).into_response();
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
