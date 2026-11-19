//! Admin panel gate: fail-closed mode and unlock throttling.

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::Value;
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

/// Boots the real router against a fresh in-memory store and returns its base URL.
async fn spawn_gateway(opts: AppOptions) -> (String, tempfile_dir::TempDir) {
    let store = Store::connect_in_memory().await.unwrap();
    let work_dir = tempfile_dir::TempDir::new();
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
        opts,
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}"), work_dir)
}

/// Minimal temp-dir helper so the test does not pull in extra dev-dependencies.
mod tempfile_dir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let n = SEQ.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "tesie-auth-test-{}-{n}",
                std::process::id()
            ));
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

#[tokio::test]
async fn fail_closed_mode_rejects_requests_when_no_credential_is_configured() {
    let (base, _dir) = spawn_gateway(AppOptions {
        require_admin_auth: true,
        fetch_live_models: false,
        ..Default::default()
    })
    .await;
    let client = reqwest::Client::new();

    let res = client
        .get(format!("{base}/api/admin/providers"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "fail-closed mode must not serve the admin API without a credential"
    );

    // The status probe stays open so the SPA can explain why it is locked.
    let status: Value = client
        .get(format!("{base}/api/admin/auth/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["protected"].as_bool(), Some(true));
    assert_eq!(status["authenticated"].as_bool(), Some(false));
}

#[tokio::test]
async fn default_mode_stays_open_without_a_credential() {
    let (base, _dir) = spawn_gateway(AppOptions {
        fetch_live_models: false,
        ..Default::default()
    })
    .await;

    let res = reqwest::Client::new()
        .get(format!("{base}/api/admin/providers"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "desktop installs without a credential keep working"
    );
}

#[tokio::test]
async fn env_token_unlocks_and_a_wrong_token_is_rejected() {
    let (base, _dir) = spawn_gateway(AppOptions {
        admin_token: Some("s3cret-token".into()),
        require_admin_auth: true,
        fetch_live_models: false,
        ..Default::default()
    })
    .await;
    let client = reqwest::Client::new();

    let ok = client
        .get(format!("{base}/api/admin/providers"))
        .header("X-Admin-Token", "s3cret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let bearer = client
        .get(format!("{base}/api/admin/providers"))
        .header("Authorization", "Bearer s3cret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bearer.status(), StatusCode::OK);

    let bad = client
        .get(format!("{base}/api/admin/providers"))
        .header("X-Admin-Token", "wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn repeated_failed_unlocks_are_throttled() {
    let (base, _dir) = spawn_gateway(AppOptions {
        admin_token: Some("s3cret-token".into()),
        require_admin_auth: true,
        fetch_live_models: false,
        ..Default::default()
    })
    .await;
    let client = reqwest::Client::new();

    let url = format!("{base}/api/admin/providers");
    for attempt in 1..=5 {
        let res = client
            .get(&url)
            .header("X-Admin-Token", format!("wrong-{attempt}"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {attempt} should be a plain rejection before the threshold"
        );
    }

    // The sixth call trips the limiter, and the correct token is refused while locked out.
    let locked = client
        .get(&url)
        .header("X-Admin-Token", "s3cret-token")
        .send()
        .await
        .unwrap();
    assert_eq!(
        locked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "lockout must apply regardless of whether the supplied credential is correct"
    );
}
