use std::sync::Arc;

use serde_json::{json, Value};
use tesie::{build_app_router, AppOptions};
use tesie_providers::{ModelCatalogCache, OpenCodeConfig, OpenCodeEngine};
use tesie_proxy::{IpPoolSelector, MihomoConfig, MihomoController};
use tesie_store::Store;
use tokio::net::TcpListener;

#[tokio::test]
#[ignore]
async fn audit_every_listed_model_end_to_end() {
    let home = tesie_admin::system::default_production_home_dir();
    let prod_db = home.join("tesie.db");
    let core_bin = home.join("bin").join(if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    });
    assert!(prod_db.exists(), "prod db missing");
    assert!(core_bin.exists(), "tesie-core missing");

    let tmp_dir = std::env::temp_dir().join(format!("tesie_live_model_audit_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let tmp_db = tmp_dir.join("audit.db");
    std::fs::copy(&prod_db, &tmp_db).unwrap();

    let store = Store::connect_file(&tmp_db).await.unwrap();
    let ctl_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ctl_port = ctl_listener.local_addr().unwrap().port();
    drop(ctl_listener);

    let mihomo = MihomoController::start_managed(
        MihomoConfig {
            bin_path: core_bin,
            work_dir: tmp_dir.join("core"),
            ctl_addr: format!("127.0.0.1:{ctl_port}"),
            ctl_secret: "audit-secret".into(),
            allow_loopback_targets: false,
            insecure_skip_verify: false,
        },
        &store,
    )
    .await
    .unwrap();

    // Run one fast group health check so TesieCore knows alive proxies
    let _ = mihomo.test_all_delays(&store, None).await;

    let selector = Arc::new(IpPoolSelector::new(store.clone()));
    let engine = Arc::new(OpenCodeEngine::new(
        store.clone(),
        mihomo.clone(),
        selector,
        OpenCodeConfig {
            free_base_url: "https://opencode.ai".into(),
            zen_base_url: "https://opencode.ai".into(),
            timeout_ms: 45_000,
        },
    ));

    // Refresh Antigravity account quotas live so normalized model list is 100% fresh
    let refreshed = engine.antigravity().refresh_all_account_quotas().await.unwrap_or(0);
    println!("=== Antigravity refreshed accounts: {refreshed} ===");

    let catalog = Arc::new(ModelCatalogCache::new());
    let app = build_app_router(
        store.clone(),
        mihomo.clone(),
        engine.clone(),
        catalog,
        AppOptions {
            admin_token: None,
            require_api_key: false,
            fetch_live_models: true,
            frontend_dir: None,
            ..Default::default()
        },
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(55))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    // Grab an existing API key from the copied DB if keys exist
    let keys_res: Value = client
        .get(format!("{base}/api/admin/keys"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bearer_key = keys_res["keys"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|k| k["enabled"] == true && k["raw_key"].is_string())
                .and_then(|k| k["raw_key"].as_str())
                .map(str::to_string)
        });

    let mut req_models = client.get(format!("{base}/v1/models"));
    if let Some(k) = &bearer_key {
        req_models = req_models.bearer_auth(k);
    }
    let models_json: Value = req_models.send().await.unwrap().json().await.unwrap();
    let listed = models_json["data"].as_array().unwrap().clone();
    println!("=== Total /v1/models listed: {} ===", listed.len());
    for m in &listed {
        println!("  listed: id={:<34} owned_by={}", m["id"].as_str().unwrap_or(""), m["owned_by"].as_str().unwrap_or(""));
    }

    // 1. Test Argon fail-fast first
    let t_argon = std::time::Instant::now();
    let mut argon_req = client.post(format!("{base}/v1/chat/completions")).json(&json!({
        "model": "ag/gemini-4-argon",
        "stream": false,
        "messages": [{ "role": "user", "content": "Hi" }]
    }));
    if let Some(k) = &bearer_key {
        argon_req = argon_req.bearer_auth(k);
    }
    let argon_resp = argon_req.send().await.unwrap();
    let argon_ms = t_argon.elapsed().as_millis();
    let argon_status = argon_resp.status().as_u16();
    let argon_txt = argon_resp.text().await.unwrap_or_default();
    println!(
        "\n[FAIL-FAST CHECK] ag/gemini-4-argon -> HTTP {argon_status} in {argon_ms}ms: {argon_txt}"
    );

    // 2. Test EVERY listed non-combo model end-to-end
    println!("\n=== END-TO-END MODEL AUDIT ===");
    for entry in &listed {
        let id = entry["id"].as_str().unwrap_or("");
        let owner = entry["owned_by"].as_str().unwrap_or("");
        if owner == "combo" {
            continue;
        }
        let prefixed = if owner == "antigravity" {
            format!("ag/{id}")
        } else {
            format!("oc/{id}")
        };
        let started = std::time::Instant::now();
        let mut req = client
            .post(format!("{base}/v1/chat/completions"))
            .json(&json!({
                "model": prefixed,
                "stream": false,
                "max_tokens": 48,
                "messages": [{ "role": "user", "content": "Reply with the single word OK." }]
            }));
        if let Some(k) = &bearer_key {
            req = req.bearer_auth(k);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let proxy = resp
                    .headers()
                    .get("x-tesie-proxy")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("-")
                    .to_string();
                let txt = resp.text().await.unwrap_or_default();
                let ms = started.elapsed().as_millis();
                if status == 200 {
                    let v: Value = serde_json::from_str(&txt).unwrap_or_default();
                    let content = v["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .replace('\n', " ");
                    let reasoning = v["choices"][0]["message"]["reasoning_content"]
                        .as_str()
                        .unwrap_or("")
                        .replace('\n', " ");
                    let preview: String = if !content.is_empty() {
                        content.chars().take(60).collect()
                    } else {
                        format!("[reasoning] {}", reasoning.chars().take(50).collect::<String>())
                    };
                    println!(
                        "  [PASS] {:<36} ({:<11}) {:>5}ms via {:<10} -> {}",
                        prefixed, owner, ms, proxy, preview
                    );
                } else {
                    let snippet: String = txt.replace('\n', " ").chars().take(160).collect();
                    println!(
                        "  [FAIL] {:<36} ({:<11}) {:>5}ms HTTP {} -> {}",
                        prefixed, owner, ms, status, snippet
                    );
                }
            }
            Err(err) => {
                println!(
                    "  [ERR ] {:<36} ({:<11}) -> {}",
                    prefixed, owner, err
                );
            }
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
}
