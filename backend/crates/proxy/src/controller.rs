use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures::stream::{self, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tesie_core::{Result, TesRouteError};
use tesie_store::{now_ms, Store};
use tokio::process::Child;
use tracing::{debug, info, warn};

use crate::config_gen::{generate_mihomo_yaml, DEFAULT_MIHOMO_TEST_URL, TESROUTE_POOL_GROUP};

#[derive(Debug, Clone)]
pub struct MihomoConfig {
    pub bin_path: PathBuf,
    pub work_dir: PathBuf,
    pub ctl_addr: String,
    pub ctl_secret: String,
    pub allow_loopback_targets: bool,
    pub insecure_skip_verify: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MihomoHealth {
    pub status: String,
    pub ja4_cipher_hash: String,
    pub proxies_loaded: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MihomoProxyState {
    pub name: String,
    pub proxy_type: String,
    pub alive: bool,
    pub delay_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EgressProbeResult {
    pub proxy: String,
    pub egress_ip: String,
    pub delay_ms: i64,
}

#[derive(Debug, Clone)]
pub struct RelayRequest {
    pub target_url: String,
    pub proxy_name: String,
    pub method: String,
    pub fingerprint: String,
    pub timeout_ms: u64,
    pub upstream_authorization: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

pub struct MihomoController {
    base_url: String,
    ctl_addr: String,
    ctl_secret: String,
    work_dir: PathBuf,
    bin_path: Option<PathBuf>,
    allow_loopback_targets: bool,
    insecure_skip_verify: bool,
    http: reqwest::Client,
    child: Mutex<Option<Child>>,
    quarantined_proxies: Mutex<HashMap<String, i64>>,
    reload_lock: tokio::sync::Mutex<()>,
    test_lock: tokio::sync::Mutex<()>,
    last_auto_test_at: AtomicI64,
    stopped: AtomicBool,
}

impl MihomoController {
    pub fn connect_external(
        base_url: impl Into<String>,
        ctl_secret: impl Into<String>,
        work_dir: PathBuf,
        allow_loopback_targets: bool,
        insecure_skip_verify: bool,
    ) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let ctl_addr = base_url
            .strip_prefix("http://")
            .or_else(|| base_url.strip_prefix("https://"))
            .unwrap_or(&base_url)
            .to_string();
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| TesRouteError::ProxyRelay(format!("build mihomo client: {e}")))?;
        Ok(Self {
            base_url,
            ctl_addr,
            ctl_secret: ctl_secret.into(),
            work_dir,
            bin_path: None,
            allow_loopback_targets,
            insecure_skip_verify,
            http,
            child: Mutex::new(None),
            quarantined_proxies: Mutex::new(HashMap::new()),
            reload_lock: tokio::sync::Mutex::new(()),
            test_lock: tokio::sync::Mutex::new(()),
            last_auto_test_at: AtomicI64::new(0),
            stopped: AtomicBool::new(false),
        })
    }

    pub async fn start_managed(cfg: MihomoConfig, store: &Store) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.work_dir)
            .map_err(|e| TesRouteError::ProxyRelay(format!("create mihomo work_dir: {e}")))?;

        let abs_work_dir = std::path::absolute(&cfg.work_dir)
            .map_err(|e| TesRouteError::ProxyRelay(format!("resolve work_dir: {e}")))?;
        let config_path = abs_work_dir.join("config.yaml");

        let nodes = store.proxies().list_proxies().await?;
        let yaml = generate_mihomo_yaml(&cfg.ctl_addr, &cfg.ctl_secret, &nodes)?;
        std::fs::write(&config_path, yaml)
            .map_err(|e| TesRouteError::ProxyRelay(format!("write mihomo config.yaml: {e}")))?;

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            // Clean up any stale orphaned TesieCore processes before binding
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", "tesie-core.exe"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            std::thread::sleep(Duration::from_millis(200));
        }
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("pkill")
                .args(["-9", "-x", "tesie-core"])
                .output();
            std::thread::sleep(Duration::from_millis(200));
        }

        let mut cmd = tokio::process::Command::new(&cfg.bin_path);
        cmd.arg("-d")
            .arg(&abs_work_dir)
            .arg("-f")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|e| {
            TesRouteError::ProxyRelay(format!(
                "failed to spawn TesieCore at {}: {e}",
                cfg.bin_path.display()
            ))
        })?;

        let base_url = format!("http://{}", cfg.ctl_addr);
        let controller = Arc::new(Self {
            base_url,
            ctl_addr: cfg.ctl_addr,
            ctl_secret: cfg.ctl_secret,
            work_dir: abs_work_dir,
            bin_path: Some(cfg.bin_path),
            allow_loopback_targets: cfg.allow_loopback_targets,
            insecure_skip_verify: cfg.insecure_skip_verify,
            http: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| TesRouteError::ProxyRelay(format!("build mihomo client: {e}")))?,
            child: Mutex::new(Some(child)),
            quarantined_proxies: Mutex::new(HashMap::new()),
            reload_lock: tokio::sync::Mutex::new(()),
            test_lock: tokio::sync::Mutex::new(()),
            last_auto_test_at: AtomicI64::new(0),
            stopped: AtomicBool::new(false),
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut last_err = String::new();
        loop {
            if let Some(status) = controller.try_wait_child()? {
                return Err(TesRouteError::ProxyRelay(format!(
                    "TesieCore exited prematurely with status {status}"
                )));
            }
            match controller.health().await {
                Ok(h) => {
                    info!(
                        ja4 = %h.ja4_cipher_hash,
                        proxies = h.proxies_loaded,
                        "TesieCore controller ready"
                    );
                    return Ok(controller);
                }
                Err(e) => {
                    last_err = e.to_string();
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(TesRouteError::ProxyRelay(format!(
                    "timed out waiting for TesieCore /tesie/health ({last_err})"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn try_wait_child(&self) -> Result<Option<std::process::ExitStatus>> {
        let mut guard = self.child.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            return child
                .try_wait()
                .map_err(|e| TesRouteError::ProxyRelay(format!("check mihomo process: {e}")));
        }
        Ok(None)
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
    }

    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    pub fn last_auto_test_at(&self) -> Option<i64> {
        let ts = self.last_auto_test_at.load(Ordering::Relaxed);
        (ts > 0).then_some(ts)
    }

    /// Automatically checks if the managed TesieCore child process is running,
    /// respawning it immediately if it exited or crashed so the proxy relay never stays offline.
    pub async fn ensure_child_running(&self, store: &Store) -> Result<()> {
        if self.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(bin_path) = &self.bin_path else {
            return Ok(());
        };

        let is_dead = {
            let mut guard = self.child.lock().unwrap();
            match guard.as_mut() {
                Some(child) => child.try_wait().map(|st| st.is_some()).unwrap_or(true),
                None => true,
            }
        };

        if is_dead {
            warn!("TesieCore child process is not running; regenerating config and respawning");
            let nodes = store.proxies().list_proxies().await.unwrap_or_default();
            let yaml = generate_mihomo_yaml(&self.ctl_addr, &self.ctl_secret, &nodes)?;
            let config_path = self.work_dir.join("config.yaml");
            let _ = std::fs::write(&config_path, yaml);

            let mut cmd = tokio::process::Command::new(bin_path);
            cmd.arg("-d")
                .arg(&self.work_dir)
                .arg("-f")
                .arg(&config_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            {
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }

            match cmd.spawn() {
                Ok(child) => {
                    {
                        let mut guard = self.child.lock().unwrap();
                        *guard = Some(child);
                    }
                    info!("respawned TesieCore process successfully");
                    tokio::time::sleep(Duration::from_millis(600)).await;
                }
                Err(err) => {
                    warn!(error = %err, "failed to respawn TesieCore");
                }
            }
        }

        Ok(())
    }

    /// Immediately marks a proxy unhealthy/idle after an upstream dial/EOF/TLS failure so
    /// OpenCode Round-Robin skips it right away until a future Mihomo latency check succeeds.
    pub fn mark_proxy_unhealthy(&self, proxy_name: &str, now_ms: i64) {
        if proxy_name.trim().is_empty() || proxy_name.eq_ignore_ascii_case("DIRECT") {
            return;
        }
        if let Ok(mut guard) = self.quarantined_proxies.lock() {
            guard.insert(proxy_name.to_string(), now_ms);
        }
    }

    fn clear_proxy_quarantine(&self, proxy_name: &str) {
        if let Ok(mut guard) = self.quarantined_proxies.lock() {
            guard.remove(proxy_name);
        }
    }

    pub async fn health(&self) -> Result<MihomoHealth> {
        let mut req = self.http.get(format!("{}/tesie/health", self.base_url));
        if !self.ctl_secret.is_empty() {
            req = req.bearer_auth(&self.ctl_secret);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("health check failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(TesRouteError::ProxyRelay(format!(
                "TesieCore health status {status}: {text}"
            )));
        }
        resp.json::<MihomoHealth>()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("parse health JSON: {e}")))
    }

    /// Reads Mihomo's authoritative live proxy health & delay state via `GET /proxies`,
    /// combined with immediate live dial-failure quarantine until the next successful delay check.
    pub async fn fetch_proxy_states(&self) -> Result<HashMap<String, MihomoProxyState>> {
        self.fetch_proxy_states_internal(true).await
    }

    async fn fetch_proxy_states_internal(
        &self,
        apply_quarantine: bool,
    ) -> Result<HashMap<String, MihomoProxyState>> {
        let mut req = self.http.get(format!("{}/proxies", self.base_url));
        if !self.ctl_secret.is_empty() {
            req = req.bearer_auth(&self.ctl_secret);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("mihomo /proxies failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(TesRouteError::ProxyRelay(format!(
                "mihomo /proxies status {status}: {text}"
            )));
        }

        let payload: Value = resp
            .json()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("parse mihomo /proxies JSON: {e}")))?;

        let quarantined: HashMap<String, i64> = if apply_quarantine {
            self.quarantined_proxies
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        let mut states = HashMap::new();
        if let Some(map) = payload.get("proxies").and_then(Value::as_object) {
            for (name, item) in map {
                let proxy_type = item
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .to_string();
                let is_quarantined = quarantined.contains_key(name);
                let alive = !is_quarantined
                    && item.get("alive").and_then(Value::as_bool).unwrap_or(true);
                let delay_ms = if is_quarantined {
                    None
                } else {
                    item.get("history")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.last())
                        .and_then(|h| h.get("delay"))
                        .and_then(Value::as_i64)
                        .filter(|&d| d > 0)
                };
                states.insert(
                    name.clone(),
                    MihomoProxyState {
                        name: name.clone(),
                        proxy_type,
                        alive,
                        delay_ms,
                    },
                );
            }
        }

        Ok(states)
    }

    /// Uses Mihomo's built-in single-proxy latency check: `GET /proxies/{name}/delay`.
    /// Returns `Ok(Some(ms))` when healthy, or `Ok(None)` when Mihomo marks the proxy timed out / failed.
    pub async fn test_proxy_delay(
        &self,
        proxy_name: &str,
        test_url: Option<&str>,
        timeout_ms: u16,
    ) -> Result<Option<i64>> {
        let encoded_name = encode_uri_path_segment(proxy_name);
        let url = test_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_MIHOMO_TEST_URL);
        let timeout_clamped = timeout_ms.clamp(500, 30_000);

        let mut req = self
            .http
            .get(format!("{}/proxies/{}/delay", self.base_url, encoded_name))
            .query(&[
                ("url", url),
                ("timeout", &timeout_clamped.to_string()),
            ]);
        if !self.ctl_secret.is_empty() {
            req = req.bearer_auth(&self.ctl_secret);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("mihomo delay test {proxy_name}: {e}")))?;

        if resp.status().is_success() {
            let body: Value = resp.json().await.map_err(|e| {
                TesRouteError::ProxyRelay(format!("parse mihomo delay JSON: {e}"))
            })?;
            let delay = body
                .get("delay")
                .and_then(Value::as_i64)
                .filter(|&d| d > 0);
            if delay.is_some() {
                self.clear_proxy_quarantine(proxy_name);
            } else {
                self.mark_proxy_unhealthy(proxy_name, now_ms());
            }
            return Ok(delay);
        }

        let code = resp.status().as_u16();
        if code == 502 || code == 503 || code == 504 || code == 408 {
            self.mark_proxy_unhealthy(proxy_name, now_ms());
            return Ok(None);
        }

        let text = resp.text().await.unwrap_or_default();
        Err(TesRouteError::ProxyRelay(format!(
            "mihomo delay test {proxy_name} returned {code}: {text}"
        )))
    }

    /// Uses Mihomo's built-in group latency test (`GET /group/TESROUTE_POOL/delay`) to test
    /// all proxies in the global pool concurrently, then syncs Mihomo's authoritative `alive`
    /// and `delay` states into SQLite.
    pub async fn test_all_delays(
        &self,
        store: &Store,
        test_url: Option<&str>,
    ) -> Result<Vec<MihomoProxyState>> {
        let _guard = self.test_lock.lock().await;
        let url = test_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_MIHOMO_TEST_URL);

        let encoded_group = encode_uri_path_segment(TESROUTE_POOL_GROUP);
        let mut req = self
            .http
            .get(format!("{}/group/{}/delay", self.base_url, encoded_group))
            .query(&[("url", url), ("timeout", "5000")]);
        if !self.ctl_secret.is_empty() {
            req = req.bearer_auth(&self.ctl_secret);
        }

        // Even if all proxies time out (504), Mihomo's GroupBase.URLTest finishes updating
        // `alive` and `history` on every member before returning.
        let _ = req.send().await;

        let states_map = self.fetch_proxy_states_internal(false).await?;
        let tested_at = now_ms();
        self.last_auto_test_at.store(tested_at, Ordering::Relaxed);

        let proxies = store.proxies().list_proxies().await?;
        let mut out = Vec::with_capacity(proxies.len());
        let mut need_egress_resolution = Vec::new();

        for node in proxies {
            if let Some(st) = states_map.get(&node.name) {
                if st.alive {
                    self.clear_proxy_quarantine(&node.name);
                } else {
                    self.mark_proxy_unhealthy(&node.name, tested_at);
                }
                store
                    .proxies()
                    .sync_mihomo_delay(&node.name, st.alive, st.delay_ms, tested_at)
                    .await?;
                if st.alive && node.egress_ip.is_none() && !self.allow_loopback_targets {
                    need_egress_resolution.push(node);
                }
                out.push(st.clone());
            }
        }

        // Resolve real egress IP once for newly added healthy proxies that do not have an IP yet
        if !need_egress_resolution.is_empty() {
            let mut egress_stream = stream::iter(need_egress_resolution)
                .map(|node| async move {
                    let res = self.probe_proxy(&node.name, None).await;
                    (node, now_ms(), res)
                })
                .buffer_unordered(12);

            while let Some((node, ts, Ok(probe))) = egress_stream.next().await {
                let current_delay = states_map
                    .get(&node.name)
                    .and_then(|s| s.delay_ms)
                    .unwrap_or(probe.delay_ms);
                let _ = store
                    .proxies()
                    .update_proxy_probe(&node.id, &probe.egress_ip, current_delay, ts)
                    .await;
            }
        }

        Ok(out)
    }

    /// Starts the background automatic Mihomo proxy pool latency tester.
    /// Reads `proxy_test_interval_sec` (default 15s) dynamically so interval updates take effect live.
    pub fn start_auto_test_loop(self: &Arc<Self>, store: Store) {
        let ctl = Arc::clone(self);
        tokio::spawn(async move {
            // Initial test right after startup
            if let Err(err) = ctl.test_all_delays(&store, None).await {
                warn!(error = %err, "initial mihomo pool delay test failed");
            }

            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;
                if ctl.stopped.load(Ordering::Relaxed) {
                    break;
                }

                let interval_sec = store
                    .settings()
                    .get_settings()
                    .await
                    .map(|s| s.proxy_test_interval_sec)
                    .unwrap_or(15)
                    .clamp(5, 3600);
                let interval_ms = (interval_sec as i64) * 1000;
                let last_ts = ctl.last_auto_test_at.load(Ordering::Relaxed);
                let now = now_ms();

                if now.saturating_sub(last_ts) >= interval_ms {
                    debug!(interval_sec, "running scheduled mihomo proxy pool latency test");
                    ctl.last_auto_test_at.store(now, Ordering::Relaxed);
                    if let Err(err) = ctl.test_all_delays(&store, None).await {
                        warn!(error = %err, "scheduled mihomo pool delay test failed");
                        let _ = ctl.ensure_child_running(&store).await;
                    }
                }
            }
        });
    }

    pub async fn reload_from_store(&self, store: &Store) -> Result<()> {
        let _guard = self.reload_lock.lock().await;
        let nodes = store.proxies().list_proxies().await?;
        let yaml = generate_mihomo_yaml(&self.ctl_addr, &self.ctl_secret, &nodes)?;
        let config_path = self.work_dir.join("config.yaml");
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&config_path, &yaml)
            .map_err(|e| TesRouteError::ProxyRelay(format!("write config.yaml: {e}")))?;

        let mut req = self
            .http
            .put(format!("{}/configs?force=true", self.base_url))
            .json(&serde_json::json!({ "payload": yaml }));
        if !self.ctl_secret.is_empty() {
            req = req.bearer_auth(&self.ctl_secret);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("reload mihomo configs: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TesRouteError::ProxyRelay(format!(
                "mihomo /configs reload returned {status}: {body}"
            )));
        }
        Ok(())
    }

    pub async fn probe_proxy(
        &self,
        proxy_name: &str,
        probe_url: Option<&str>,
    ) -> Result<EgressProbeResult> {
        let encoded_name = encode_uri_path_segment(proxy_name);
        let mut req = self
            .http
            .get(format!("{}/tesie/egress/{}", self.base_url, encoded_name));
        if !self.ctl_secret.is_empty() {
            req = req.bearer_auth(&self.ctl_secret);
        }
        let mut query: Vec<(&str, &str)> = vec![("timeout", "6000")];
        if let Some(url) = probe_url
            && !url.trim().is_empty()
        {
            query.push(("url", url.trim()));
        }
        if self.allow_loopback_targets {
            query.push(("allow_loopback", "1"));
        }
        if self.insecure_skip_verify && self.allow_loopback_targets {
            query.push(("insecure_skip_verify", "1"));
        }
        req = req.query(&query);

        let resp = req
            .send()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("probe {proxy_name}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(TesRouteError::ProxyRelay(format!(
                "probe {proxy_name} failed ({status}): {text}"
            )));
        }

        resp.json::<EgressProbeResult>()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("parse probe JSON: {e}")))
    }

    pub async fn probe_all_enabled(
        &self,
        store: &Store,
        probe_url: Option<&str>,
    ) -> Result<Vec<EgressProbeResult>> {
        let settings = store.settings().get_settings().await?;
        let proxies = store
            .proxies()
            .list_enabled(settings.strict_proxy_mode)
            .await?;

        let probe_url_owned = probe_url.map(str::to_string);
        let mut probe_stream = stream::iter(proxies)
            .map(|node| {
                let url_ref = probe_url_owned.clone();
                async move {
                    let res = self.probe_proxy(&node.name, url_ref.as_deref()).await;
                    (node, now_ms(), res)
                }
            })
            .buffer_unordered(24);

        let mut results = Vec::new();
        while let Some((node, probed_at, outcome)) = probe_stream.next().await {
            match outcome {
                Ok(res) => {
                    store
                        .proxies()
                        .update_proxy_probe(&node.id, &res.egress_ip, res.delay_ms, probed_at)
                        .await?;
                    results.push(res);
                }
                Err(err) => {
                    warn!(proxy = %node.name, error = %err, "egress probe failed");
                    let _ = store.proxies().record_proxy_failure(&node.id, probed_at).await;
                }
            }
        }
        Ok(results)
    }

    pub async fn relay(&self, req: RelayRequest) -> Result<reqwest::Response> {
        let mut headers = HeaderMap::new();
        if !self.ctl_secret.is_empty() {
            let val = HeaderValue::from_str(&format!("Bearer {}", self.ctl_secret))
                .map_err(|e| TesRouteError::Internal(format!("invalid ctl secret header: {e}")))?;
            headers.insert(AUTHORIZATION, val);
        }

        insert_header(&mut headers, "x-tesie-target-url", &req.target_url)?;
        insert_header(&mut headers, "x-tesie-proxy", &req.proxy_name)?;
        insert_header(&mut headers, "x-tesie-method", &req.method)?;
        insert_header(&mut headers, "x-tesie-fingerprint", &req.fingerprint)?;
        insert_header(
            &mut headers,
            "x-tesie-timeout-ms",
            &req.timeout_ms.to_string(),
        )?;
        if let Some(auth) = &req.upstream_authorization
            && !auth.is_empty()
        {
            insert_header(&mut headers, "x-tesie-upstream-authorization", auth)?;
        }
        if self.allow_loopback_targets {
            insert_header(&mut headers, "x-tesie-allow-loopback", "1")?;
        }
        if self.insecure_skip_verify && self.allow_loopback_targets {
            insert_header(&mut headers, "x-tesie-insecure-skip-verify", "1")?;
        }

        for (k, v) in &req.headers {
            let kl = k.to_ascii_lowercase();
            if kl == "authorization" || kl.starts_with("x-tesie-") || kl.starts_with("x-tesroute-")
            {
                continue;
            }
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, val);
            }
        }

        let resp = self
            .http
            .post(format!("{}/tesie/relay", self.base_url))
            .headers(headers)
            .body(req.body)
            .send()
            .await
            .map_err(|e| TesRouteError::ProxyRelay(format!("relay request failed: {e}")))?;

        Ok(resp)
    }
}

fn insert_header(map: &mut HeaderMap, name: &'static str, value: &str) -> Result<()> {
    let hname = HeaderName::from_static(name);
    let hval = HeaderValue::from_str(value)
        .map_err(|e| TesRouteError::BadRequest(format!("invalid header {name}: {e}")))?;
    map.insert(hname, hval);
    Ok(())
}

/// Percent-encodes a URL path segment strictly according to RFC 3986 (`%20` for spaces, NEVER `+`)
/// so Mihomo's chi router resolves proxy names containing spaces like `NODE - 150`.
pub fn encode_uri_path_segment(input: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let is_unreserved = byte.is_ascii_alphanumeric()
            || matches!(*byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            out.push(*byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_proxy_names_with_spaces_using_percent20() {
        assert_eq!(encode_uri_path_segment("NODE - 150"), "NODE%20-%20150");
        assert_eq!(
            encode_uri_path_segment("proxy/alpha #1"),
            "proxy%2Falpha%20%231"
        );
    }
}
