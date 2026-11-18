#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::sync::atomic::Ordering;
#[cfg(windows)]
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tesie_admin::system::default_production_home_dir;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

fn open_exclusive_singleton_lock(lock_path: &Path) -> Option<File> {
    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Exclusive OS-level file handle lock: automatically released by Windows if process exits/crashes
        opts.share_mode(0);
    }
    opts.open(lock_path).ok()
}

fn open_rotating_log(logs_dir: &Path) -> Option<File> {
    let _ = fs::create_dir_all(logs_dir);
    let log_path = logs_dir.join("tesie.log");
    if let Ok(meta) = fs::metadata(&log_path)
        && meta.len() >= MAX_LOG_BYTES
    {
        let old_path = logs_dir.join("tesie.log.old");
        let _ = fs::remove_file(&old_path);
        let _ = fs::rename(&log_path, &old_path);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .ok()
}

fn log_supervisor_line(logs_dir: &Path, msg: &str) {
    if let Some(mut f) = open_rotating_log(logs_dir) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[tesie-daemon ts={ts}] {msg}");
    }
}

fn resolve_tesie_binary(home_dir: &Path) -> PathBuf {
    let exe_name = if cfg!(windows) { "tesie.exe" } else { "tesie" };
    if let Ok(cur) = std::env::current_exe()
        && let Some(dir) = cur.parent()
    {
        let sibling = dir.join(exe_name);
        if sibling.is_file() {
            return sibling;
        }
    }
    home_dir.join("bin").join(exe_name)
}

