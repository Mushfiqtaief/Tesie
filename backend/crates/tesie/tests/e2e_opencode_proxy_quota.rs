use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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

static COUNTER: AtomicU64 = AtomicU64::new(1);

async fn spawn_connect_proxy(target_addr: SocketAddr) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
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
                let first_line = head.lines().next().unwrap_or("");
                if !first_line.starts_with("CONNECT ") {
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
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    });

    addr
}

#[derive(Clone)]
struct MockUpstreamState {
    exit_ip: String,
    rate_limit_chat: bool,
}

async fn spawn_mock_opencode(exit_ip: &str, rate_limit_chat: bool) -> SocketAddr {
    let state = MockUpstreamState {
        exit_ip: exit_ip.to_string(),
        rate_limit_chat,
    };

    let app = Router::new()
        .route("/trace", get(mock_trace))
        .route("/inference/openai/v1/chat/completions", post(mock_chat))
        .route("/inference/openai/v1/responses", post(mock_responses))
        .route("/inference/anthropic/v1/messages", post(mock_messages))
        .route("/inference/v1/chat/completions", post(mock_chat))
        .route("/inference/v1/responses", post(mock_responses))
        .route("/inference/v1/messages", post(mock_messages))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}

async fn mock_trace(State(st): State<MockUpstreamState>) -> String {
    format!("fl=test\nip={}\nts=1760000000\n", st.exit_ip)
}

fn verify_opencode_gates(headers: &HeaderMap, anthropic: bool) -> Result<(), String> {
    if anthropic {
        let api_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if api_key != "public" {
            return Err(format!("expected x-api-key public, got {api_key:?}"));
        }
        if headers.get("authorization").is_some() {
            return Err("unexpected Authorization header on Anthropic v2 lane".into());
        }
    } else {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer public" {
            return Err(format!("expected Authorization Bearer public, got {auth:?}"));
        }
    }
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if ua != "opencode/latest/2.1.1/cli" {
        return Err(format!("unexpected User-Agent: {ua:?}"));
    }
    let sid = headers
        .get("x-opencode-session")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !sid.starts_with("ses_") || sid.len() != 30 {
        return Err(format!("invalid x-opencode-session: {sid:?}"));
    }
    let proj = headers
        .get("x-opencode-project")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if proj.len() != 40 || !proj.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return Err(format!("invalid 40-hex x-opencode-project: {proj:?}"));
    }
    Ok(())
}

async fn mock_chat(
    State(st): State<MockUpstreamState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Err(msg) = verify_opencode_gates(&headers, false) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from(
                json!({ "error": { "type": "FreeTierError", "message": msg } }).to_string(),
            ))
            .unwrap();
    }

    if st.rate_limit_chat {
        return Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .header("Retry-After", "180")
            .header("Content-Type", "application/json")
            .body(Body::from(
                json!({
                    "error": {
                        "type": "FreeUsageLimitError",
                        "message": "Rate limit exceeded, reset after 180s"
                    }
                })
                .to_string(),
            ))
            .unwrap();
    }

    // Verify full OpenCode 2.1.1 v2 prompt + 13 official CLI tools + opaque mcp_<12hex> client tool alias
    let sys_text = body["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        sys_text.contains("You are opencode, an interactive CLI tool"),
        "missing official OpenCode CLI prompt"
    );
    assert!(sys_text.contains("<env>"), "missing <env> prompt marker");
    assert!(
        sys_text.contains("<available_skills>"),
        "missing <available_skills> prompt block"
    );

    let tool_names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    for official in ["read", "shell", "edit", "glob", "grep", "patch"] {
        assert!(
            tool_names.contains(&official),
            "missing {official} official v2 tool"
        );
    }

    // Minimal agent-shaped quota probe ("Reply with OK.", max_tokens = 64, 13 official v2 tools)
    if body["max_tokens"].as_i64() == Some(64)
        && body["messages"][1]["content"].as_str() == Some("Reply with OK.")
    {
        assert_eq!(tool_names.len(), 13, "probe must carry exact 13 official v2 tools");
        let probe_sse = concat!(
            "data: {\"id\":\"chatcmpl-probe\",\"object\":\"chat.completion.chunk\",\"created\":1760000000,\"model\":\"mimo-v2.6-flash-free\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"OK\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":1,\"total_tokens\":121}}\n\n",
            "data: [DONE]\n\n"
        );
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/event-stream")
            .body(Body::from(probe_sse))
            .unwrap();
    }

    assert!(
        !tool_names.contains(&"playwright_browser_run_code_unsafe"),
        "raw client tool name leaked upstream"
    );
    let wire_alias = tool_names
        .iter()
        .copied()
        .find(|n| n.starts_with("mcp_"))
        .expect("expected opaque mcp_<12hex> wire alias for client tool");

    let sse = format!(
        concat!(
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1760000000,\"model\":\"deepseek-v4.1-flash-free\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"tool_calls\":[{{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{{\"name\":\"{}\",\"arguments\":\"{{\\\"input\\\":\\\"ok\\\"}}\"}}}}]}},\"finish_reason\":null}}]}}\n\n",
            "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1760000000,\"model\":\"deepseek-v4.1-flash-free\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}],\"usage\":{{\"prompt_tokens\":42,\"completion_tokens\":18,\"total_tokens\":60}}}}\n\n",
            "data: [DONE]\n\n"
        ),
        wire_alias
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}

