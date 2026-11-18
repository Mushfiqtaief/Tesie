use std::path::{Path, PathBuf};
use std::process::Command;

pub fn default_production_home_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|s| !s.is_empty()) {
        return PathBuf::from(local).join("Tesie");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(home).join(".local").join("share").join("tesie");
    }
    PathBuf::from(".data")
}

pub fn resolve_daemon_exe_path(data_dir: &Path) -> PathBuf {
    let exe_name = if cfg!(windows) {
        "tesie-daemon.exe"
    } else {
        "tesie-daemon"
    };
    let prod_candidate = default_production_home_dir().join("bin").join(exe_name);
    if prod_candidate.is_file() {
        return prod_candidate;
    }
    let data_candidate = data_dir.join("bin").join(exe_name);
    if data_candidate.is_file() {
        return data_candidate;
    }
    if let Ok(cur) = std::env::current_exe()
        && let Some(dir) = cur.parent()
    {
        let sibling = dir.join(exe_name);
        if sibling.is_file() {
            return sibling;
        }
    }
    prod_candidate
}

#[cfg(windows)]
const RUN_KEY_PATH: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE_NAME: &str = "Tesie";

pub fn is_autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        let out = Command::new("reg")
            .args(["query", RUN_KEY_PATH, "/v", RUN_VALUE_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        matches!(out, Ok(o) if o.status.success())
    }
    #[cfg(target_os = "macos")]
    {
        macos_launch_agent_path()
            .map(|p| p.is_file())
            .unwrap_or(false)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let systemd = linux_systemd_service_path()
            .map(|p| p.is_file())
            .unwrap_or(false);
        let desktop = linux_desktop_autostart_path()
            .map(|p| p.is_file())
            .unwrap_or(false);
        systemd || desktop
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn macos_launch_agent_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("LaunchAgents")
                .join("ai.tesie.daemon.plist")
        })
}

#[cfg(target_os = "macos")]
fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_systemd_service_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("systemd")
                .join("user")
                .join("tesie.service")
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_desktop_autostart_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("autostart")
                .join("tesie.desktop")
        })
}

pub fn set_autostart_enabled(enabled: bool, daemon_exe: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        if enabled {
            let quoted = format!("\"{}\"", daemon_exe.display());
            let out = Command::new("reg")
                .args([
                    "add",
                    RUN_KEY_PATH,
                    "/v",
                    RUN_VALUE_NAME,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &quoted,
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("failed to invoke reg.exe: {e}"))?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            Ok(true)
        } else {
            let _ = Command::new("reg")
                .args(["delete", RUN_KEY_PATH, "/v", RUN_VALUE_NAME, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            Ok(false)
        }
    }
    #[cfg(target_os = "macos")]
    {
        let plist_path = macos_launch_agent_path()
            .ok_or_else(|| "HOME directory is not set".to_string())?;
        if enabled {
            if let Some(parent) = plist_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create LaunchAgents directory: {e}"))?;
            }
            let exe_escaped = xml_escape(&daemon_exe.to_string_lossy());
            let plist_body = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>ai.tesie.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_escaped}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
</dict>
</plist>
"#
            );
            std::fs::write(&plist_path, plist_body)
                .map_err(|e| format!("failed to write {}: {e}", plist_path.display()))?;
            Ok(true)
        } else {
            let _ = std::fs::remove_file(&plist_path);
            Ok(false)
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let service_path = linux_systemd_service_path()
            .ok_or_else(|| "HOME directory is not set".to_string())?;
        let desktop_path = linux_desktop_autostart_path()
            .ok_or_else(|| "HOME directory is not set".to_string())?;
        if enabled {
            if let Some(parent) = service_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create systemd user dir: {e}"))?;
            }
            let unit_body = format!(
                "[Unit]\nDescription=Tesie AI Routing Gateway\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart=\"{}\"\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
                daemon_exe.display()
            );
            std::fs::write(&service_path, unit_body)
                .map_err(|e| format!("failed to write {}: {e}", service_path.display()))?;

            let reload_ok = matches!(
                Command::new("systemctl")
                    .args(["--user", "daemon-reload"])
                    .output(),
                Ok(o) if o.status.success()
            );
            let enable_ok = reload_ok
                && matches!(
                    Command::new("systemctl")
                        .args(["--user", "enable", "tesie.service"])
                        .output(),
                    Ok(o) if o.status.success()
                );

            if !enable_ok {
                if let Some(parent) = desktop_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create autostart dir: {e}"))?;
                }
                let desktop_body = format!(
                    "[Desktop Entry]\nType=Application\nName=Tesie\nExec=\"{}\"\nHidden=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
                    daemon_exe.display()
                );
                std::fs::write(&desktop_path, desktop_body)
                    .map_err(|e| format!("failed to write {}: {e}", desktop_path.display()))?;
            }
            Ok(true)
        } else {
            let _ = Command::new("systemctl")
                .args(["--user", "disable", "tesie.service"])
                .output();
            let _ = std::fs::remove_file(&service_path);
            let _ = std::fs::remove_file(&desktop_path);
            Ok(false)
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (enabled, daemon_exe);
        Ok(false)
    }
}

