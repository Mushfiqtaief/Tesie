use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

static COUNTER: AtomicU64 = AtomicU64::new(100);

/// Spawns a killable CONNECT proxy that tags upstream TCP connections by forwarding to a dedicated
/// per-node mock listener, and immediately closes active/idle TCP tunnels when `alive_rx` flips false!
async fn spawn_killable_connect_proxy(
    target_addr: SocketAddr,
    alive_rx: tokio::sync::watch::Receiver<bool>,
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            if !*alive_rx.borrow() {
                drop(client);
                continue;
            }
            let mut conn_rx = alive_rx.clone();
            tokio::spawn(async move {
                let mut buf = Vec::with_capacity(1024);
                let mut tmp = [0u8; 512];
                loop {
                    let Ok(n) = client.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    if buf.len() > 16_384 {
                        return;
                    }
                }

                let header_end = buf
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                    .unwrap_or(buf.len());
                let head = String::from_utf8_lossy(&buf[..header_end]);
                if !head.lines().next().unwrap_or("").starts_with("CONNECT ") {
                    let _ = client
                        .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                        .await;
                    return;
                }

                let Ok(mut upstream) = TcpStream::connect(target_addr).await else {
                    let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
                    return;
                };

                if client
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                if header_end < buf.len()
                    && upstream.write_all(&buf[header_end..]).await.is_err()
                {
                    return;
                }
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {}
                    _ = async {
                        while conn_rx.changed().await.is_ok() {
                            if !*conn_rx.borrow() {
                                break;
                            }
                        }
                    } => {}
                }
            });
        }
    });

    addr
}

#[derive(Clone)]
struct MockAgNodeState {
    node_tag: String,
    exit_ip: String,
    exhaust_account_1: Arc<AtomicBool>,
    recorded_chat_hits: Arc<Mutex<Vec<(String, String)>>>,
}

async fn spawn_mock_antigravity_upstream(
    node_tag: &str,
    exit_ip: &str,
    exhaust_account_1: Arc<AtomicBool>,
    recorded_chat_hits: Arc<Mutex<Vec<(String, String)>>>,
) -> SocketAddr {
    let st = MockAgNodeState {
        node_tag: node_tag.to_string(),
        exit_ip: exit_ip.to_string(),
        exhaust_account_1,
        recorded_chat_hits,
    };

    let app = Router::new()
        .route("/trace", get(mock_trace))
        .route("/token", post(mock_google_token))
        .route("/v1internal:loadCodeAssist", post(mock_load_code_assist))
        .route(
            "/v1internal:fetchAvailableModels",
            post(mock_fetch_available_models),
        )
        .route(
            "/v1internal:retrieveUserQuotaSummary",
            post(mock_retrieve_user_quota_summary),
        )
        .route(
            "/v1internal:streamGenerateContent",
            post(mock_stream_generate_content),
        )
        .with_state(st);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

async fn mock_trace(State(st): State<MockAgNodeState>) -> String {
    format!("fl=test\nip={}\nts=1760000000\n", st.exit_ip)
}

async fn mock_google_token(body: String) -> Json<Value> {
    let is_acct2 = body.contains("rt_acct_2");
    let access_token = if is_acct2 {
        "ya29.access_acct_2"
    } else {
        "ya29.access_acct_1"
    };
    Json(json!({
        "access_token": access_token,
        "expires_in": 3600,
        "token_type": "Bearer",
        "scope": "https://www.googleapis.com/auth/cloud-platform"
    }))
}

async fn mock_load_code_assist(headers: HeaderMap) -> Json<Value> {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ua.starts_with("antigravity/cli/"),
        "expected antigravity CLI UA on discovery, got {ua}"
    );
    Json(json!({
        "cloudaicompanionProject": "proj-e2e-antigravity"
    }))
}