async fn mock_responses(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(msg) = verify_opencode_gates(&headers, false) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from(msg))
            .unwrap();
    }
    let instr = body["instructions"].as_str().unwrap_or("");
    assert!(instr.contains("<env>"), "missing <env> in instructions");
    let sse = concat!(
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello from spark\"}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello from spark\"}]}],\"usage\":{\"input_tokens\":10,\"output_tokens\":4,\"total_tokens\":14}}}\n\n"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}

async fn mock_messages(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Err(msg) = verify_opencode_gates(&headers, true) {
        return Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from(msg))
            .unwrap();
    }
    assert_eq!(
        headers
            .get("anthropic-version")
            .and_then(|v| v.to_str().ok()),
        Some("2023-06-01")
    );
    assert_eq!(
        headers
            .get("anthropic-beta")
            .and_then(|v| v.to_str().ok()),
        Some("interleaved-thinking-2025-05-14")
    );
    assert!(body["system"].to_string().contains("<env>"));
    let sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"union-alpha\",\"usage\":{\"input_tokens\":11}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello from union\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
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
async fn e2e_opencode_multi_ip_proxy_quota_and_failover() {
    let exe_name = if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    };
    let mihomo_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.data/bin")
        .join(exe_name);
    assert!(
        mihomo_bin.is_file(),
        "expected compiled mihomo binary at {}",
        mihomo_bin.display()
    );

    let mock_alpha_addr = spawn_mock_opencode("198.51.100.11", true).await;
    let mock_beta_addr = spawn_mock_opencode("198.51.100.22", false).await;
    let mock_gamma_addr = spawn_mock_opencode("198.51.100.33", false).await;

    let proxy_alpha_addr = spawn_connect_proxy(mock_alpha_addr).await;
    let proxy_beta_addr = spawn_connect_proxy(mock_beta_addr).await;
    let proxy_gamma_addr = spawn_connect_proxy(mock_gamma_addr).await;

    let work_dir = std::env::temp_dir().join(format!(
        "tesroute-e2e-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let store = Store::connect_in_memory().await.unwrap();
    let ctl_port = free_port().await;
    let ctl_addr = format!("127.0.0.1:{ctl_port}");

    let mihomo = MihomoController::start_managed(
        MihomoConfig {
            bin_path: mihomo_bin,
            work_dir: work_dir.clone(),
            ctl_addr,
            ctl_secret: "e2e-ctl-secret".into(),
            allow_loopback_targets: true,
            insecure_skip_verify: false,
        },
        &store,
    )
    .await
    .unwrap();

    let canonical_base_url = format!("http://{mock_alpha_addr}");
    let selector = Arc::new(IpPoolSelector::new(store.clone()));
    let engine = Arc::new(OpenCodeEngine::new(
        store.clone(),
        mihomo.clone(),
        selector,
        OpenCodeConfig {
            free_base_url: canonical_base_url.clone(),
            zen_base_url: canonical_base_url.clone(),
            timeout_ms: 15_000,
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
            ..Default::default()
        },
    );

    let app_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_addr = app_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(app_listener, app).await;
    });

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://{app_addr}");

    // 1. Enable strict_proxy_mode so DIRECT is never used for free-tier requests
    let res = client
        .put(format!("{base}/api/admin/settings"))
        .json(&json!({ "strict_proxy_mode": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // 2. Register proxy-alpha, proxy-beta, proxy-gamma, and a dead proxy (closed port) to test Mihomo health gating + Round Robin
    let dead_port = free_port().await;
    for (name, purl) in [
        ("proxy-alpha", format!("http://{proxy_alpha_addr}")),
        ("proxy-beta", format!("http://{proxy_beta_addr}")),
        ("proxy-gamma", format!("http://{proxy_gamma_addr}")),
        ("proxy-dead", format!("http://127.0.0.1:{dead_port}")),
    ] {
        let r = client
            .post(format!("{base}/api/admin/proxies"))
            .json(&json!({
                "name": name,
                "proxy_url": purl,
                "enabled": true
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201, "create proxy {name}");
    }

    // Verify Mihomo's built-in latency test marks `proxy-dead` as timed out / failed (`alive = false`)
    let dead_test_res: Value = client
        .post(format!("{base}/api/admin/proxies/proxy-dead/test"))
        .query(&[("url", format!("{canonical_base_url}/trace"))])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dead_test_res["alive"].as_bool(), Some(false));

    // Verify OpenCode Free (round-robin + per_ip) and Antigravity (fallback + per_account)
    // are present. Look them up by id rather than by index: the catalog grows as providers
    // are added, and a positional assertion breaks every time one is.
    let provs_json: Value = client
        .get(format!("{base}/api/admin/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provs = provs_json["providers"].as_array().unwrap();
    let find_prov = |id: &str| {
        provs
            .iter()
            .find(|p| p["id"].as_str() == Some(id))
            .unwrap_or_else(|| panic!("provider '{id}' missing from /api/admin/providers"))
    };

    let opencode = find_prov("opencode");
    assert_eq!(opencode["selection_strategy"].as_str(), Some("round-robin"));
    assert_eq!(opencode["quota_tracking"].as_str(), Some("per_ip"));

    let antigravity = find_prov("antigravity");
    assert_eq!(
        antigravity["selection_strategy"].as_str(),
        Some("fallback")
    );
    assert_eq!(antigravity["quota_tracking"].as_str(), Some("per_account"));

    // 3. Probe exit IPs through mihomo
    let probe_url = format!("{canonical_base_url}/trace");
    let probe_res = client
        .post(format!("{base}/api/admin/proxies/probe"))
        .query(&[("url", probe_url.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(probe_res.status(), 200);

    let proxies_json: Value = client
        .get(format!("{base}/api/admin/proxies"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let proxies_arr = proxies_json["proxies"].as_array().unwrap();
    let alpha_row = proxies_arr
        .iter()
        .find(|p| p["name"] == "proxy-alpha")
        .unwrap();
    let beta_row = proxies_arr
        .iter()
        .find(|p| p["name"] == "proxy-beta")
        .unwrap();
    assert_eq!(alpha_row["egress_ip"].as_str(), Some("198.51.100.11"));
    assert_eq!(beta_row["egress_ip"].as_str(), Some("198.51.100.22"));

    // 4. Send /v1/chat/completions with a gated tool name; proxy-alpha returns 429 and
    // TesRoute must seamlessly fail over to proxy-beta and restore the caller tool name.
    let chat_res = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "oc/deepseek-v4.1-flash-free",
            "stream": false,
            "messages": [{ "role": "user", "content": "Run tool" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "playwright_browser_run_code_unsafe",
                    "description": "Execute code",
                    "parameters": { "type": "object", "properties": { "input": { "type": "string" } } }
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(chat_res.status(), 200);
    assert_eq!(
        chat_res
            .headers()
            .get("x-tesroute-egress-ip")
            .and_then(|v| v.to_str().ok()),
        Some("198.51.100.22")
    );
    assert_eq!(
        chat_res
            .headers()
            .get("x-tesroute-proxy")
            .and_then(|v| v.to_str().ok()),
        Some("proxy-beta")
    );

    let chat_body: Value = chat_res.json().await.unwrap();
    assert_eq!(
        chat_body["choices"][0]["message"]["tool_calls"][0]["function"]["name"].as_str(),
        Some("playwright_browser_run_code_unsafe")
    );
    assert_eq!(chat_body["usage"]["total_tokens"].as_i64(), Some(60));

    // 5. Verify SQLite per-IP quota tracker state
    let quotas_json: Value = client
        .get(format!("{base}/api/admin/quotas/ips"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let quotas = quotas_json["ip_quotas"].as_array().unwrap();
    let q_alpha = quotas
        .iter()
        .find(|q| q["ip_key"] == "198.51.100.11")
        .unwrap();
    let q_beta = quotas
        .iter()
        .find(|q| q["ip_key"] == "198.51.100.22")
        .unwrap();

    assert_eq!(q_alpha["status"].as_str(), Some("cooldown"));
    assert_eq!(q_alpha["total_rate_limits"].as_i64(), Some(1));
    assert!(q_alpha["cooldown_until"].as_i64().unwrap() > 0);

    assert_eq!(q_beta["status"].as_str(), Some("active"));
    assert_eq!(q_beta["requests_in_window"].as_i64(), Some(1));
    assert_eq!(q_beta["tokens_in_window"].as_i64(), Some(60));

    // 6. Test /v1/responses (muse-spark) and /v1/messages (union-alpha) and verify Round-Robin across healthy proxies
    let resp_res = client
        .post(format!("{base}/v1/responses"))
        .json(&json!({
            "model": "muse-spark-1.3-contributor-free",
            "stream": false,
            "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_res.status(), 200);
    let rr_first_proxy = resp_res
        .headers()
        .get("x-tesroute-proxy")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    assert_ne!(rr_first_proxy, "proxy-dead");
    let resp_json: Value = resp_res.json().await.unwrap();
    assert_eq!(
        resp_json["output"][0]["content"][0]["text"].as_str(),
        Some("hello from spark")
    );

    let msg_res = client
        .post(format!("{base}/v1/messages"))
        .json(&json!({
            "model": "union-alpha",
            "max_tokens": 128,
            "stream": false,
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg_res.status(), 200);
    let rr_second_proxy = msg_res
        .headers()
        .get("x-tesroute-proxy")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();
    assert_ne!(rr_second_proxy, "proxy-dead");
    assert_ne!(
        rr_first_proxy, rr_second_proxy,
        "expected Round-Robin rotation between healthy proxies, got {rr_first_proxy} twice"
    );
    let msg_json: Value = msg_res.json().await.unwrap();
    assert_eq!(
        msg_json["content"][0]["text"].as_str(),
        Some("hello from union")
    );

    let models_json: Value = client
        .get(format!("{base}/v1/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let model_ids: Vec<&str> = models_json["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    assert!(model_ids.contains(&"deepseek-v4.1-flash-free"));
    assert!(model_ids.contains(&"iceman-free"));
    assert!(model_ids.contains(&"inkling-2-small-free"));
    assert!(!model_ids.contains(&"muse-spark-1.3-contributor-free"));
    assert!(!model_ids.contains(&"union-alpha"));
    assert!(
        !model_ids.iter().any(|m| m.starts_with("gemini-4-")),
        "Antigravity models must not appear in /v1/models when 0 Antigravity accounts are connected"
    );

    // Verify OpenCode Thinking Level + Probe Settings update & persistence
    let patch_cfg: Value = client
        .put(format!("{base}/api/admin/providers/opencode/config"))
        .json(&json!({
            "thinking_level": "xhigh",
            "probe_enabled": false,
            "probe_interval_sec": 80
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patch_cfg["config"]["thinking_level"].as_str(), Some("xhigh"));
    assert_eq!(patch_cfg["config"]["probe_enabled"].as_bool(), Some(false));
    assert_eq!(patch_cfg["config"]["probe_interval_sec"].as_i64(), Some(80));

    // 7. Verify usage summary (tokens_per_sec, by_proxy) and filterable logs endpoint
    let usage_json: Value = client
        .get(format!("{base}/api/admin/usage"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(usage_json["total_requests"].as_i64().unwrap() >= 3);
    assert!(usage_json["by_proxy"].as_array().is_some_and(|a| !a.is_empty()));
    assert!(usage_json["avg_tokens_per_sec"].as_f64().unwrap() > 0.0);
    // Dashboard `recent` feed must NEVER contain failed/429 requests
    let recent_feed = usage_json["recent"].as_array().unwrap();
    assert!(!recent_feed.is_empty());
    assert!(
        recent_feed.iter().all(|r| {
            let code = r["status_code"].as_i64().unwrap_or(0);
            (200..300).contains(&code)
        }),
        "Dashboard recent feed must never include failed requests: {recent_feed:?}"
    );

    let logs_json: Value = client
        .get(format!("{base}/api/admin/logs?status=2xx&limit=20"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let logs = logs_json["logs"].as_array().unwrap();
    assert!(!logs.is_empty());
    assert!(logs.iter().all(|l| l["status_code"] == 200));
    assert!(logs.iter().any(|l| l["tokens_per_sec"].as_f64().is_some_and(|v| v > 0.0)));
    assert!(logs.iter().any(|l| l["attempt_count"].as_i64().unwrap_or(0) >= 2));
    assert!(logs.iter().all(|l| l["session_id"].as_str().is_some_and(|s| s.starts_with("ses_"))));
    assert!(logs.iter().all(|l| l["upstream_url"].as_str().is_some_and(|u| u.contains("/inference/"))));

    let err_logs_json: Value = client
        .get(format!("{base}/api/admin/logs?status=429"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let err_logs = err_logs_json["logs"].as_array().unwrap();
    assert!(!err_logs.is_empty());
    assert!(err_logs[0]["error_detail"].as_str().is_some_and(|d| d.contains("FreeUsageLimitError")));

    // 8. Verify live-availability-gated cooldown reset & 30s minimal-request quota probe
    // 198.51.100.11 is genuinely rate-limited upstream -> `reset-all` MUST NOT reset it!
    let reset_blocked: Value = client
        .post(format!("{base}/api/admin/quotas/ips/reset-all"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reset_blocked["checked"].as_u64(), Some(1));
    assert_eq!(reset_blocked["reset"].as_u64(), Some(0));
    assert_eq!(reset_blocked["still_cooling"].as_u64(), Some(1));

    // Put 198.51.100.22 (whose upstream HAS real available quota) into temporary cooldown, then verify reset succeeds!
    store
        .quotas()
        .record_rate_limit("198.51.100.22", Some(300_000), "manual test", tesie_store::now_ms())
        .await
        .unwrap();
    let single_reset: Value = client
        .post(format!("{base}/api/admin/quotas/ips/198.51.100.22/reset"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(single_reset["reset"].as_bool(), Some(true));
    assert_eq!(single_reset["available"].as_bool(), Some(true));

    let check_res: Value = client
        .post(format!("{base}/api/admin/quotas/ips/check"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(check_res["summary"]["checked"].as_u64(), Some(3));
    assert_eq!(check_res["summary"]["ready"].as_u64(), Some(2));
    assert_eq!(check_res["summary"]["cooling"].as_u64(), Some(1));

    mihomo.shutdown();
    let _ = std::fs::remove_dir_all(&work_dir);
}
