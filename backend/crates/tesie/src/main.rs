use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rand::RngCore;
use serde_json::Value;
use tesie::{build_app_router, AppOptions};
use tesie_admin::system::{
    default_production_home_dir, ensure_global_cli_installed, resolve_daemon_exe_path,
};
use tesie_providers::{ModelCatalogCache, OpenCodeConfig, OpenCodeEngine};
use tesie_proxy::{IpPoolSelector, MihomoConfig, MihomoController};
use tesie_store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "tesie",
    version,
    about = "Tesie AI Routing Gateway powered by TesieCore — run `tesie` to wake up the background daemon and open the dashboard"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<TesieCommand>,

    /// Run foreground gateway server mode (used internally by tesie-daemon)
    #[arg(long, default_value_t = false)]
    serve: bool,

    /// When waking Tesie via bare `tesie`, do not open the browser automatically
    #[arg(long, default_value_t = false)]
    no_open: bool,

    #[arg(long, env = "HOST", default_value = "127.0.0.1")]
    host: String,

    #[arg(long, env = "PORT", default_value_t = 20129)]
    port: u16,

    /// Safe dedicated directory for Tesie SQLite data, TesieCore state, and logs.
    /// Defaults to `%LOCALAPPDATA%\Tesie` on Windows or `~/.local/share/tesie` on macOS/Linux.
    #[arg(long, env = "TESIE_DATA_DIR")]
    data_dir: Option<PathBuf>,

    #[arg(long, alias = "mihomo-bin", env = "TESIE_CORE_BIN")]
    core_bin: Option<PathBuf>,

    #[arg(
        long,
        alias = "mihomo-ctl-addr",
        env = "TESIE_CORE_CTL_ADDR",
        default_value = "127.0.0.1:29090"
    )]
    core_ctl_addr: String,

    #[arg(long, alias = "mihomo-secret", env = "TESIE_CORE_SECRET")]
    core_secret: Option<String>,

    #[arg(long, env = "ADMIN_TOKEN")]
    admin_token: Option<String>,

    /// Refuse to serve the admin API while no credential is configured. Enabled in the
    /// container image so a Docker deployment is never accidentally wide open.
    #[arg(
        long,
        env = "TESIE_REQUIRE_ADMIN_AUTH",
        default_value_t = false,
        action = clap::ArgAction::Set,
        value_parser = parse_boolish
    )]
    require_admin_auth: bool,

    #[arg(long, env = "REQUIRE_API_KEY", default_value_t = false)]
    require_api_key: bool,

    #[arg(long, env = "AUTO_PROBE_ON_START", default_value_t = true)]
    auto_probe: bool,

    #[arg(long, env = "FRONTEND_DIR")]
    frontend_dir: Option<PathBuf>,

    /// Import newline/comma-separated proxy URLs into tesie.db and exit cleanly.
    #[arg(long)]
    import_proxies_only: Option<String>,

    /// Print SOCKS4 proxy status from tesie.db and exit cleanly.
    #[arg(long, default_value_t = false)]
    list_socks4: bool,
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
enum TesieCommand {
    /// Wake up the background Tesie daemon & open the dashboard (default command)
    #[command(alias = "up")]
    Wake {
        /// Wake Tesie in the background without opening the browser
        #[arg(long, default_value_t = false)]
        no_open: bool,
    },
    /// Start Tesie daemon cleanly in the background (headless unless --open is passed)
    Start {
        /// Also open the dashboard in your default browser once healthy
        #[arg(long, default_value_t = false)]
        open: bool,
    },
    /// Show live Tesie daemon & TesieCore proxy pool status
    Status,
    /// Wake Tesie if needed and open the dashboard URL in your browser
    Open,
    /// Stop the running background Tesie daemon, gateway, and TesieCore processes
    Stop,
    /// Restart the background Tesie daemon cleanly
    Restart {
        /// Also open the dashboard in your default browser once restarted
        #[arg(long, default_value_t = false)]
        open: bool,
    },
    /// Install `tesie` globally onto system PATH (`%LOCALAPPDATA%\Tesie\bin` or `~/.local/bin`)
    Install,
    /// Run the foreground HTTP gateway server directly
    Serve,
}

impl Cli {
    fn should_run_server(&self) -> bool {
        self.serve
            || matches!(self.command, Some(TesieCommand::Serve))
            || (self.command.is_none()
                && (self.import_proxies_only.is_some() || self.list_socks4))
    }
}