async fn mock_fetch_available_models(headers: HeaderMap) -> Json<Value> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let is_acct2 = auth.contains("access_acct_2");

    let mut models = serde_json::Map::new();
    models.insert(
        "gemini-4-carbon-medium".into(),
        json!({
            "displayName": "Gemini 4 Carbon Medium",
            "quotaInfo": {
                "remainingFraction": 0.99,
                "resetTime": "2026-12-01T00:00:00Z"
            }
        }),
    );
    models.insert(
        "claude-sonnet-4-6".into(),
        json!({
            "displayName": "Claude Sonnet 4.6 (Thinking)",
            "quotaInfo": {
                "remainingFraction": 1.0,
                "resetTime": "2026-12-01T00:00:00Z"
            }
        }),
    );
    // Simulate `gemini-4-argon-medium` NOT unlocked on Account 1, but UNLOCKED on Account 2!
    if is_acct2 {
        models.insert(
            "gemini-4-argon-medium".into(),
            json!({
                "displayName": "Gemini 4 Argon Medium",
                "quotaInfo": {
                    "remainingFraction": 0.95,
                    "resetTime": "2026-12-01T00:00:00Z"
                }
            }),
        );
    }

    Json(json!({ "models": models }))
}

async fn mock_retrieve_user_quota_summary() -> Json<Value> {
    Json(json!({
        "groups": [
            {
                "displayName": "Gemini Models",
                "buckets": [
                    { "window": "weekly", "remainingFraction": 1.0, "resetTime": "2026-12-07T00:00:00Z" },
                    { "window": "5h", "remainingFraction": 0.99, "resetTime": "2026-12-01T05:00:00Z" }
                ]
            },
            {
                "displayName": "Claude and GPT models",
                "buckets": [
                    { "window": "weekly", "remainingFraction": 1.0, "resetTime": "2026-12-07T00:00:00Z" },
                    { "window": "5h", "remainingFraction": 1.0, "resetTime": "2026-12-01T05:00:00Z" }
                ]
            }
        ]
    }))
}

