use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tesie::{build_app_router, AppOptions};
use tesie_providers::{ModelCatalogCache, OpenCodeConfig, OpenCodeEngine};
use tesie_proxy::{IpPoolSelector, MihomoConfig, MihomoController};
use tesie_store::Store;
use tokio::net::TcpListener;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct MockOpenAiUpstream {
    seen_auths: Arc<Mutex<Vec<String>>>,
}

async fn mock_models() -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": [
            { "id": "gpt-4o-mini", "object": "model" },
            { "id": "deepseek-chat", "object": "model" }
        ]
    }))
}

async fn mock_chat_completions(
    State(st): State<MockOpenAiUpstream>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    st.seen_auths.lock().unwrap().push(auth);

    let model = body["model"].as_str().unwrap_or("gpt-4o-mini");
    let chunk1 = json!({
        "id": "chatcmpl-custom-1",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": "Hello from " },
            "finish_reason": Value::Null
        }]
    });
    let chunk2 = json!({
        "id": "chatcmpl-custom-1",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "content": "custom upstream!" },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 12,
            "completion_tokens": 5,
            "total_tokens": 17
        }
    });

    let sse = format!("data: {chunk1}\n\ndata: {chunk2}\n\ndata: [DONE]\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}

async fn spawn_mock_custom_upstream() -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
    let seen_auths = Arc::new(Mutex::new(Vec::new()));
    let state = MockOpenAiUpstream {
        seen_auths: Arc::clone(&seen_auths),
    };
    let app = Router::new()
        .route("/v1/models", get(mock_models))
        .route("/v1/chat/completions", post(mock_chat_completions))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, seen_auths)
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

