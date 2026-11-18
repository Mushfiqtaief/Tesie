use tesie_core::{IpQuotaStatus, TesRouteError};
use tesie_store::keys::CreateApiKeyInput;
use tesie_store::proxies::CreateProxyInput;
use tesie_store::quotas::UpdateIpQuotaLimitsInput;
use tesie_store::settings::UpdateSettingsInput;
use tesie_store::{now_ms, Store};

#[tokio::test]
async fn shared_exit_ip_shares_quota_bucket_and_rotates_on_cooldown() {
    let store = Store::connect_in_memory().await.unwrap();
    store
        .settings()
        .update_settings(UpdateSettingsInput {
            strict_proxy_mode: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();

    let p1 = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            name: "p1".into(),
            proxy_url: Some("http://127.0.0.1:8001".into()),
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let p2 = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            name: "p2".into(),
            proxy_url: Some("socks5://127.0.0.1:8002".into()),
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let p3 = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            name: "p3".into(),
            proxy_url: Some("http://127.0.0.1:8003".into()),
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(p1.proxy_type, "http");
    assert_eq!(p2.proxy_type, "socks5");

    let t0 = now_ms();
    // p1 and p2 share exit IP 103.35.111.126; p3 has 198.51.100.77
    store
        .proxies()
        .update_proxy_probe(&p1.id, "103.35.111.126", 45, t0)
        .await
        .unwrap();
    store
        .proxies()
        .update_proxy_probe(&p2.id, "103.35.111.126", 55, t0)
        .await
        .unwrap();
    store
        .proxies()
        .update_proxy_probe(&p3.id, "198.51.100.77", 80, t0)
        .await
        .unwrap();

    // Candidates deduplicate by egress IP -> p1 (for 103.35.111.126) and p3 (for 198.51.100.77)
    let candidates = store.quotas().select_candidates(t0, true).await.unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].proxy.name, "p1");
    assert_eq!(candidates[1].proxy.name, "p3");

    // If p1 fails Mihomo delay test (`alive = false`), p2 (healthy on same IP) takes over!
    store
        .proxies()
        .sync_mihomo_delay("p1", false, None, t0 + 500)
        .await
        .unwrap();
    let failover_same_ip = store
        .quotas()
        .select_candidates(t0 + 500, true)
        .await
        .unwrap();
    assert_eq!(failover_same_ip.len(), 2);
    assert_eq!(failover_same_ip[0].proxy.name, "p2");

    // Rate-limit 103.35.111.126 -> both p1 and p2 are excluded because they share the ip_key!
    store
        .quotas()
        .record_rate_limit("103.35.111.126", None, "429 FreeUsageLimitError", t0)
        .await
        .unwrap();

    let after_rl = store
        .quotas()
        .select_candidates(t0 + 1_000, true)
        .await
        .unwrap();
    assert_eq!(after_rl.len(), 1);
    assert_eq!(after_rl[0].proxy.name, "p3");
    assert_eq!(after_rl[0].quota.ip_key, "198.51.100.77");

    // Default queued re-probe delay is 30 seconds (30_000 ms) when explicit_cooldown_ms is None
    let earliest = store
        .quotas()
        .earliest_cooldown_until(t0 + 1_000, true)
        .await
        .unwrap();
    assert_eq!(earliest, Some(t0 + 30_000));

    // At 29 minutes, queued node is still skipped
    let still_cooling = store
        .quotas()
        .select_candidates(t0 + 29 * 60_000, true)
        .await
        .unwrap();
    assert_eq!(still_cooling.len(), 1);

    // Even after 30+ minutes, queued node MUST NOT auto-promote to live without a successful probe!
    let still_queued_until_verified = store
        .quotas()
        .select_candidates(t0 + 30 * 60_000 + 1_000, true)
        .await
        .unwrap();
    assert_eq!(still_queued_until_verified.len(), 1);

    // Only once live probe/reset confirms availability does the IP return to the live pool
    let t_recover = t0 + 30 * 60_000 + 2_000;
    store
        .quotas()
        .reset_ip_quota("103.35.111.126", t_recover)
        .await
        .unwrap();
    store
        .quotas()
        .record_probe_available("103.35.111.126", t_recover)
        .await
        .unwrap();
    let recovered = store
        .quotas()
        .select_candidates(t_recover, true)
        .await
        .unwrap();
    assert_eq!(recovered.len(), 2);
    assert_eq!(recovered[0].quota.status, IpQuotaStatus::Active);
}