/// Accepts the spellings people actually put in a compose file or shell export.
fn parse_boolish(raw: &str) -> std::result::Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(format!("expected true/false, got '{other}'")),
    }
}

fn resolve_data_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Some(legacy_env) = std::env::var_os("DATA_DIR").filter(|s| !s.is_empty()) {
        return PathBuf::from(legacy_env);
    }
    default_production_home_dir()
}

fn resolve_frontend_dir(explicit: Option<PathBuf>, data_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.join("index.html").is_file().then_some(p);
    }
    let mut candidates = vec![
        data_dir.join("www"),
        PathBuf::from("frontend").join("dist"),
        PathBuf::from("..").join("frontend").join("dist"),
        PathBuf::from("..").join("..").join("frontend").join("dist"),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        candidates.insert(0, bin_dir.join("www"));
        if let Some(parent) = bin_dir.parent() {
            candidates.insert(0, parent.join("www"));
        }
    }
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

fn resolve_core_bin(explicit: Option<PathBuf>, data_dir: &Path) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    let core_name = if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    };
    let mut candidates = vec![
        data_dir.join("bin").join(core_name),
        PathBuf::from(".data").join("bin").join(core_name),
        PathBuf::from("..").join(".data").join("bin").join(core_name),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(bin_dir) = exe.parent()
    {
        candidates.insert(0, bin_dir.join(core_name));
    }
    for c in candidates {
        if c.is_file() {
            return c;
        }
    }
    PathBuf::from(core_name)
}

fn ensure_safe_database_path(data_dir: &Path) -> PathBuf {
    let target_db = data_dir.join("tesie.db");
    if target_db.is_file() {
        return target_db;
    }

    // Migrate existing legacy database seamlessly so all proxies, logs, keys, and accounts stay safe
    let legacy_candidates = [
        data_dir.join("tesroute.db"),
        PathBuf::from(".data").join("tesroute.db"),
    ];
    for legacy in legacy_candidates {
        if legacy.is_file() {
            if std::fs::copy(&legacy, &target_db).is_ok() {
                let legacy_wal = PathBuf::from(format!("{}-wal", legacy.display()));
                let target_wal = PathBuf::from(format!("{}-wal", target_db.display()));
                if legacy_wal.is_file() {
                    let _ = std::fs::copy(&legacy_wal, &target_wal);
                }
                let legacy_shm = PathBuf::from(format!("{}-shm", legacy.display()));
                let target_shm = PathBuf::from(format!("{}-shm", target_db.display()));
                if legacy_shm.is_file() {
                    let _ = std::fs::copy(&legacy_shm, &target_shm);
                }
                info!(
                    from = %legacy.display(),
                    to = %target_db.display(),
                    "migrated database (including WAL) to dedicated Tesie storage path"
                );
                return target_db;
            }
        }
    }

    target_db
}

fn generate_secret() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

async fn http_get_local(host: &str, port: u16, path: &str) -> Option<(u16, String)> {
    let addr = format!("{host}:{port}");
    let mut stream = tokio::time::timeout(
        Duration::from_millis(450),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .ok()?
    .ok()?;

    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    tokio::time::timeout(Duration::from_millis(450), stream.write_all(req.as_bytes()))
        .await
        .ok()?
        .ok()?;

    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_millis(800), stream.read_to_end(&mut raw))
        .await
        .ok()?
        .ok()?;

    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()?;
    Some((status, body.to_string()))
}

async fn probe_local_gateway(host: &str, port: u16) -> Option<Option<Value>> {
    let (status, body) = http_get_local(host, port, "/healthz").await?;
    if status != 200 || !body.contains("ok") {
        return None;
    }
    let mut admin_status = match http_get_local(host, port, "/api/admin/status").await {
        Some((200, json_body)) => serde_json::from_str::<Value>(&json_body).ok(),
        _ => None,
    };
    if let Some(ref mut st) = admin_status
        && let Some((200, px_body)) = http_get_local(host, port, "/api/admin/proxies").await
        && let Ok(px_json) = serde_json::from_str::<Value>(&px_body)
        && let Some(arr) = px_json.get("proxies").and_then(|v| v.as_array())
    {
        let external: Vec<&Value> = arr
            .iter()
            .filter(|p| !p.get("is_direct").and_then(|v| v.as_bool()).unwrap_or(false))
            .collect();
        let healthy = external
            .iter()
            .filter(|p| p.get("alive").and_then(|v| v.as_bool()).unwrap_or(false))
            .count();
        if let Some(obj) = st.as_object_mut() {
            obj.insert("external_proxies_total".to_string(), Value::from(external.len()));
            obj.insert("external_proxies_healthy".to_string(), Value::from(healthy));
        }
    }
    Some(admin_status)
}

