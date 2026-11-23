#[cfg(windows)]
pub mod tray;

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::Path as AxumPath;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::json;
use tesie_admin::{admin_router, AdminState, AuthAttemptLimiter};
use tesie_gateway::{gateway_router, GatewayState};
use tesie_providers::{ModelCatalogCache, OpenCodeEngine};
use tesie_proxy::MihomoController;
use tesie_store::Store;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub struct AppOptions {
    pub admin_token: Option<String>,
    pub require_api_key: bool,
    pub fetch_live_models: bool,
    pub frontend_dir: Option<PathBuf>,
    /// Reject admin requests outright while no credential is configured, instead of
    /// serving them unauthenticated. The container image enables this.
    pub require_admin_auth: bool,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            admin_token: None,
            require_api_key: false,
            fetch_live_models: true,
            frontend_dir: None,
            require_admin_auth: false,
        }
    }
}

pub fn build_app_router(
    store: Store,
    mihomo: Arc<MihomoController>,
    engine: Arc<OpenCodeEngine>,
    catalog: Arc<ModelCatalogCache>,
    opts: AppOptions,
) -> Router {
    let gw_state = GatewayState {
        store: store.clone(),
        engine: engine.clone(),
        mihomo: mihomo.clone(),
        catalog: catalog.clone(),
        require_api_key: opts.require_api_key,
        fetch_live_models: opts.fetch_live_models,
    };
    let adm_state = AdminState {
        store: store.clone(),
        mihomo,
        engine: engine.clone(),
        catalog,
        zen_base_url: engine.config().zen_base_url.clone(),
        fetch_live_models: opts.fetch_live_models,
        admin_token: opts.admin_token,
        require_admin_auth: opts.require_admin_auth,
        auth_limiter: Arc::new(AuthAttemptLimiter::default()),
    };

    let icon_store = store;
    let mut router = Router::new()
        .route("/healthz", get(|| async { Json(json!({ "status": "ok" })) }))
        .route(
            "/api/custom-providers/{id}/icon",
            get(move |AxumPath(id): AxumPath<String>| {
                let st = icon_store.clone();
                async move { serve_custom_provider_icon(st, id).await }
            }),
        )
        .merge(gateway_router(gw_state))
        .nest("/api/admin", admin_router(adm_state));

    if let Some(dir) = opts.frontend_dir {
        let root = Arc::new(dir);
        router = router.fallback(move |req: Request<Body>| {
            let root = root.clone();
            async move { serve_spa_asset(&root, req.method(), req.uri()).await }
        });
    }

    router
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

/// Coding agents send multi-megabyte prompts; axum's 2 MB default rejected them here
/// before the upstream ever saw them.
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;

async fn serve_spa_asset(root: &Path, method: &Method, uri: &Uri) -> Response {
    // API-shaped paths must never reach the static file server. Check the path before the
    // method so unmatched endpoints answer 404 instead of 405 (a 405 with an empty body is
    // indistinguishable from a broken upstream to clients like Antigravity).
    let raw_path = uri.path();
    if raw_path.starts_with("/api/")
        || raw_path == "/api"
        || raw_path.starts_with("/v1/")
        || raw_path == "/v1"
        || raw_path.starts_with("/v1internal")
    {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "endpoint not found" })),
        )
            .into_response();
    }

    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    let trimmed = raw_path.trim_start_matches('/');
    let rel_path = Path::new(trimmed);
    for comp in rel_path.components() {
        if !matches!(comp, Component::Normal(_)) {
            return StatusCode::BAD_REQUEST.into_response();
        }
    }

    let candidate = if trimmed.is_empty() {
        root.join("index.html")
    } else {
        let joined = root.join(rel_path);
        if joined.is_file() {
            joined
        } else if rel_path.extension().is_none() {
            // Client-side SPA route fallback to index.html
            root.join("index.html")
        } else {
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    match tokio::fs::read(&candidate).await {
        Ok(bytes) => {
            let mime = content_type_for_path(&candidate);
            let cache_ctl = if raw_path.starts_with("/assets/") {
                "public, max-age=31536000, immutable"
            } else if raw_path.starts_with("/providers/") || raw_path == "/favicon.svg" {
                "public, max-age=3600"
            } else {
                "no-cache"
            };
            let is_head = method == Method::HEAD;
            let body = if is_head {
                Body::empty()
            } else {
                Body::from(bytes)
            };
            let mut resp = Response::new(body);
            *resp.status_mut() = StatusCode::OK;
            resp.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(mime));
            resp.headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static(cache_ctl));
            resp
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_custom_provider_icon(store: Store, id: String) -> Response {
    let Ok(Some(cp)) = store.custom_providers().get(&id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(raw) = cp.favicon_url.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(rest) = raw.strip_prefix("data:") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((meta, b64)) = rest.split_once(";base64,") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = meta.split(';').next().unwrap_or("").trim();
    let content_type = if mime.is_empty() {
        "image/png"
    } else if mime.starts_with("image/") {
        mime
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(bytes) = STANDARD.decode(b64.trim()) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut resp = Response::new(Body::from(bytes));
    *resp.status_mut() = StatusCode::OK;
    if let Ok(hv) = HeaderValue::from_str(content_type) {
        resp.headers_mut().insert(CONTENT_TYPE, hv);
    }
    resp.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400, immutable"),
    );
    resp
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