#[tokio::test]
async fn soft_cap_and_window_rollover_and_api_key_quota() {
    let store = Store::connect_in_memory().await.unwrap();
    let settings = store.settings().get_settings().await.unwrap();
    assert_eq!(settings.proxy_test_interval_sec, 15);

    let p = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            name: "alpha".into(),
            proxy_url: Some("socks5://127.0.0.1:9001".into()),
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let t0 = 1_700_000_000_000_i64;
    store
        .proxies()
        .update_proxy_probe(&p.id, "203.0.113.10", 20, t0)
        .await
        .unwrap();
    store
        .quotas()
        .update_ip_quota_limits(
            "203.0.113.10",
            UpdateIpQuotaLimitsInput {
                window_duration_ms: Some(10_000),
                max_requests_per_window: Some(Some(1)),
                max_tokens_per_window: None,
            },
            t0,
        )
        .await
        .unwrap();

    // First selection succeeds
    let c1 = store.quotas().select_candidates(t0, true).await.unwrap();
    assert_eq!(c1.len(), 1);
    store
        .quotas()
        .record_success("203.0.113.10", 50, 25, t0 + 100)
        .await
        .unwrap();

    // Soft cap of 1 request reached -> excluded until window rolls over
    let c2 = store
        .quotas()
        .select_candidates(t0 + 5_000, true)
        .await
        .unwrap();
    assert!(c2.is_empty());

    // After window_duration_ms elapses -> resets and returns
    let c3 = store
        .quotas()
        .select_candidates(t0 + 12_000, true)
        .await
        .unwrap();
    assert_eq!(c3.len(), 1);
    assert_eq!(c3[0].quota.requests_in_window, 0);
    assert_eq!(c3[0].quota.total_requests, 1);

    // API key quota verification
    let created = store
        .keys()
        .create_key(CreateApiKeyInput {
            name: "test-key".into(),
            quota_tokens_limit: Some(100),
            quota_requests_limit: Some(2),
        })
        .await
        .unwrap();
    let rec = store
        .keys()
        .verify_and_check_quota(&created.raw_key)
        .await
        .unwrap();
    assert_eq!(rec.id, created.record.id);

    store.keys().add_usage(&rec.id, 100, t0).await.unwrap();
    let err = store
        .keys()
        .verify_and_check_quota(&created.raw_key)
        .await
        .unwrap_err();
    assert!(matches!(err, TesRouteError::ApiKeyQuotaExceeded(_)));
}

#[tokio::test]
async fn node_sequential_naming_structured_edit_and_batch_delete() {
    use tesie_store::proxies::UpdateProxyInput;

    let store = Store::connect_in_memory().await.unwrap();

    // Auto-name single proxy -> NODE - 1
    let n1 = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            proxy_type: Some("socks5".into()),
            host: Some("10.0.0.1".into()),
            port: Some(1080),
            username: Some("alice".into()),
            password: Some("s3cr3t".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(n1.name, "NODE - 1");
    assert_eq!(n1.host.as_deref(), Some("10.0.0.1"));
    assert_eq!(n1.port, Some(1080));
    assert_eq!(n1.username.as_deref(), Some("alice"));
    assert_eq!(n1.password.as_deref(), Some("s3cr3t"));

    // Batch import maintains sequential NODE - 2, NODE - 3
    let imported = store
        .proxies()
        .bulk_import_proxy_urls("socks5://10.0.0.2:1081#custom\nhttp://bob:pw@10.0.0.3:8080\n")
        .await
        .unwrap();
    assert_eq!(imported.len(), 2);
    assert_eq!(imported[0].name, "NODE - 2");
    assert_eq!(imported[1].name, "NODE - 3");
    assert_eq!(imported[1].username.as_deref(), Some("bob"));

    // Structured edit updates host, port, type, user, password
    let edited = store
        .proxies()
        .update_proxy(
            &n1.id,
            UpdateProxyInput {
                name: Some("NODE - 1".into()),
                proxy_type: Some("http".into()),
                host: Some("10.0.0.99".into()),
                port: Some(3128),
                username: Some(Some("charlie".into())),
                password: Some(Some("newpw".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.proxy_type, "http");
    assert_eq!(edited.host.as_deref(), Some("10.0.0.99"));
    assert_eq!(edited.port, Some(3128));
    assert_eq!(edited.username.as_deref(), Some("charlie"));
    assert_eq!(edited.password.as_deref(), Some("newpw"));

    // Batch delete removes selected external proxies
    let deleted = store
        .proxies()
        .delete_proxies_batch(&[imported[0].id.clone(), imported[1].id.clone()])
        .await
        .unwrap();
    assert_eq!(deleted, 2);
}

#[tokio::test]
async fn quotas_respect_selected_proxy_config() {
    use tesie_store::provider_configs::UpdateProviderConfigInput;

    let store = Store::connect_in_memory().await.unwrap();
    let p1 = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            proxy_type: Some("socks5".into()),
            host: Some("10.10.0.1".into()),
            port: Some(1080),
            ..Default::default()
        })
        .await
        .unwrap();
    let p2 = store
        .proxies()
        .upsert_proxy(CreateProxyInput {
            proxy_type: Some("socks5".into()),
            host: Some("10.10.0.2".into()),
            port: Some(1080),
            ..Default::default()
        })
        .await
        .unwrap();

    let cfg = store
        .provider_configs()
        .update_config(
            "opencode",
            UpdateProviderConfigInput {
                proxy_mode: Some("selected".into()),
                selected_proxy_ids: Some(vec![p1.id.clone()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let scoped = store.quotas().list_ip_quotas_for_config(&cfg).await.unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].proxies, vec![p1.name]);
    assert_ne!(scoped[0].quota.ip_key, p2.ip_key());
}