fn spawn_detached_daemon(data_dir: &Path, port: u16, admin_token: Option<&str>) -> Result<()> {
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::remove_file(data_dir.join("daemon.stop"));
    let daemon_exe = resolve_daemon_exe_path(data_dir);

    let mut cmd = if daemon_exe.is_file() && port == 20129 {
        std::process::Command::new(&daemon_exe)
    } else {
        let cur_exe = std::env::current_exe().context("resolve current tesie executable")?;
        let mut c = std::process::Command::new(cur_exe);
        c.arg("--serve")
            .arg("--port")
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(data_dir);
        c
    };

    cmd.current_dir(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(tok) = admin_token {
        cmd.env("ADMIN_TOKEN", tok);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    cmd.spawn()
        .map(|_| ())
        .with_context(|| format!("failed to launch Tesie daemon ({})", daemon_exe.display()))
}

fn stop_background_processes(data_dir: &Path) {
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(data_dir.join("daemon.stop"), b"stop");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if tesie::tray::post_close_to_running_tray() {
            std::thread::sleep(Duration::from_millis(650));
        }
        let self_filter = format!("PID ne {}", std::process::id());
        for image in ["tesie-daemon.exe", "tesie.exe", "tesie-core.exe"] {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", image, "/FI", &self_filter])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    }
    #[cfg(unix)]
    {
        for pid_name in ["tesie-daemon.pid", "tesie.pid"] {
            let pid_path = data_dir.join(pid_name);
            if let Ok(raw) = std::fs::read_to_string(&pid_path)
                && let Ok(pid) = raw.trim().parse::<u32>()
                && pid != std::process::id()
            {
                let _ = std::process::Command::new("kill")
                    .args(["-TERM", &pid.to_string()])
                    .output();
            }
            let _ = std::fs::remove_file(&pid_path);
        }
        let _ = std::process::Command::new("pkill")
            .args(["-TERM", "-x", "tesie-core"])
            .output();
    }
}

fn open_dashboard_url(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn print_gateway_status(
    state_label: &str,
    host: &str,
    port: u16,
    data_dir: &Path,
    status_json: Option<&Value>,
) {
    println!("Tesie · {state_label}");
    println!("  Dashboard   http://{host}:{port}");
    println!("  Gateway API http://{host}:{port}/v1");
    if let Some(st) = status_json {
        let core_ok = st
            .get("core_healthy")
            .or_else(|| st.get("mihomo_healthy"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let healthy = st
            .get("external_proxies_healthy")
            .and_then(|v| v.as_u64());
        let total = st
            .get("external_proxies_total")
            .or_else(|| st.get("total_proxies"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let core_word = if core_ok { "Online" } else { "Starting" };
        match healthy {
            Some(h) => println!("  TesieCore   {core_word} ({h}/{total} proxies healthy)"),
            None => println!("  TesieCore   {core_word} ({total} proxies)"),
        }
        if let Some(autostart) = st.get("auto_start_enabled").and_then(|v| v.as_bool()) {
            println!("  Auto-start  {}", if autostart { "Enabled on boot" } else { "Off" });
        }
    }
    println!("  Data Dir    {}", data_dir.display());
}

async fn wake_gateway(cli: &Cli, data_dir: &Path, open_browser: bool) -> Result<()> {
    let url = format!("http://{}:{}", cli.host, cli.port);
    if let Some(status_json) = probe_local_gateway(&cli.host, cli.port).await {
        print_gateway_status(
            "Online (already running)",
            &cli.host,
            cli.port,
            data_dir,
            status_json.as_ref(),
        );
        if open_browser && std::env::var_os("TESIE_NO_OPEN").is_none() {
            open_dashboard_url(&url);
        }
        return Ok(());
    }

    let start = Instant::now();
    spawn_detached_daemon(data_dir, cli.port, cli.admin_token.as_deref())?;

    let mut ready_status: Option<Option<Value>> = None;
    let deadline = start + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Some(st) = probe_local_gateway(&cli.host, cli.port).await {
            ready_status = Some(st);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    match ready_status {
        Some(st) => {
            let elapsed = start.elapsed().as_secs_f32();
            print_gateway_status(
                &format!("Online (woke up in {elapsed:.1}s)"),
                &cli.host,
                cli.port,
                data_dir,
                st.as_ref(),
            );
            if open_browser && std::env::var_os("TESIE_NO_OPEN").is_none() {
                open_dashboard_url(&url);
            }
            Ok(())
        }
        None => {
            anyhow::bail!(
                "Tesie daemon did not become healthy on {url} within 12s (check {})",
                data_dir.join("logs").join("tesie.log").display()
            );
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = resolve_data_dir(cli.data_dir.clone());

    if !cli.should_run_server() {
        match cli.command.as_ref() {
            Some(TesieCommand::Status) => {
                match probe_local_gateway(&cli.host, cli.port).await {
                    Some(st) => {
                        print_gateway_status(
                            "Online",
                            &cli.host,
                            cli.port,
                            &data_dir,
                            st.as_ref(),
                        );
                    }
                    None => {
                        println!("Tesie · Offline");
                        println!("  Data Dir    {}", data_dir.display());
                        println!("  Tip         Run `tesie` to wake up the background daemon.");
                        // Non-zero so this doubles as a health probe in scripts/containers.
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            Some(TesieCommand::Stop) => {
                stop_background_processes(&data_dir);
                let deadline = Instant::now() + Duration::from_secs(3);
                while Instant::now() < deadline {
                    if probe_local_gateway(&cli.host, cli.port).await.is_none() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                println!("Tesie · Stopped background daemon & TesieCore");
                return Ok(());
            }
            Some(TesieCommand::Restart { open }) => {
                stop_background_processes(&data_dir);
                tokio::time::sleep(Duration::from_millis(450)).await;
                return wake_gateway(&cli, &data_dir, *open && !cli.no_open).await;
            }
            Some(TesieCommand::Install) => {
                let prod_bin = default_production_home_dir().join("bin");
                let paths = ensure_global_cli_installed(&prod_bin)
                    .map_err(|e| anyhow::anyhow!("global CLI install failed: {e}"))?;
                println!("Tesie · Installed global CLI");
                for p in paths {
                    println!("  Registered  {}", p.display());
                }
                return Ok(());
            }
            Some(TesieCommand::Start { open }) => {
                return wake_gateway(&cli, &data_dir, *open && !cli.no_open).await;
            }
            Some(TesieCommand::Open) => {
                return wake_gateway(&cli, &data_dir, !cli.no_open).await;
            }
            Some(TesieCommand::Wake { no_open }) => {
                return wake_gateway(&cli, &data_dir, !*no_open && !cli.no_open).await;
            }
            Some(TesieCommand::Serve) => {}
            None => {
                return wake_gateway(&cli, &data_dir, !cli.no_open).await;
            }
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .init();

    let data_dir = resolve_data_dir(cli.data_dir);
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("create data dir {}", data_dir.display()))?;

    let db_path = ensure_safe_database_path(&data_dir);
    let store = Store::connect_file(&db_path)
        .await
        .map_err(|e| anyhow::anyhow!("connect sqlite store: {e}"))?;

    if let Some(raw_urls) = cli.import_proxies_only {
        let normalized = raw_urls.replace(',', "\n");
        let imported = store
            .proxies()
            .bulk_import_proxy_urls(&normalized)
            .await
            .map_err(|e| anyhow::anyhow!("import proxies: {e}"))?;
        let total = store.proxies().list_proxies().await.map(|v| v.len()).unwrap_or(0);
        store.pool().close().await;
        info!(
            imported = imported.len(),
            total_pool = total,
            db = %db_path.display(),
            "imported proxies into Tesie database"
        );
        return Ok(());
    }

    if cli.list_socks4 {
        let nodes = store
            .proxies()
            .list_proxies()
            .await
            .map_err(|e| anyhow::anyhow!("list proxies: {e}"))?;
        for p in nodes.iter().filter(|n| n.proxy_type.eq_ignore_ascii_case("socks4") || n.proxy_type.eq_ignore_ascii_case("socks4a")) {
            println!(
                "{} | type={} | alive={} | delay_ms={:?} | egress_ip={:?} | url={:?}",
                p.name, p.proxy_type, p.alive, p.last_delay_ms, p.egress_ip, p.proxy_url
            );
        }
        store.pool().close().await;
        return Ok(());
    }

    let bin_path = resolve_core_bin(cli.core_bin, &data_dir);
    let ctl_secret = cli.core_secret.unwrap_or_else(generate_secret);

    // Fail before spawning TesieCore so a misconfigured container exits immediately
    // instead of sitting up with a wide-open admin panel.
    if cli.require_admin_auth {
        let has_env_token = cli
            .admin_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        let has_password = store
            .settings()
            .has_admin_password()
            .await
            .unwrap_or(false);
        if !has_env_token && !has_password {
            bail!(
                "TESIE_REQUIRE_ADMIN_AUTH is enabled but no admin credential is configured.\n\
                 Set ADMIN_TOKEN in the environment (docker run -e ADMIN_TOKEN=...) or set a\n\
                 dashboard password first, then start again."
            );
        }
    }

    info!(
        core_bin = %bin_path.display(),
        ctl_addr = %cli.core_ctl_addr,
        data_dir = %data_dir.display(),
        "starting managed TesieCore proxy engine"
    );
    let core = MihomoController::start_managed(
        MihomoConfig {
            bin_path,
            work_dir: data_dir.join("core"),
            ctl_addr: cli.core_ctl_addr,
            ctl_secret,
            allow_loopback_targets: false,
            insecure_skip_verify: false,
        },
        &store,
    )
    .await
    .map_err(|e| anyhow::anyhow!("start TesieCore: {e}"))?;

    // Automatic TesieCore native proxy latency tester (default: every 15s, configurable)
    if cli.auto_probe {
        core.start_auto_test_loop(store.clone());
    }

    let selector = Arc::new(IpPoolSelector::new(store.clone()));
    let engine = Arc::new(OpenCodeEngine::new(
        store.clone(),
        core.clone(),
        selector,
        OpenCodeConfig::default(),
    ));

    // Automatic 30s OpenCode minimal-request per-IP quota checker + 60s Antigravity quota sync
    if cli.auto_probe {
        engine.start_auto_quota_check_loop();
    }

    // Periodic background log retention & cache pruning loop (every 30 minutes)
    let prune_store = store.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        loop {
            if let Ok(settings) = prune_store.settings().get_settings().await {
                let _ = prune_store
                    .usage()
                    .prune_logs(settings.log_retention_days, settings.log_max_entries)
                    .await;
                let _ = prune_store.response_cache().prune_expired().await;
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(1800)).await;
        }
    });
    let catalog = Arc::new(ModelCatalogCache::new());
    let frontend_dir = resolve_frontend_dir(cli.frontend_dir, &data_dir);
    if let Some(dir) = &frontend_dir {
        info!(frontend_dir = %dir.display(), "serving Tesie dashboard SPA");
    }

    let app = build_app_router(
        store,
        core.clone(),
        engine,
        catalog,
        AppOptions {
            admin_token: cli.admin_token,
            require_api_key: cli.require_api_key,
            fetch_live_models: true,
            frontend_dir,
            require_admin_auth: cli.require_admin_auth,
        },
    );

    let bind_addr = format!("{}:{}", cli.host, cli.port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("bind {bind_addr}"))?;
    info!(addr = %bind_addr, "Tesie gateway listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("shutdown signal received");
        })
        .await?;

    core.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_invocation_defaults_to_wake_mode() {
        let cli = Cli::try_parse_from(["tesie"]).expect("parse bare tesie");
        assert!(cli.command.is_none());
        assert!(!cli.should_run_server());
    }

    #[test]
    fn explicit_serve_and_daemon_flags_select_server_mode() {
        let serve_flag = Cli::try_parse_from(["tesie", "--serve"]).expect("parse --serve");
        assert!(serve_flag.should_run_server());

        let serve_sub = Cli::try_parse_from(["tesie", "serve"]).expect("parse subcommand serve");
        assert!(serve_sub.should_run_server());

        let serve_flag2 = Cli::try_parse_from([
            "tesie",
            "--serve",
            "--port",
            "20129",
            "--core-bin",
            "tesie-core.exe",
        ])
        .expect("parse server args");
        assert!(serve_flag2.should_run_server());
    }

    #[test]
    fn lifecycle_subcommands_stay_in_cli_mode() {
        for args in [
            vec!["tesie", "status"],
            vec!["tesie", "start"],
            vec!["tesie", "stop"],
            vec!["tesie", "restart"],
            vec!["tesie", "open"],
            vec!["tesie", "install"],
            vec!["tesie", "--no-open"],
        ] {
            let parsed = Cli::try_parse_from(&args).expect("parse lifecycle cmd");
            assert!(!parsed.should_run_server(), "unexpected serve mode for {args:?}");
        }
    }
}