async fn mock_stream_generate_content(
    State(st): State<MockAgNodeState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Verify outer envelope matches Antigravity Cloud Code v1internal format (no requestType: "agent")
    assert!(
        body.get("requestType").is_none(),
        "outer wrapper must omit requestType to avoid agent bucket 429"
    );
    assert_eq!(body["model"].as_str(), Some("gemini-4-carbon-medium"));

    // Verify `_ide` tool cloaking + decoy tools injection
    let decls = body["request"]["tools"][0]["functionDeclarations"]
        .as_array()
        .expect("expected functionDeclarations");
    assert!(
        decls.iter().any(|d| d["name"] == "write_file_ide"),
        "expected user tool write_file to be cloaked as write_file_ide"
    );
    assert!(
        decls.len() >= 10,
        "expected Antigravity decoy tools alongside user tool"
    );

    st.recorded_chat_hits
        .lock()
        .unwrap()
        .push((auth.clone(), st.node_tag.clone()));

    if auth.contains("access_acct_1") && st.exhaust_account_1.load(Ordering::SeqCst) {
        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "error": {
                        "code": 429,
                        "status": "RESOURCE_EXHAUSTED",
                        "message": "Quota exhausted for gemini-4-carbon on account 1"
                    }
                })
                .to_string(),
            ))
            .unwrap();
    }

    let chunk1 = json!({
        "response": {
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [
                        { "thought": true, "text": "Planning write..." },
                        {
                            "functionCall": {
                                "name": "write_file_ide",
                                "args": { "path": "hello.txt", "content": "hi" }
                            }
                        }
                    ]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 42,
                "candidatesTokenCount": 14,
                "totalTokenCount": 56
            }
        }
    });

    let sse = format!("data: {chunk1}\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

#[tokio::test]
async fn e2e_antigravity_smart_proxy_partition_sticky_and_fallback() {
    let exe_name = if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    };
    let mihomo_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.data/bin")
        .join(exe_name);
    assert!(mihomo_bin.is_file());

    let exhaust_acct_1 = Arc::new(AtomicBool::new(false));
    let chat_hits: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));

    // Spawn 6 distinct per-node mock servers + 6 killable proxies so we can prove exact proxy routing
    let mut proxy_addrs = Vec::new();
    let mut alive_flags = Vec::new();
    let mut first_upstream_addr = None;

    for idx in 1..=6 {
        let node_tag = format!("NODE - {idx}");
        let exit_ip = format!("203.0.113.{idx}");
        let up_addr = spawn_mock_antigravity_upstream(
            &node_tag,
            &exit_ip,
            exhaust_acct_1.clone(),
            chat_hits.clone(),
        )
        .await;
        if first_upstream_addr.is_none() {
            first_upstream_addr = Some(up_addr);
        }
        let (alive_tx, alive_rx) = tokio::sync::watch::channel(true);
        let p_addr = spawn_killable_connect_proxy(up_addr, alive_rx).await;
        proxy_addrs.push(p_addr);
        alive_flags.push(alive_tx);
    }

    let canonical_up = format!("http://{}", first_upstream_addr.unwrap());
    // Point Antigravity OAuth, discovery, and chat URLs at the canonical mock address (each CONNECT
    // proxy forwards to its own per-node mock listener!)
    // SAFETY: single integration test binary (`e2e_antigravity_smart_proxy`)
    unsafe {
        std::env::set_var("ANTIGRAVITY_OAUTH_TOKEN_URL", format!("{canonical_up}/token"));
        std::env::set_var("ANTIGRAVITY_DISCOVERY_BASE_URL", &canonical_up);
        std::env::set_var("ANTIGRAVITY_CHAT_BASE_URL", &canonical_up);
    }

    let work_dir = std::env::temp_dir().join(format!(
        "tesroute-e2e-ag-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let store = Store::connect_in_memory().await.unwrap();
    let ctl_port = free_port().await;
    let mihomo = MihomoController::start_managed(
        MihomoConfig {
            bin_path: mihomo_bin,
            work_dir,
            ctl_addr: format!("127.0.0.1:{ctl_port}"),
            ctl_secret: "e2e-ag-secret".into(),
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
            free_base_url: canonical_up.clone(),
            zen_base_url: canonical_up.clone(),
            timeout_ms: 10_000,
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

    let app_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_addr = app_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(app_listener, app).await;
    });

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://{app_addr}");

    // 1. Batch import 8 proxies (6 real + 2 temp, including SOCKS4!) and verify sequential `NODE - 1 .. NODE - 8` naming
    let temp_p7 = free_port().await;
    let temp_p8 = free_port().await;
    let mut import_lines: Vec<String> = proxy_addrs
        .iter()
        .map(|a| format!("http://{a}"))
        .collect();
    import_lines.push(format!("socks4://127.0.0.1:{temp_p7}"));
    import_lines.push(format!("http://127.0.0.1:{temp_p8}"));

    let imp_res: Value = client
        .post(format!("{base}/api/admin/proxies/import"))
        .json(&json!({ "urls_text": import_lines.join("\n") }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(imp_res["imported"].as_u64(), Some(8));

    let px_list: Value = client
        .get(format!("{base}/api/admin/proxies"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let external: Vec<&Value> = px_list["proxies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["is_direct"] == false)
        .collect();
    assert_eq!(external.len(), 8);
    assert_eq!(external[0]["name"].as_str(), Some("NODE - 1"));
    assert_eq!(external[5]["name"].as_str(), Some("NODE - 6"));
    assert_eq!(external[7]["name"].as_str(), Some("NODE - 8"));

    // 2. Test Proxy Edit (`PATCH /api/admin/proxies/{id}`) and Batch Delete (`NODE - 7`, `NODE - 8`)
    let node7_id = external[6]["id"].as_str().unwrap().to_string();
    let node8_id = external[7]["id"].as_str().unwrap().to_string();

    let edit_res = client
        .patch(format!("{base}/api/admin/proxies/{node7_id}"))
        .json(&json!({
            "name": "NODE - 7",
            "proxy_type": "http",
            "host": "127.0.0.1",
            "port": proxy_addrs[0].port(),
            "username": "alice",
            "password": "secret"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(edit_res.status(), 200);
    let edited_json: Value = edit_res.json().await.unwrap();
    assert_eq!(edited_json["proxy"]["username"].as_str(), Some("alice"));

    let del_res: Value = client
        .post(format!("{base}/api/admin/proxies/batch-delete"))
        .json(&json!({ "ids": [node7_id, node8_id] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del_res["deleted"].as_u64(), Some(2));

    // Probe all 6 proxies so Mihomo marks them alive & records exit IPs
    let probe_res = client
        .post(format!("{base}/api/admin/proxies/probe"))
        .query(&[("url", format!("{canonical_up}/trace"))])
        .send()
        .await
        .unwrap();
    assert_eq!(probe_res.status(), 200);

    // 3. Import 2 Antigravity accounts and verify disjoint 3 / 3 Smart Proxy Partitioning
    for (name, email, rt) in [
        ("Acct One", "acct1@example.com", "rt_acct_1"),
        ("Acct Two", "acct2@example.com", "rt_acct_2"),
    ] {
        let r = client
            .post(format!("{base}/api/admin/antigravity/accounts/import"))
            .json(&json!({
                "name": name,
                "email": email,
                "refresh_token": rt
            }))
            .send()
            .await
            .unwrap();
        assert!(
            r.status().is_success(),
            "expected 2xx on import, got {}",
            r.status()
        );
    }

    let accts_json: Value = client
        .get(format!("{base}/api/admin/antigravity/accounts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let accts = accts_json["accounts"].as_array().unwrap();
    assert_eq!(accts.len(), 2);

    let slice1: Vec<&str> = accts[0]["partition"]["assigned_proxies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    let slice2: Vec<&str> = accts[1]["partition"]["assigned_proxies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(slice1, vec!["NODE - 1", "NODE - 2", "NODE - 3"]);
    assert_eq!(slice2, vec!["NODE - 4", "NODE - 5", "NODE - 6"]);
    assert_eq!(
        accts[0]["partition"]["primary_proxy"]["name"].as_str(),
        Some("NODE - 1")
    );
    // Verify dual-pool `retrieveUserQuotaSummary` + unlocked/locked model detection:
    // `gemini-4-argon-medium` is LOCKED (`unlocked == false`) on Account 1, and UNLOCKED (`unlocked == true`) on Account 2!
    let acct1_pools = accts[0]["quota"]["pools"].as_array().unwrap();
    assert_eq!(acct1_pools.len(), 4);
    assert_eq!(acct1_pools[0]["group_name"].as_str(), Some("Gemini Models"));
    assert_eq!(acct1_pools[0]["window_label"].as_str(), Some("Weekly limit"));
    assert_eq!(acct1_pools[1]["window_label"].as_str(), Some("5h limit"));
    assert_eq!(acct1_pools[2]["group_name"].as_str(), Some("Claude and GPT models"));

    //Account 1 has Argon LOCKED -> excluded from available-only `quota.models`!
    assert!(
        accts[0]["quota"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["model"] != "gemini-4-argon" && m["model"] != "gemini-4-argon-medium"),
        "locked Argon must not appear in available-only quota models"
    );

    // Account 2 has Argon UNLOCKED -> variants merge into a single canonical `gemini-4-argon` entry!
    let acct2_argon_entries: Vec<&Value> = accts[1]["quota"]["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["model"].as_str().unwrap_or("").starts_with("gemini-4-argon"))
        .collect();
    assert_eq!(acct2_argon_entries.len(), 1, "variants must merge into one entry");
    assert_eq!(acct2_argon_entries[0]["model"].as_str(), Some("gemini-4-argon"));
    assert_eq!(acct2_argon_entries[0]["display_name"].as_str(), Some("Gemini 4 Argon"));
    assert_eq!(acct2_argon_entries[0]["unlocked"].as_bool(), Some(true));

    // 4. Two consecutive Antigravity chat completions must stick to Account 1's `NODE - 1` (NO round-robin!)
    //    and uncloak `write_file_ide` -> `write_file`
    for _ in 0..2 {
        let chat_res = client
            .post(format!("{base}/v1/chat/completions"))
            .json(&json!({
                "model": "ag/gemini-4-carbon",
                "stream": false,
                "messages": [{ "role": "user", "content": "Create hello.txt" }],
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "write_file",
                        "description": "Write file",
                        "parameters": { "type": "object", "properties": { "path": { "type": "string" } } }
                    }
                }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(chat_res.status(), 200);
        let body: Value = chat_res.json().await.unwrap();
        let tc_name = body["choices"][0]["message"]["tool_calls"][0]["function"]["name"]
            .as_str()
            .unwrap();
        assert_eq!(tc_name, "write_file", "_ide suffix must be uncloaked");
    }

    {
        let hits = chat_hits.lock().unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].1, "NODE - 1");
        assert_eq!(
            hits[1].1, "NODE - 1",
            "Antigravity must stick to primary proxy and never round-robin"
        );
    }

    // 5. Kill `NODE - 1` -> Account 1 must fall back within its OWN slice (`NODE - 2`), never touching `NODE - 4..6`
    let _ = alive_flags[0].send(false);
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let failover_res = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "ag/gemini-4-carbon",
            "stream": false,
            "messages": [{ "role": "user", "content": "Create hello.txt" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "write_file",
                    "parameters": { "type": "object" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(failover_res.status(), 200);
    {
        let hits = chat_hits.lock().unwrap();
        let last = hits.last().unwrap();
        assert!(last.0.contains("access_acct_1"));
        assert_eq!(
            last.1, "NODE - 2",
            "Account 1 must fail over to NODE - 2 within its own slice"
        );
    }

    // 6. Exhaust Account 1 (`429 RESOURCE_EXHAUSTED`) -> TesRoute must automatically fall back to Account 2 on `NODE - 4`!
    exhaust_acct_1.store(true, Ordering::SeqCst);
    let acct_failover_res = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "ag/gemini-4-carbon",
            "stream": false,
            "messages": [{ "role": "user", "content": "Create hello.txt" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "write_file",
                    "parameters": { "type": "object" }
                }
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(acct_failover_res.status(), 200);
    {
        let hits = chat_hits.lock().unwrap();
        let last = hits.last().unwrap();
        assert!(
            last.0.contains("access_acct_2"),
            "expected fallback to Account 2 after Account 1 returned 429"
        );
        assert_eq!(
            last.1, "NODE - 4",
            "Account 2 must route through its dedicated primary proxy NODE - 4"
        );
    }

    // 7. Fail fast on no quota: disable Account 2 so NO enabled account has Argon quota.
    //    Requesting `ag/gemini-4-argon` must NOT attempt any upstream chat request and must
    //    immediately return "No quota for Argon."
    let acct2_id = accts[1]["id"].as_str().unwrap().to_string();
    let disable_res = client
        .patch(format!("{base}/api/admin/connections/{acct2_id}"))
        .json(&json!({ "enabled": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(disable_res.status(), 200);

    let hits_before = chat_hits.lock().unwrap().len();
    let no_quota_res = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "ag/gemini-4-argon-high",
            "stream": false,
            "messages": [{ "role": "user", "content": "Hello Argon" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_quota_res.status(), 403);
    let err_json: Value = no_quota_res.json().await.unwrap();
    assert_eq!(
        err_json["error"]["message"].as_str(),
        Some("No quota available for Argon.")
    );
    let hits_after = chat_hits.lock().unwrap().len();
    assert_eq!(
        hits_before, hits_after,
        "fail-fast must not dispatch any upstream request when no account has quota"
    );
}
