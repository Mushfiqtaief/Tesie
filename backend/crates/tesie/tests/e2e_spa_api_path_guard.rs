//! Regression guard: API-shaped paths must 404 (never 405) from the SPA fallback.
//!
//! Antigravity posts `/v1internal:*` before the IDE knows which endpoints Tesie serves. If the
//! static-file fallback rejects those with an empty-body 405, the client reports a broken account
//! setup instead of a missing endpoint. The path guard therefore has to run before the method
//! check, and this test pins that ordering.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use tesie::{build_app_router, AppOptions};
use tesie_providers::{ModelCatalogCache, OpenCodeConfig, OpenCodeEngine};
use tesie_proxy::{IpPoolSelector, MihomoConfig, MihomoController};
use tesie_store::Store;
use tokio::net::TcpListener;

fn core_bin() -> PathBuf {
    let exe_name = if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.data/bin")
        .join(exe_name);
    assert!(
        path.is_file(),
        "expected tesie-core binary at {}",
        path.display()
    );
    path
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

/// Boots the router with a real static-asset fallback rooted at a temp dir, so the SPA handler
/// is actually exercised rather than bypassed by the absence of `frontend_dir`.
async fn spawn_gateway_with_spa() -> (String, PathBuf, tempfile_dir::TempDir) {
    let store = Store::connect_in_memory().await.unwrap();
    let work_dir = tempfile_dir::TempDir::new();
    let www_dir = work_dir.path().join("www");
    std::fs::create_dir_all(&www_dir).unwrap();
    std::fs::write(www_dir.join("index.html"), "<html>spa</html>").unwrap();
    std::fs::write(www_dir.join("app.js"), "console.log(1)").unwrap();

    let ctl_port = free_port().await;
    let mihomo = MihomoController::start_managed(
        MihomoConfig {
            bin_path: core_bin(),
            work_dir: work_dir.path().to_path_buf(),
            ctl_addr: format!("127.0.0.1:{ctl_port}"),
            ctl_secret: "test-secret".into(),
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
            free_base_url: "http://127.0.0.1:1/v1".into(),
            zen_base_url: "http://127.0.0.1:1/v1".into(),
            timeout_ms: 1_000,
        },
    ));
    let app = build_app_router(
        store,
        mihomo.clone(),
        engine,
        Arc::new(ModelCatalogCache::new()),
        AppOptions {
            frontend_dir: Some(www_dir.clone()),
            fetch_live_models: false,
            ..Default::default()
        },
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), www_dir, work_dir)
}

#[tokio::test]
async fn unhandled_api_paths_answer_404_not_405_even_for_post() {
    let (base, _www, _dir) = spawn_gateway_with_spa().await;
    let client = reqwest::Client::new();

    for path in [
        "/v1internal:doesNotExist",
        "/v1internal/doesNotExist",
        "/api/doesNotExist",
        "/v1/doesNotExist",
    ] {
        let res = client
            .post(format!("{base}{path}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::NOT_FOUND,
            "POST {path} must be 404 (unknown endpoint); a 405 means the path guard ran after the method check"
        );
    }
}

#[tokio::test]
async fn non_api_paths_still_reject_wrong_methods_and_serve_assets() {
    let (base, _www, _dir) = spawn_gateway_with_spa().await;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{base}/some-spa-route"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "non-API paths keep the static-asset method restriction"
    );

    let res = client.get(format!("{base}/app.js")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.text().await.unwrap(),
        "console.log(1)",
        "real static assets must still be served"
    );

    let res = client.get(format!("{base}/deep/client/route")).send().await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "extensionless client routes fall back to index.html"
    );
    assert_eq!(res.text().await.unwrap(), "<html>spa</html>");
}

mod tempfile_dir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let n = SEQ.fetch_add(1, Ordering::SeqCst);
            let path =
                std::env::temp_dir().join(format!("tesie-spa-test-{}-{n}", std::process::id()));
            let _ = std::fs::create_dir_all(&path);
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