fn main() {
    let home_dir = default_production_home_dir();
    let _ = fs::create_dir_all(&home_dir);
    let logs_dir = home_dir.join("logs");
    let stop_file = home_dir.join("daemon.stop");
    let lock_file = home_dir.join("tesie-daemon.lock");

    if std::env::args().any(|a| a == "--stop") {
        let _ = fs::write(&stop_file, b"stop");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            if tesie::tray::post_close_to_running_tray() {
                thread::sleep(Duration::from_millis(650));
            }
            let self_filter = format!("PID ne {}", std::process::id());
            for image in ["tesie-daemon.exe", "tesie.exe", "tesie-core.exe"] {
                let _ = Command::new("taskkill")
                    .args(["/F", "/IM", image, "/FI", &self_filter])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
            }
        }
        #[cfg(unix)]
        {
            for pid_name in ["tesie-daemon.pid", "tesie.pid"] {
                let pid_path = home_dir.join(pid_name);
                if let Ok(raw) = fs::read_to_string(&pid_path)
                    && let Ok(pid) = raw.trim().parse::<u32>()
                    && pid != std::process::id()
                {
                    let _ = Command::new("kill")
                        .args(["-TERM", &pid.to_string()])
                        .output();
                }
                let _ = fs::remove_file(&pid_path);
            }
            let _ = Command::new("pkill")
                .args(["-TERM", "-x", "tesie-core"])
                .output();
        }
        return;
    }

    let _ = fs::remove_file(&stop_file);

    #[cfg(unix)]
    {
        let daemon_pid_path = home_dir.join("tesie-daemon.pid");
        if let Ok(raw) = fs::read_to_string(&daemon_pid_path)
            && let Ok(existing_pid) = raw.trim().parse::<u32>()
            && existing_pid != std::process::id()
            && matches!(
                Command::new("kill")
                    .args(["-0", &existing_pid.to_string()])
                    .status(),
                Ok(s) if s.success()
            )
        {
            return;
        }
        let _ = fs::write(&daemon_pid_path, std::process::id().to_string());
    }

    // Ensure only a single background supervisor runs across the user session
    let Some(_singleton_guard) = open_exclusive_singleton_lock(&lock_file) else {
        return;
    };

    let tesie_bin = resolve_tesie_binary(&home_dir);
    let core_bin = home_dir.join("bin").join(if cfg!(windows) {
        "tesie-core.exe"
    } else {
        "tesie-core"
    });
    let www_dir = home_dir.join("www");

    log_supervisor_line(
        &logs_dir,
        &format!(
            "Tesie background supervisor started (home={}, bin={})",
            home_dir.display(),
            tesie_bin.display()
        ),
    );

    #[cfg(windows)]
    {
        let daemon_exe = std::env::current_exe()
            .unwrap_or_else(|_| home_dir.join("bin").join("tesie-daemon.exe"));
        let control = tesie::tray::DaemonControl::new(20129, home_dir.clone(), daemon_exe);
        let sup_control = Arc::clone(&control);
        let sup_home = home_dir.clone();
        let sup_logs = logs_dir.clone();
        let sup_stop = stop_file.clone();
        let sup_tesie_bin = tesie_bin.clone();
        let sup_core_bin = core_bin.clone();
        let sup_www_dir = www_dir.clone();

        thread::spawn(move || {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;

            loop {
                if sup_control.quit.load(Ordering::Acquire) || sup_stop.exists() {
                    log_supervisor_line(&sup_logs, "stop signal detected; exiting supervisor");
                    tesie::tray::post_close_to_running_tray();
                    break;
                }

                let stdout_cfg = open_rotating_log(&sup_logs)
                    .map(Stdio::from)
                    .unwrap_or_else(Stdio::null);
                let stderr_cfg = open_rotating_log(&sup_logs)
                    .map(Stdio::from)
                    .unwrap_or_else(Stdio::null);

                let mut cmd = Command::new(&sup_tesie_bin);
                cmd.current_dir(&sup_home)
                    .env("TESIE_TRAY_ACTIVE", "1")
                    .arg("--serve")
                    .arg("--port")
                    .arg("20129")
                    .arg("--data-dir")
                    .arg(&sup_home)
                    .arg("--core-bin")
                    .arg(&sup_core_bin)
                    .arg("--frontend-dir")
                    .arg(&sup_www_dir)
                    .stdin(Stdio::null())
                    .stdout(stdout_cfg)
                    .stderr(stderr_cfg)
                    .creation_flags(CREATE_NO_WINDOW);

                match cmd.spawn() {
                    Ok(mut child) => {
                        sup_control.child_pid.store(child.id(), Ordering::Release);
                        let status = child.wait();
                        sup_control.child_pid.store(0, Ordering::Release);
                        if sup_control.quit.load(Ordering::Acquire) || sup_stop.exists() {
                            log_supervisor_line(&sup_logs, "stop signal detected after exit; stopping supervisor");
                            tesie::tray::post_close_to_running_tray();
                            break;
                        }
                        if sup_control.restart.swap(false, Ordering::AcqRel) {
                            log_supervisor_line(&sup_logs, "restarting tesie gateway from system tray");
                            continue;
                        }
                        log_supervisor_line(
                            &sup_logs,
                            &format!("tesie exited ({status:?}); restarting in 2s"),
                        );
                    }
                    Err(err) => {
                        log_supervisor_line(
                            &sup_logs,
                            &format!("failed to launch tesie ({err}); retrying in 5s"),
                        );
                        thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                }

                thread::sleep(Duration::from_secs(2));
            }
        });

        tesie::tray::run_tray_loop(control);
    }

    #[cfg(not(windows))]
    {
        let daemon_pid_path = home_dir.join("tesie-daemon.pid");
        let child_pid_path = home_dir.join("tesie.pid");
        loop {
            if stop_file.exists() {
                log_supervisor_line(&logs_dir, "stop signal detected; exiting supervisor");
                let _ = fs::remove_file(&child_pid_path);
                let _ = fs::remove_file(&daemon_pid_path);
                break;
            }

            let stdout_cfg = open_rotating_log(&logs_dir)
                .map(Stdio::from)
                .unwrap_or_else(Stdio::null);
            let stderr_cfg = open_rotating_log(&logs_dir)
                .map(Stdio::from)
                .unwrap_or_else(Stdio::null);

            let mut cmd = Command::new(&tesie_bin);
            cmd.current_dir(&home_dir)
                .arg("--serve")
                .arg("--port")
                .arg("20129")
                .arg("--data-dir")
                .arg(&home_dir)
                .arg("--core-bin")
                .arg(&core_bin)
                .arg("--frontend-dir")
                .arg(&www_dir)
                .stdin(Stdio::null())
                .stdout(stdout_cfg)
                .stderr(stderr_cfg);

            match cmd.spawn() {
                Ok(mut child) => {
                    let _ = fs::write(&child_pid_path, child.id().to_string());
                    let status = child.wait();
                    let _ = fs::remove_file(&child_pid_path);
                    if stop_file.exists() {
                        let _ = fs::remove_file(&daemon_pid_path);
                        break;
                    }
                    log_supervisor_line(
                        &logs_dir,
                        &format!("tesie exited ({status:?}); restarting in 2s"),
                    );
                }
                Err(err) => {
                    log_supervisor_line(
                        &logs_dir,
                        &format!("failed to launch tesie ({err}); retrying in 5s"),
                    );
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }

            thread::sleep(Duration::from_secs(2));
        }
    }
}
