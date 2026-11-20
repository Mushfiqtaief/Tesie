import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowRightLeft,
  Check,
  Copy,
  Database,
  Download,
  HardDrive,
  Lock,
  LockOpen,
  Power,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import { api, setStoredAdminToken } from "../api";
import { Button, Card, INPUT_CLASS, StatusPill } from "../components/UiPrimitives";
import type { BackupImportSummary, SettingsFullResponse } from "../types";

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export const SettingsView = memo(function SettingsView({
  onNotify,
  onReloadAll,
  onAutostartChanged,
}: {
  onNotify: (text: string, isError?: boolean) => void;
  onReloadAll: () => Promise<void>;
  onAutostartChanged?: (enabled: boolean) => void;
}) {
  const [data, setData] = useState<SettingsFullResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [copiedPath, setCopiedPath] = useState(false);
  const [togglingAutostart, setTogglingAutostart] = useState(false);
  const [togglingFailover, setTogglingFailover] = useState(false);

  // Proxy health interval
  const [healthIntervalInput, setHealthIntervalInput] = useState("15");
  const [savingInterval, setSavingInterval] = useState(false);

  // Backup Export / Import
  const [includeSecrets, setIncludeSecrets] = useState(true);
  const [replaceOnImport, setReplaceOnImport] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [lastImportSummary, setLastImportSummary] = useState<BackupImportSummary | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  // Dashboard Password
  const [passwordInput, setPasswordInput] = useState("");
  const [savingPassword, setSavingPassword] = useState(false);

  // Maintenance actions
  const [resettingCooldowns, setResettingCooldowns] = useState(false);
  const [clearingLogs, setClearingLogs] = useState(false);
  const [clearingTokenSaver, setClearingTokenSaver] = useState(false);

  const loadSettings = useCallback(async () => {
    setLoading(true);
    try {
      const res = await api.getFullSettings();
      setData(res);
      setHealthIntervalInput(String(res.settings.proxy_test_interval_sec ?? 15));
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to load Tesie settings", true);
    } finally {
      setLoading(false);
    }
  }, [onNotify]);

  useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  async function handleCopyDataDir() {
    if (!data?.system.data_dir) return;
    try {
      await navigator.clipboard.writeText(data.system.data_dir);
      setCopiedPath(true);
      onNotify("Copied safe data directory path");
      setTimeout(() => setCopiedPath(false), 1500);
    } catch {
      onNotify("Unable to access clipboard", true);
    }
  }

  async function handleToggleAutostart() {
    if (!data) return;
    setTogglingAutostart(true);
    try {
      const res = await api.updateAutostart(!data.system.auto_start_enabled);
      setData((prev) =>
        prev
          ? {
              ...prev,
              system: { ...prev.system, auto_start_enabled: res.auto_start_enabled },
            }
          : prev,
      );
      onAutostartChanged?.(res.auto_start_enabled);
      onNotify(
        res.auto_start_enabled
          ? "Enabled Windows Startup (runs silently in System Tray)"
          : "Disabled Windows Startup launch",
      );
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to toggle Windows Startup", true);
    } finally {
      setTogglingAutostart(false);
    }
  }

  async function handleToggleFailover(exhaustive: boolean) {
    if (!data) return;
    setTogglingFailover(true);
    try {
      const res = await api.updateFullSettings({ exhaustive_failover: exhaustive });
      setData(res);
      onNotify(
        exhaustive
          ? "Enabled exhaustive pool failover (tries all available accounts & proxies before failing)"
          : "Enabled direct error mode (returns upstream error to client on first failure)",
      );
      await onReloadAll();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to update failover policy", true);
    } finally {
      setTogglingFailover(false);
    }
  }

  async function handleSaveHealthInterval(e: React.FormEvent) {
    e.preventDefault();
    const clamped = Math.max(5, Math.min(3600, Number(healthIntervalInput) || 15));
    setHealthIntervalInput(String(clamped));
    setSavingInterval(true);
    try {
      const updated = await api.updateFullSettings({ proxy_test_interval_sec: clamped });
      setData(updated);
      onNotify(`TesieCore proxy check interval saved (${clamped}s)`);
      await onReloadAll();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to save interval", true);
    } finally {
      setSavingInterval(false);
    }
  }

  async function handleExportBackup() {
    setExporting(true);
    try {
      const doc = await api.exportBackup(includeSecrets);
      const blob = new Blob([JSON.stringify(doc, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const stamp = new Date().toISOString().slice(0, 10);
      const a = document.createElement("a");
      a.href = url;
      a.download = `tesie-backup-${stamp}.json`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
      onNotify(
        includeSecrets
          ? "Exported full Tesie configuration backup"
          : "Exported sanitized backup (secrets omitted)",
      );
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to export backup", true);
    } finally {
      setExporting(false);
    }
  }

  async function handleFileSelected(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    setImporting(true);
    try {
      const raw = await file.text();
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      const res = await api.importBackup(parsed, replaceOnImport);
      setLastImportSummary(res.summary);
      onNotify(
        `Imported backup (${res.summary.proxies} proxies, ${res.summary.custom_providers} custom providers, ${res.summary.combos} combos)`,
      );
      await loadSettings();
      await onReloadAll();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to import backup JSON", true);
    } finally {
      setImporting(false);
    }
  }

  async function handleSavePassword(e: React.FormEvent) {
    e.preventDefault();
    const clean = passwordInput.trim();
    if (!clean) return;
    setSavingPassword(true);
    try {
      const res = await api.updateFullSettings({ admin_password: clean });
      setStoredAdminToken(clean);
      setData(res);
      setPasswordInput("");
      onNotify("Dashboard password enabled and saved in this browser");
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to save dashboard password", true);
    } finally {
      setSavingPassword(false);
    }
  }

  async function handleClearPassword() {
    setSavingPassword(true);
    try {
      const res = await api.updateFullSettings({ clear_admin_password: true });
      setStoredAdminToken("");
      setData(res);
      setPasswordInput("");
      onNotify("Dashboard password protection removed");
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to remove dashboard password", true);
    } finally {
      setSavingPassword(false);
    }
  }

  async function handleResetCooldowns() {
    setResettingCooldowns(true);
    try {
      const res = await api.resetAllCooldownsMaintenance();
      onNotify(
        `Cleared ${res.cleared_ips} queued IPs, ${res.cleared_connections} accounts, and ${res.cleared_model_locks + res.cleared_model_failures} model locks`,
      );
      await onReloadAll();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to reset cooldowns", true);
    } finally {
      setResettingCooldowns(false);
    }
  }

  async function handleClearLogs() {
    setClearingLogs(true);
    try {
      const res = await api.clearLogs();
      onNotify(`Cleared ${res.deleted} request log entries`);
      await onReloadAll();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to clear logs", true);
    } finally {
      setClearingLogs(false);
    }
  }

  async function handleClearTokenSaver() {
    setClearingTokenSaver(true);
    try {
      const res = await api.clearTokenSaverStats();
      onNotify(`Cleared ${res.deleted} Token Saver telemetry entries`);
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to clear Token Saver stats", true);
    } finally {
      setClearingTokenSaver(false);
    }
  }

  if (loading && !data) {
    return (
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {[1, 2, 3, 4].map((i) => (
          <div
            key={i}
            className="h-56 animate-pulse rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]"
          />
        ))}
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        {/* Card 1: Configuration Backup (Export & Import) */}
        <Card
          title="Configuration Backup"
          subtitle="Export or restore providers, proxies, combos, custom endpoints, API keys, and settings"
          actions={
            <StatusPill tone="accent">
              <Database className="h-3 w-3" />
              JSON v1
            </StatusPill>
          }
        >
          <div className="space-y-4">
            <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4 space-y-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <div className="text-sm font-medium text-[var(--text-primary)]">
                    Export Snapshot
                  </div>
                  <div className="text-xs text-[var(--text-muted)]">
                    Downloads a portable <code className="font-mono">.json</code> backup of your
                    Tesie setup.
                  </div>
                </div>
                <Button
                  variant="primary"
                  size="sm"
                  disabled={exporting}
                  onClick={() => void handleExportBackup()}
                >
                  <Download className="h-4 w-4" />
                  {exporting ? "Exporting…" : "Export JSON"}
                </Button>
              </div>

              <label className="flex cursor-pointer items-center gap-2 text-xs text-[var(--text-secondary)] select-none">
                <input
                  type="checkbox"
                  checked={includeSecrets}
                  onChange={(e) => setIncludeSecrets(e.target.checked)}
                  className="h-4 w-4 rounded border-[var(--border-strong)] accent-[var(--accent)]"
                />
                Include API keys, custom provider keys, OAuth tokens, and proxy passwords
              </label>
            </div>

            <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4 space-y-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <div className="text-sm font-medium text-[var(--text-primary)]">
                    Restore from Backup
                  </div>
                  <div className="text-xs text-[var(--text-muted)]">
                    Select a previously exported Tesie <code className="font-mono">.json</code>{" "}
                    file.
                  </div>
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={importing}
                  onClick={() => fileInputRef.current?.click()}
                >
                  <Upload className="h-4 w-4" />
                  {importing ? "Importing…" : "Import JSON"}
                </Button>
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".json,application/json"
                  className="hidden"
                  onChange={(e) => void handleFileSelected(e)}
                />
              </div>

              <div className="flex flex-wrap items-center gap-3 text-xs text-[var(--text-secondary)]">
                <label className="flex cursor-pointer items-center gap-1.5 select-none">
                  <input
                    type="radio"
                    name="import-mode"
                    checked={!replaceOnImport}
                    onChange={() => setReplaceOnImport(false)}
                    className="accent-[var(--accent)]"
                  />
                  Merge with existing items
                </label>
                <label className="flex cursor-pointer items-center gap-1.5 select-none">
                  <input
                    type="radio"
                    name="import-mode"
                    checked={replaceOnImport}
                    onChange={() => setReplaceOnImport(true)}
                    className="accent-[var(--accent)]"
                  />
                  Replace current configuration
                </label>
              </div>

              {lastImportSummary && (
                <div className="rounded-lg border border-[color-mix(in_srgb,var(--success)_35%,transparent)] bg-[var(--success-soft)] px-3 py-2 text-xs text-[var(--success)]">
                  Restored {lastImportSummary.proxies} proxies,{" "}
                  {lastImportSummary.custom_providers} custom providers,{" "}
                  {lastImportSummary.combos} combos, {lastImportSummary.connections} accounts,{" "}
                  {lastImportSummary.api_keys} API keys.
                </div>
              )}
            </div>
          </div>
        </Card>

        {/* Card 2: Background Service, System Tray & Storage */}
        <Card
          title="Background Daemon & Storage"
          subtitle="Always-on Windows System Tray service, boot persistence, and SQLite database path"
          actions={
            <StatusPill tone={data?.system.tray_active ? "success" : "neutral"} dot>
              {data?.system.tray_active ? "System Tray Active" : "Background Ready"}
            </StatusPill>
          }
        >
          <div className="space-y-4">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4">
              <div className="min-w-0">
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  Start on Windows Boot
                </div>
                <div className="text-xs text-[var(--text-muted)]">
                  Launches Tesie silently in your System Tray when Windows signs in.
                </div>
              </div>
              <Button
                variant={data?.system.auto_start_enabled ? "primary" : "secondary"}
                size="sm"
                disabled={togglingAutostart}
                onClick={() => void handleToggleAutostart()}
              >
                <Power className="h-4 w-4" />
                {data?.system.auto_start_enabled ? "Enabled (ON)" : "Disabled (OFF)"}
              </Button>
            </div>

            <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4 space-y-2.5">
              <div className="flex items-center justify-between gap-2">
                <span className="inline-flex items-center gap-1.5 text-xs font-medium text-[var(--text-secondary)]">
                  <HardDrive className="h-3.5 w-3.5 text-[var(--accent)]" />
                  Safe Data Path
                </span>
                <span className="font-mono text-xs text-[var(--text-muted)] tabular-nums">
                  DB: {formatBytes(data?.system.db_size_bytes ?? 0)}
                </span>
              </div>
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 truncate rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2.5 py-1.5 font-mono text-xs text-[var(--text-primary)]">
                  {data?.system.data_dir || "—"}
                </code>
                <Button variant="secondary" size="sm" onClick={() => void handleCopyDataDir()}>
                  {copiedPath ? (
                    <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                  ) : (
                    <Copy className="h-3.5 w-3.5" />
                  )}
                  Copy
                </Button>
              </div>
            </div>

            <form
              onSubmit={handleSaveHealthInterval}
              className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4"
            >
              <div>
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  TesieCore Proxy Health Interval
                </div>
                <div className="text-xs text-[var(--text-muted)]">
                  Background proxy latency check cadence (5–3600 seconds).
                </div>
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  min={5}
                  max={3600}
                  value={healthIntervalInput}
                  onChange={(e) => setHealthIntervalInput(e.target.value)}
                  className={`${INPUT_CLASS} w-24 text-right font-mono`}
                />
                <span className="text-xs text-[var(--text-muted)]">sec</span>
                <Button type="submit" variant="secondary" size="sm" disabled={savingInterval}>
                  {savingInterval ? "Saving…" : "Save"}
                </Button>
              </div>
            </form>
          </div>
        </Card>

        {/* Card: Routing & Failover Policy */}
        <Card
          title="Failover & Error Dispatch Policy"
          subtitle="Configure whether Tesie load balances and retries across all accounts/proxies or returns errors immediately"
          actions={
            <StatusPill tone={data?.settings.exhaustive_failover ? "success" : "warning"} dot>
              {data?.settings.exhaustive_failover ? "Try to the last (Active)" : "Direct Error Mode"}
            </StatusPill>
          }
        >
          <div className="space-y-4">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4">
              <div className="min-w-0 max-w-md">
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  Quota & Rate-Limit Failover Mode
                </div>
                <div className="text-xs text-[var(--text-muted)] mt-1">
                  {data?.settings.exhaustive_failover
                    ? "Tesie automatically load balances and retries across every ready proxy IP (OpenCode) and operational account (Antigravity). Downstream client only receives a failure if every available unit is exhausted."
                    : "Returns the upstream rate limit or quota error immediately on the very first failed attempt without retrying remaining proxies or accounts."}
                </div>
              </div>

              <div className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-1">
                <button
                  type="button"
                  disabled={togglingFailover}
                  onClick={() => void handleToggleFailover(true)}
                  className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                    data?.settings.exhaustive_failover
                      ? "bg-[var(--accent)] text-white shadow-xs"
                      : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  <ArrowRightLeft className="h-3.5 w-3.5" />
                  Try to the last
                </button>
                <button
                  type="button"
                  disabled={togglingFailover}
                  onClick={() => void handleToggleFailover(false)}
                  className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                    !data?.settings.exhaustive_failover
                      ? "bg-[var(--warning)] text-black font-semibold shadow-xs"
                      : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  Show error immediately
                </button>
              </div>
            </div>

            <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-3.5 flex items-start gap-2.5">
              <Sparkles className="h-4 w-4 text-[var(--accent)] shrink-0 mt-0.5" />
              <div className="text-xs leading-relaxed text-[var(--text-secondary)]">
                <strong className="text-[var(--text-primary)] font-medium">Automatic Context-Window Compacting:</strong> Deterministic context length errors (<code className="font-mono text-[11px]">context_length_exceeded</code> / prompt too long) always fast-fail directly with clean HTTP 400 JSON so AI coding CLIs (OpenCode, Claude Code, Aider) can trigger their automatic compact workflow without burning unnecessary proxy retries.
              </div>
            </div>
          </div>
        </Card>

        {/* Card 3: Dashboard Password Protection (Optional) */}
        <Card
          title="Dashboard Access Lock"
          subtitle="Optional password to protect the Tesie web UI and /api/admin/* controls"
          actions={
            data?.security.protection_enabled ? (
              <StatusPill tone="warning" dot>
                <Lock className="h-3 w-3" />
                Protected
              </StatusPill>
            ) : (
              <StatusPill tone="success" dot>
                <LockOpen className="h-3 w-3" />
                Open (Localhost)
              </StatusPill>
            )
          }
        >
          <div className="space-y-4">
            <p className="text-xs leading-relaxed text-[var(--text-muted)]">
              Tesie runs locally on <code className="font-mono">127.0.0.1</code> and works without a
              password by default. Set a password only if this machine is shared or exposed on a LAN
              and you want to require authentication before editing providers, proxies, or API keys.
            </p>

            {data?.security.env_admin_token_active ? (
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 text-xs text-[var(--text-secondary)]">
                Locked via <code className="font-mono font-semibold">ADMIN_TOKEN</code> environment
                variable on the running server process. Unset that variable if you prefer managing a
                password directly here.
              </div>
            ) : (
              <form onSubmit={handleSavePassword} className="space-y-3">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    {data?.security.has_admin_password
                      ? "Change Dashboard Password"
                      : "Set Dashboard Password"}
                  </label>
                  <div className="flex flex-wrap gap-2">
                    <input
                      type="password"
                      placeholder={
                        data?.security.has_admin_password
                          ? "Enter new password…"
                          : "Leave blank to keep open on localhost"
                      }
                      value={passwordInput}
                      onChange={(e) => setPasswordInput(e.target.value)}
                      className={`${INPUT_CLASS} flex-1 font-mono`}
                    />
                    <Button
                      type="submit"
                      variant="primary"
                      size="md"
                      disabled={!passwordInput.trim() || savingPassword}
                    >
                      <ShieldCheck className="h-4 w-4" />
                      {savingPassword ? "Saving…" : "Save Password"}
                    </Button>
                    {data?.security.has_admin_password && (
                      <Button
                        type="button"
                        variant="secondary"
                        size="md"
                        disabled={savingPassword}
                        onClick={() => void handleClearPassword()}
                      >
                        Remove Lock
                      </Button>
                    )}
                  </div>
                </div>
              </form>
            )}
          </div>
        </Card>

        {/* Card 4: Gateway Maintenance & Diagnostics */}
        <Card
          title="Maintenance & Reset"
          subtitle="Clear active cooldown queues, historical request logs, or compression counters"
          actions={
            <Button variant="ghost" size="sm" onClick={() => void loadSettings()}>
              <RefreshCw className="h-3.5 w-3.5" />
              Refresh
            </Button>
          }
        >
          <div className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4">
              <div>
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  Reset All Cooldowns
                </div>
                <div className="text-xs text-[var(--text-muted)]">
                  Moves queued OpenCode IPs and cooling Antigravity accounts/models back to Ready.
                </div>
              </div>
              <Button
                variant="secondary"
                size="sm"
                disabled={resettingCooldowns}
                onClick={() => void handleResetCooldowns()}
              >
                <RotateCcw className="h-4 w-4 text-[var(--warning)]" />
                {resettingCooldowns ? "Resetting…" : "Reset Cooldowns"}
              </Button>
            </div>

            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4">
              <div>
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  Clear Request Logs
                </div>
                <div className="text-xs text-[var(--text-muted)]">
                  Deletes recorded request/latency history from <code className="font-mono">tesie.db</code>.
                </div>
              </div>
              <Button
                variant="secondary"
                size="sm"
                disabled={clearingLogs}
                onClick={() => void handleClearLogs()}
              >
                <Trash2 className="h-4 w-4 text-[var(--danger)]" />
                {clearingLogs ? "Clearing…" : "Clear Logs"}
              </Button>
            </div>

            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4">
              <div>
                <div className="text-sm font-medium text-[var(--text-primary)]">
                  Clear Token Saver Telemetry
                </div>
                <div className="text-xs text-[var(--text-muted)]">
                  Resets RTK compression &amp; style savings counters without altering settings.
                </div>
              </div>
              <Button
                variant="secondary"
                size="sm"
                disabled={clearingTokenSaver}
                onClick={() => void handleClearTokenSaver()}
              >
                <Trash2 className="h-4 w-4 text-[var(--text-muted)]" />
                {clearingTokenSaver ? "Resetting…" : "Reset Stats"}
              </Button>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
});