/// Ensures `bin_dir` (e.g. `%LOCALAPPDATA%\Tesie\bin`) is registered on Windows User `PATH`
/// and writes lightweight shims into `%USERPROFILE%\.local\bin` when present so already-open
/// shells also resolve `tesie` immediately.
pub fn ensure_global_cli_installed(bin_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut registered = Vec::new();
    let normalized_bin = bin_dir
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;

        let ps_script = r#"
$target = ($env:TESIE_CLI_BIN_DIR).TrimEnd('\', '/')
$current = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($null -eq $current) { $current = '' }
$parts = @($current -split ';' | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne '' })
$exists = $false
foreach ($p in $parts) {
    if ($p.TrimEnd('\', '/').ToLowerInvariant() -eq $target.ToLowerInvariant()) {
        $exists = $true
        break
    }
}
if (-not $exists) {
    $updated = (@($parts + $target)) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $updated, 'User')
}
try {
    Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);
'@ -ErrorAction SilentlyContinue | Out-Null
    $r = [UIntPtr]::Zero
    [Win32.NativeMethods]::SendMessageTimeout([IntPtr]0xffff, 0x001A, [UIntPtr]::Zero, 'Environment', 2, 1000, [ref]$r) | Out-Null
} catch {}
"#;
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
            .env("TESIE_CLI_BIN_DIR", &normalized_bin)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("failed to update Windows User PATH: {e}"))?;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() {
                "PowerShell PATH update failed".to_string()
            } else {
                err
            });
        }
        registered.push(PathBuf::from(&normalized_bin));

        // Write lightweight forwarder shims into %USERPROFILE%\.local\bin so existing terminals work too
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|s| !s.is_empty()) {
            let local_bin = PathBuf::from(profile).join(".local").join("bin");
            if local_bin.is_dir() {
                let exe_path = PathBuf::from(&normalized_bin).join("tesie.exe");
                let cmd_shim = local_bin.join("tesie.cmd");
                let cmd_body = format!("@echo off\r\n\"{}\" %*\r\n", exe_path.display());
                std::fs::write(&cmd_shim, cmd_body)
                    .map_err(|e| format!("failed to write {}: {e}", cmd_shim.display()))?;
                registered.push(cmd_shim);

                let sh_shim = local_bin.join("tesie");
                let posix_exe = exe_path.to_string_lossy().replace('\\', "/");
                let sh_body = format!("#!/usr/bin/env sh\nexec \"{posix_exe}\" \"$@\"\n");
                std::fs::write(&sh_shim, sh_body)
                    .map_err(|e| format!("failed to write {}: {e}", sh_shim.display()))?;
                registered.push(sh_shim);
            }
        }
    }

    #[cfg(all(unix, not(windows)))]
    {
        use std::os::unix::fs::PermissionsExt;

        let target_exe = PathBuf::from(&normalized_bin).join("tesie");
        let shim_body = format!("#!/usr/bin/env sh\nexec \"{}\" \"$@\"\n", target_exe.display());

        let home = std::env::var_os("HOME")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "HOME directory is not set".to_string())?;
        let home_dir = PathBuf::from(home);
        let local_bin = home_dir.join(".local").join("bin");
        std::fs::create_dir_all(&local_bin)
            .map_err(|e| format!("failed to create {}: {e}", local_bin.display()))?;

        let local_shim = local_bin.join("tesie");
        std::fs::write(&local_shim, &shim_body)
            .map_err(|e| format!("failed to write {}: {e}", local_shim.display()))?;
        std::fs::set_permissions(&local_shim, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("failed to chmod {}: {e}", local_shim.display()))?;
        registered.push(local_shim);

        for sys_bin in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let dir = Path::new(sys_bin);
            if dir.is_dir() {
                let sys_shim = dir.join("tesie");
                if std::fs::write(&sys_shim, &shim_body).is_ok()
                    && std::fs::set_permissions(&sys_shim, std::fs::Permissions::from_mode(0o755))
                        .is_ok()
                {
                    registered.push(sys_shim);
                }
            }
        }

        // Idempotently ensure ~/.local/bin is on PATH in interactive shell rc files
        let path_export = "\n# Added by Tesie\nexport PATH=\"$HOME/.local/bin:$PATH\"\n";
        for (rc_name, create_if_missing) in [
            (".zshrc", cfg!(target_os = "macos")),
            (".bashrc", false),
            (".profile", false),
        ] {
            let rc_path = home_dir.join(rc_name);
            let exists = rc_path.is_file();
            if exists || create_if_missing {
                let existing = std::fs::read_to_string(&rc_path).unwrap_or_default();
                if !existing.contains(".local/bin") {
                    let _ = std::fs::write(&rc_path, format!("{existing}{path_export}"));
                }
            }
        }
    }

    #[cfg(not(any(windows, unix)))]
    {
        registered.push(PathBuf::from(normalized_bin));
    }

    Ok(registered)
}