#[tokio::test]
async fn e2e_custom_provider_streaming_round_robin_backup_and_security() {
    let exe_name = if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    };
    let mihomo_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.data/bin")
        .join(exe_name);
    assert!(mihomo_bin.is_file(), "expected tesie-core binary at {}", mihomo_bin.display());

    let (upstream_addr, seen_auths) = spawn_mock_custom_upstream().await;
    let upstream_base = format!("http://{upstream_addr}/v1");

    let work_dir = std::env::temp_dir().join(format!(
        "tesie-e2e-custom-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let store = Store::connect_in_memory().await.unwrap();
    let ctl_port = free_port().await;
    let mihomo = MihomoController::start_managed(
        MihomoConfig {
            bin_path: mihomo_bin,
            work_dir: work_dir.clone(),
            ctl_addr: format!("127.0.0.1:{ctl_port}"),
            ctl_secret: "e2e-custom-secret".into(),
            allow_loopback_targets: true,
            insecure_skip_verify: false,
        },
        &store,
    )
    .await
    .unwrap();

    let selector = Arc::new(IpPoolSelector::new(store.clone()));
    let engine = Arc::new(OpenCodeEngine::new(
        store.clone(),
        mihomo.clone(),
        selector,
        OpenCodeConfig {
            free_base_url: upstream_base.clone(),
            zen_base_url: upstream_base.clone(),
            timeout_ms: 8_000,
        },
    ));
    let catalog = Arc::new(ModelCatalogCache::new());
    let app = build_app_router(
        store.clone(),
        mihomo.clone(),
        engine,
        catalog,
        AppOptions {
            admin_token: None,
            require_api_key: false,
            fetch_live_models: false,
            frontend_dir: None,
        },
    );

    let gw_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(gw_listener, app).await;
    });

    let base = format!("http://{gw_addr}");
    let client = reqwest::Client::new();

    // 1. Create a Custom OpenAI-compatible provider with 2 API keys
    let create_res = client
        .post(format!("{base}/api/admin/custom-providers"))
        .json(&json!({
            "name": "Logfare Test",
            "prefix": "lf",
            "protocol": "openai",
            "api_type": "chat",
            "base_url": upstream_base,
            "api_keys": ["sk-lf-key-1", "sk-lf-key-2"],
            "models": ["gpt-4o-mini"],
            "favicon_url": "data:image/png;base64,iVBORw0KGgo=",
            "enabled": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_res.status(), 201);
    let created_json: Value = create_res.json().await.unwrap();
    let cp_id = created_json["custom_provider"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(cp_id, "cp-lf");

    // 2. Verify /models discovery & save + provider connectivity test
    let fetch_res = client
        .post(format!("{base}/api/admin/custom-providers/{cp_id}/fetch-models"))
        .json(&json!({ "save": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(fetch_res.status(), 200);
    let fetch_json: Value = fetch_res.json().await.unwrap();
    let discovered = fetch_json["models"].as_array().unwrap();
    assert!(discovered.iter().any(|m| m.as_str() == Some("deepseek-chat")));

    let test_res: Value = client
        .post(format!("{base}/api/admin/providers/{cp_id}/test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(test_res["ok"].as_bool(), Some(true));

    // 3. Configure Round-Robin strategy on custom provider
    let cfg_res = client
        .put(format!("{base}/api/admin/providers/{cp_id}/config"))
        .json(&json!({
            "proxy_mode": "direct",
            "routing_strategy": "round-robin"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cfg_res.status(), 200);
    let cfg_json: Value = cfg_res.json().await.unwrap();
    assert_eq!(
        cfg_json["config"]["routing_strategy"].as_str(),
        Some("round-robin")
    );

    // 4. Verify custom model appears in /v1/models and routes streaming chat across both keys
    let models_json: Value = client
        .get(format!("{base}/v1/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(models_json["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"].as_str() == Some("lf/gpt-4o-mini")));

    for _ in 0..2 {
        let chat_res = client
            .post(format!("{base}/v1/chat/completions"))
            .json(&json!({
                "model": "lf/gpt-4o-mini",
                "stream": true,
                "messages": [{ "role": "user", "content": "hi" }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(chat_res.status(), 200);
        let sse_text = chat_res.text().await.unwrap();
        assert!(sse_text.contains("Hello from "), "unexpected SSE body: {sse_text}");
        assert!(sse_text.contains("custom upstream!"), "unexpected SSE body: {sse_text}");
    }

    {
        let auths = seen_auths.lock().unwrap();
        assert_eq!(auths.len(), 2);
        assert!(auths.iter().any(|a| a == "Bearer sk-lf-key-1"));
        assert!(auths.iter().any(|a| a == "Bearer sk-lf-key-2"));
    }

    // 5. Export backup, delete custom provider, import backup with replace=true & verify restoration
    let backup_doc: Value = client
        .get(format!("{base}/api/admin/backup/export?include_secrets=true"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(backup_doc["app"].as_str(), Some("Tesie"));

    let del_res = client
        .delete(format!("{base}/api/admin/custom-providers/{cp_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del_res.status(), 204);

    let list_after_del: Value = client
        .get(format!("{base}/api/admin/custom-providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_after_del["custom_providers"]
        .as_array()
        .unwrap()
        .is_empty());

    let import_res = client
        .post(format!("{base}/api/admin/backup/import?replace=true"))
        .json(&json!({ "backup": backup_doc, "replace": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(import_res.status(), 200);
    let import_json: Value = import_res.json().await.unwrap();
    assert_eq!(import_json["summary"]["custom_providers"].as_u64(), Some(1));

    let list_after_restore: Value = client
        .get(format!("{base}/api/admin/custom-providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        list_after_restore["custom_providers"][0]["prefix"].as_str(),
        Some("lf")
    );

    // 6. Test Optional Dashboard Password + Maintenance cooldown reset
    let set_pw = client
        .put(format!("{base}/api/admin/settings"))
        .json(&json!({ "admin_password": "my-secret-pass" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set_pw.status(), 200);

    let unauth_providers = client
        .get(format!("{base}/api/admin/providers"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_providers.status(), 401);

    let auth_status: Value = client
        .get(format!("{base}/api/admin/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(auth_status["protected"].as_bool(), Some(true));
    assert_eq!(auth_status["authenticated"].as_bool(), Some(false));

    let reset_res = client
        .post(format!("{base}/api/admin/maintenance/reset-cooldowns"))
        .header("X-Admin-Token", "my-secret-pass")
        .send()
        .await
        .unwrap();
    assert_eq!(reset_res.status(), 200);

    let clear_pw = client
        .put(format!("{base}/api/admin/settings"))
        .header("X-Admin-Token", "my-secret-pass")
        .json(&json!({ "clear_admin_password": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(clear_pw.status(), 200);

    let open_again = client
        .get(format!("{base}/api/admin/providers"))
        .send()
        .await
        .unwrap();
    assert_eq!(open_again.status(), 200);

    mihomo.shutdown();
    let _ = std::fs::remove_dir_all(work_dir);
}
