import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowRightLeft,
  ArrowUpCircle,
  Check,
  Copy,
  Database,
  Download,
  ExternalLink,
  Github,
  Loader2,
  Lock,
  LockOpen,
  Power,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Sparkles,
  Trash2,
  Upload,
  Zap,
} from "lucide-react";
import { api, setStoredAdminToken } from "../api";
import { Button, INPUT_CLASS, StatusPill } from "../components/UiPrimitives";
import type {
  BackupImportSummary,
  ResponseCacheStats,
  SettingsFullResponse,
  UpdateApplyResponse,
  UpdateCheckResponse,
} from "../types";

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

  // Update System State
  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResponse | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [applyingUpdate, setApplyingUpdate] = useState(false);
  const [updateLog, setUpdateLog] = useState<string | null>(null);

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

  // Response Cache & Log Retention state
  const [cacheStats, setCacheStats] = useState<ResponseCacheStats | null>(null);
  const [togglingCache, setTogglingCache] = useState(false);
  const [purgingCache, setPurgingCache] = useState(false);
  const [pruningLogs, setPruningLogs] = useState(false);

  const loadCacheStats = useCallback(async () => {
    try {
      const stats = await api.getCacheStats();
      setCacheStats(stats);
    } catch {
      // Non-fatal
    }
  }, []);

  const loadSettings = useCallback(async () => {
    setLoading(true);
    try {
      const res = await api.getFullSettings();
      setData(res);
      setHealthIntervalInput(String(res.settings.proxy_test_interval_sec ?? 15));
      void loadCacheStats();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to load Tesie settings", true);
    } finally {
      setLoading(false);
    }
  }, [onNotify, loadCacheStats]);

  const handleCheckUpdate = useCallback(
    async (silent = false) => {
      setCheckingUpdate(true);
      try {
        const res = await api.checkUpdate();
        setUpdateInfo(res);
        if (!silent) {
          if (res.update_available) {
            onNotify(`New update available: v${res.latest_version}`);
          } else {
            onNotify(`Tesie is up to date (v${res.current_version})`);
          }
        }
      } catch (err) {
        if (!silent) {
          onNotify(err instanceof Error ? err.message : "Failed to check for updates", true);
        }
      } finally {
        setCheckingUpdate(false);
      }
    },
    [onNotify],
  );

  useEffect(() => {
    void loadSettings();
    void handleCheckUpdate(true);
  }, [loadSettings, handleCheckUpdate]);

  async function handleApplyUpdate() {
    setApplyingUpdate(true);
    setUpdateLog(null);
    try {
      const res: UpdateApplyResponse = await api.applyUpdate();
      if (res.status === "error") {
        onNotify(res.message || "Update encountered an error", true);
      } else {
        onNotify(res.message || "Update applied successfully!");
      }
      if (res.log) {
        setUpdateLog(res.log);
      }
      await loadSettings();
      await handleCheckUpdate(true);
      await onReloadAll();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to apply update", true);
    } finally {
      setApplyingUpdate(false);
    }
  }

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

  async function handleToggleCache(enabled: boolean) {
    if (!data) return;
    setTogglingCache(true);
    try {
      const updated = await api.updateFullSettings({ response_cache_enabled: enabled });
      setData(updated);
      onNotify(enabled ? "Response caching enabled" : "Response caching disabled");
      void loadCacheStats();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to toggle response caching", true);
    } finally {
      setTogglingCache(false);
    }
  }

  async function handleChangeCacheTtl(sec: number) {
    if (!data) return;
    try {
      const updated = await api.updateFullSettings({ response_cache_ttl_sec: sec });
      setData(updated);
      onNotify(`Response cache TTL set to ${sec >= 86400 ? `${sec / 86400}d` : `${sec / 3600}h`}`);
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to update cache TTL", true);
    }
  }

  async function handlePurgeCache() {
    setPurgingCache(true);
    try {
      const res = await api.purgeCache();
      onNotify(res.message || "Cache purged successfully");
      void loadCacheStats();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to purge cache", true);
    } finally {
      setPurgingCache(false);
    }
  }

  async function handleChangeLogRetention(days: number | null) {
    if (!data) return;
    try {
      const updated = await api.updateFullSettings({ log_retention_days: days });
      setData(updated);
      onNotify(days ? `Log retention set to ${days} days` : "Log retention set to unlimited");
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to update log retention", true);
    }
  }

  async function handleChangeMaxLogs(entries: number | null) {
    if (!data) return;
    try {
      const updated = await api.updateFullSettings({ log_max_entries: entries });
      setData(updated);
      onNotify(entries ? `Max log capacity set to ${entries.toLocaleString()}` : "Max log capacity set to unlimited");
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to update log capacity", true);
    }
  }

  async function handlePruneLogs() {
    setPruningLogs(true);
    try {
      const res = await api.pruneLogs();
      onNotify(res.message || "Pruned old logs and vacuumed database");
      await loadSettings();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to prune logs", true);
    } finally {
      setPruningLogs(false);
    }
  }

  const upstreamUrl =
    updateInfo?.upstream_repo || data?.system.upstream_repo || "https://github.com/Mushfiqtaief/Tesie";
  const currentVersion =
    updateInfo?.current_version || data?.system.version || "0.1.0";
  const hasUpdate = Boolean(updateInfo?.update_available);

  if (loading && !data) {
    return (
      <div className="mx-auto max-w-4xl space-y-4">
        {[1, 2, 3].map((i) => (
          <div
            key={i}
            className="h-44 animate-pulse rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]"
          />
        ))}
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-4xl space-y-6 pb-12">
      {/* ──────────────────────────────────────────────────────────── */}
      {/* Section 1: Upstream & Software Updates                       */}
      {/* ──────────────────────────────────────────────────────────── */}
      <section className="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-xs">
        <div className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 px-6 py-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold tracking-tight text-[var(--text-primary)]">
                Software Updates &amp; Upstream
              </h2>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Check GitHub releases, install updates in one click, and browse the upstream repository.
              </p>
            </div>
            <div className="flex items-center gap-2">
              {hasUpdate ? (
                <StatusPill tone="warning" dot>
                  Update v{updateInfo?.latest_version} Available
                </StatusPill>
              ) : (
                <StatusPill tone="success" dot>
                  Up to Date (v{currentVersion})
                </StatusPill>
              )}
            </div>
          </div>
        </div>

        {/* Update Available Highlight Banner */}
        {hasUpdate && (
          <div className="border-b border-amber-500/20 bg-amber-500/10 px-6 py-4 text-xs">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="space-y-1 min-w-0 flex-1">
                <div className="flex items-center gap-2 font-medium text-amber-300">
                  <ArrowUpCircle className="h-4 w-4 shrink-0 text-amber-400" />
                  <span>
                    New release <strong>v{updateInfo?.latest_version}</strong> is ready to install!
                  </span>
                </div>
                {updateInfo?.release_name && (
                  <p className="font-semibold text-[var(--text-primary)]">
                    {updateInfo.release_name}
                  </p>
                )}
                {updateInfo?.release_notes && (
                  <p className="line-clamp-2 text-[var(--text-secondary)] font-mono text-[11px]">
                    {updateInfo.release_notes}
                  </p>
                )}
              </div>
              <div className="flex items-center gap-2 shrink-0">
                <Button
                  variant="primary"
                  size="sm"
                  disabled={applyingUpdate}
                  onClick={() => void handleApplyUpdate()}
                  className="bg-amber-600 hover:bg-amber-500 text-white shadow-xs font-medium"
                >
                  {applyingUpdate ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      Updating…
                    </>
                  ) : (
                    <>
                      <ArrowUpCircle className="h-3.5 w-3.5" />
                      One-Click Update
                    </>
                  )}
                </Button>
                {updateInfo?.html_url && (
                  <a
                    href={updateInfo.html_url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-3 py-1.5 text-xs font-medium text-[var(--text-primary)] hover:bg-[var(--bg-hover)] transition-colors"
                  >
                    Release Notes
                    <ExternalLink className="h-3 w-3 opacity-70" />
                  </a>
                )}
              </div>
            </div>

            {updateLog && (
              <div className="mt-3 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-2.5 font-mono text-[11px] text-[var(--text-muted)] max-h-36 overflow-y-auto whitespace-pre-wrap">
                {updateLog}
              </div>
            )}
          </div>
        )}

        <div className="divide-y divide-[var(--border-subtle)]/60">
          {/* Row 1: Upstream GitHub Repo */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div className="flex items-center gap-3.5 min-w-0">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[var(--text-primary)]">
                <Github className="h-5 w-5" />
              </div>
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-[var(--text-primary)]">
                    GitHub Repository
                  </span>
                  <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5 text-[11px] font-mono text-[var(--text-secondary)]">
                    Mushfiqtaief/Tesie
                  </span>
                </div>
                <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                  Official upstream source code, documentation, releases, and issue tracker.
                </p>
              </div>
            </div>
            <a
              href={upstreamUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-4 py-2 text-xs font-medium text-[var(--text-primary)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-hover)] transition-colors"
            >
              <Github className="h-4 w-4" />
              View Upstream Repo
              <ExternalLink className="h-3.5 w-3.5 opacity-60" />
            </a>
          </div>

          {/* Row 2: Version Checker & Manual Update */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div>
              <div className="text-sm font-medium text-[var(--text-primary)]">
                Version &amp; Update Checker
              </div>
              <div className="mt-0.5 text-xs text-[var(--text-muted)]">
                Current installed build is <strong className="font-mono text-[var(--text-secondary)]">v{currentVersion}</strong>.
                {updateInfo?.published_at && (
                  <span className="ml-1.5">
                    Latest check: {new Date(updateInfo.published_at).toLocaleDateString()}
                  </span>
                )}
              </div>
            </div>
            <div className="flex items-center gap-2.5">
              <Button
                variant="secondary"
                size="sm"
                disabled={checkingUpdate || applyingUpdate}
                onClick={() => void handleCheckUpdate(false)}
              >
                <RefreshCw className={`h-3.5 w-3.5 ${checkingUpdate ? "animate-spin" : ""}`} />
                {checkingUpdate ? "Checking…" : "Check for Updates"}
              </Button>
              {hasUpdate && (
                <Button
                  variant="primary"
                  size="sm"
                  disabled={applyingUpdate}
                  onClick={() => void handleApplyUpdate()}
                >
                  <ArrowUpCircle className="h-3.5 w-3.5" />
                  {applyingUpdate ? "Updating…" : "One-Click Update"}
                </Button>
              )}
            </div>
          </div>
        </div>
      </section>

      {/* ──────────────────────────────────────────────────────────── */}
      {/* Section 2: Routing, Failover & Latency Policy                */}
      {/* ──────────────────────────────────────────────────────────── */}
      <section className="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-xs">
        <div className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 px-6 py-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold tracking-tight text-[var(--text-primary)]">
                Routing &amp; Failover Policy
              </h2>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Control load balancing resilience across proxy pools and set background health intervals.
              </p>
            </div>
            <StatusPill tone={data?.settings.exhaustive_failover ? "success" : "warning"} dot>
              {data?.settings.exhaustive_failover ? "Try to the Last" : "Direct Error"}
            </StatusPill>
          </div>
        </div>

        <div className="divide-y divide-[var(--border-subtle)]/60">
          {/* Row 1: Failover Mode Toggle */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div className="max-w-md">
              <div className="text-sm font-medium text-[var(--text-primary)]">
                Quota &amp; Rate-Limit Failover Mode
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)] leading-relaxed">
                {data?.settings.exhaustive_failover
                  ? "Load balances and retries across every ready proxy IP (OpenCode) and operational account (Antigravity). Client only receives an error if all units fail."
                  : "Returns upstream quota and rate limit errors immediately on the first attempt without trying backup nodes."}
              </p>
            </div>
            <div className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-1">
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
                Direct error
              </button>
            </div>
          </div>

          {/* Row 2: Proxy Health Interval */}
          <form
            onSubmit={handleSaveHealthInterval}
            className="flex flex-wrap items-center justify-between gap-4 px-6 py-4"
          >
            <div>
              <div className="text-sm font-medium text-[var(--text-primary)]">
                TesieCore Health Check Cadence
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Interval in seconds for background proxy latency verification (5–3600s).
              </p>
            </div>
            <div className="flex items-center gap-2">
              <input
                type="number"
                min={5}
                max={3600}
                value={healthIntervalInput}
                onChange={(e) => setHealthIntervalInput(e.target.value)}
                className={`${INPUT_CLASS} w-24 text-right font-mono py-1.5 text-xs`}
              />
              <span className="text-xs text-[var(--text-muted)] font-mono">sec</span>
              <Button type="submit" variant="secondary" size="sm" disabled={savingInterval}>
                {savingInterval ? "Saving…" : "Save"}
              </Button>
            </div>
          </form>

          {/* Row 3: Automated Context-Window Compacting Info */}
          <div className="flex items-start gap-3 px-6 py-3.5 bg-[var(--bg-elevated)]/20 text-xs text-[var(--text-secondary)]">
            <Sparkles className="h-4 w-4 text-[var(--accent)] shrink-0 mt-0.5" />
            <div className="leading-relaxed">
              <strong className="text-[var(--text-primary)] font-medium">Automatic Context-Window Compacting:</strong> Context limit exceeded errors (<code className="font-mono text-[11px]">context_length_exceeded</code> / prompt overflow) always fast-fail directly with clean HTTP 400 so AI tools (OpenCode, Claude Code, Aider) automatically compress chat history without exhausting proxy retries.
            </div>
          </div>

          {/* Row 4: Gateway Response & Prompt Caching */}
          <div className="px-6 py-4">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="max-w-md">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-[var(--text-primary)]">
                    Two-Tier Response &amp; Prompt Caching
                  </span>
                  <StatusPill tone={data?.settings.response_cache_enabled ? "success" : "neutral"} dot>
                    {data?.settings.response_cache_enabled ? "L1/L2 Active" : "Disabled"}
                  </StatusPill>
                </div>
                <p className="mt-0.5 text-xs text-[var(--text-muted)] leading-relaxed">
                  Caches deterministic responses in-memory and SQLite. Repeated queries from coding agents (Aider, Claude Code, OpenCode) return in ~2ms with 0 upstream tokens consumed.
                </p>
              </div>

              <div className="flex items-center gap-2.5">
                <div className="flex items-center gap-1.5 text-xs">
                  <span className="text-[var(--text-muted)]">TTL:</span>
                  <select
                    disabled={!data?.settings.response_cache_enabled}
                    value={data?.settings.response_cache_ttl_sec ?? 86400}
                    onChange={(e) => void handleChangeCacheTtl(Number(e.target.value))}
                    className={`${INPUT_CLASS} py-1 px-2.5 text-xs font-mono`}
                  >
                    <option value={3600}>1 Hour</option>
                    <option value={21600}>6 Hours</option>
                    <option value={86400}>24 Hours</option>
                    <option value={604800}>7 Days</option>
                  </select>
                </div>

                <Button
                  variant={data?.settings.response_cache_enabled ? "secondary" : "primary"}
                  size="sm"
                  disabled={togglingCache}
                  onClick={() => void handleToggleCache(!data?.settings.response_cache_enabled)}
                >
                  <Zap className="h-3.5 w-3.5" />
                  {data?.settings.response_cache_enabled ? "Disable Cache" : "Enable Cache"}
                </Button>

                <Button
                  variant="secondary"
                  size="sm"
                  disabled={purgingCache || (cacheStats?.total_entries ?? 0) === 0}
                  onClick={() => void handlePurgeCache()}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {purgingCache ? "Purging…" : "Purge Cache"}
                </Button>
              </div>
            </div>

            {/* Cache Metrics Bar */}
            {data?.settings.response_cache_enabled && cacheStats && (
              <div className="mt-3.5 grid grid-cols-2 sm:grid-cols-4 gap-2.5 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-3 text-xs">
                <div>
                  <div className="text-[11px] text-[var(--text-muted)]">Cached Entries</div>
                  <div className="text-sm font-semibold font-mono text-[var(--text-primary)]">
                    {cacheStats.total_entries.toLocaleString()}
                  </div>
                </div>
                <div>
                  <div className="text-[11px] text-[var(--text-muted)]">Hits / Misses</div>
                  <div className="text-sm font-semibold font-mono text-[var(--text-primary)]">
                    {cacheStats.hit_count.toLocaleString()}{" "}
                    <span className="text-[var(--text-muted)] font-normal text-xs">
                      / {cacheStats.miss_count.toLocaleString()}
                    </span>
                  </div>
                </div>
                <div>
                  <div className="text-[11px] text-[var(--text-muted)]">Hit Rate</div>
                  <div className="text-sm font-semibold font-mono text-[var(--success)]">
                    {cacheStats.hit_count + cacheStats.miss_count > 0
                      ? `${Math.round((cacheStats.hit_count / (cacheStats.hit_count + cacheStats.miss_count)) * 100)}%`
                      : "0%"}
                  </div>
                </div>
                <div>
                  <div className="text-[11px] text-[var(--text-muted)]">Tokens Saved</div>
                  <div className="text-sm font-semibold font-mono text-[var(--accent)]">
                    {cacheStats.tokens_saved.toLocaleString()}
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      </section>

      {/* ──────────────────────────────────────────────────────────── */}
      {/* Section 3: System, Startup & Security                        */}
      {/* ──────────────────────────────────────────────────────────── */}
      <section className="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-xs">
        <div className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 px-6 py-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold tracking-tight text-[var(--text-primary)]">
                System, Startup &amp; Security
              </h2>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Configure Windows background startup, safe database directory, and dashboard protection.
              </p>
            </div>
            <StatusPill tone={data?.system.tray_active ? "success" : "neutral"} dot>
              {data?.system.tray_active ? "System Tray Active" : "Background Daemon"}
            </StatusPill>
          </div>
        </div>

        <div className="divide-y divide-[var(--border-subtle)]/60">
          {/* Row 1: Start on Windows Boot */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div>
              <div className="text-sm font-medium text-[var(--text-primary)]">
                Start on Windows Boot
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Automatically launches Tesie silently in your Windows System Tray on login.
              </p>
            </div>
            <Button
              variant={data?.system.auto_start_enabled ? "primary" : "secondary"}
              size="sm"
              disabled={togglingAutostart}
              onClick={() => void handleToggleAutostart()}
            >
              <Power className="h-3.5 w-3.5" />
              {data?.system.auto_start_enabled ? "Enabled (ON)" : "Disabled (OFF)"}
            </Button>
          </div>

          {/* Row 2: Safe Data Path & SQLite Storage */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div className="min-w-0 max-w-md">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium text-[var(--text-primary)]">
                  Safe Data Path &amp; Database
                </span>
                <span className="rounded bg-[var(--bg-elevated)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-muted)]">
                  DB: {formatBytes(data?.system.db_size_bytes ?? 0)}
                </span>
              </div>
              <p className="mt-0.5 truncate font-mono text-xs text-[var(--text-muted)]">
                {data?.system.data_dir || "—"}
              </p>
            </div>
            <Button variant="secondary" size="sm" onClick={() => void handleCopyDataDir()}>
              {copiedPath ? (
                <Check className="h-3.5 w-3.5 text-[var(--success)]" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
              {copiedPath ? "Copied" : "Copy Path"}
            </Button>
          </div>

          {/* Row 3: Dashboard Access Lock */}
          <div className="px-6 py-4 space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div>
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-[var(--text-primary)]">
                    Dashboard Access Lock
                  </span>
                  {data?.security.protection_enabled ? (
                    <StatusPill tone="warning" dot>
                      <Lock className="h-3 w-3" />
                      Password Enabled
                    </StatusPill>
                  ) : (
                    <StatusPill tone="success" dot>
                      <LockOpen className="h-3 w-3" />
                      Open (Localhost)
                    </StatusPill>
                  )}
                </div>
                <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                  Tesie runs on <code className="font-mono">127.0.0.1</code>. Set a password only if your machine is exposed on a shared LAN.
                </p>
              </div>
            </div>

            {data?.security.env_admin_token_active ? (
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3 text-xs text-[var(--text-secondary)]">
                Locked via <code className="font-mono font-semibold">ADMIN_TOKEN</code> environment variable. Unset it to manage passwords from this UI.
              </div>
            ) : (
              <form onSubmit={handleSavePassword} className="flex flex-wrap items-center gap-2 pt-1">
                <input
                  type="password"
                  placeholder={
                    data?.security.has_admin_password
                      ? "Enter new password…"
                      : "Set dashboard password (optional)"
                  }
                  value={passwordInput}
                  onChange={(e) => setPasswordInput(e.target.value)}
                  className={`${INPUT_CLASS} w-64 py-1.5 text-xs font-mono`}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="sm"
                  disabled={!passwordInput.trim() || savingPassword}
                >
                  <ShieldCheck className="h-3.5 w-3.5" />
                  {savingPassword ? "Saving…" : "Save Password"}
                </Button>
                {data?.security.has_admin_password && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    disabled={savingPassword}
                    onClick={() => void handleClearPassword()}
                  >
                    Remove Lock
                  </Button>
                )}
              </form>
            )}
          </div>
        </div>
      </section>

      {/* ──────────────────────────────────────────────────────────── */}
      {/* Section 4: Configuration Backup & Maintenance                */}
      {/* ──────────────────────────────────────────────────────────── */}
      <section className="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-xs">
        <div className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 px-6 py-4">
          <h2 className="text-sm font-semibold tracking-tight text-[var(--text-primary)]">
            Backup &amp; Maintenance
          </h2>
          <p className="mt-0.5 text-xs text-[var(--text-muted)]">
            Export portable configuration snapshots, restore backups, or reset internal queues.
          </p>
        </div>

        <div className="divide-y divide-[var(--border-subtle)]/60">
          {/* Row 1: Export Snapshot */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div>
              <div className="text-sm font-medium text-[var(--text-primary)]">
                Export Configuration
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Downloads a portable JSON file containing your proxies, custom providers, combos, and keys.
              </p>
              <label className="mt-2 flex cursor-pointer items-center gap-2 text-xs text-[var(--text-secondary)] select-none">
                <input
                  type="checkbox"
                  checked={includeSecrets}
                  onChange={(e) => setIncludeSecrets(e.target.checked)}
                  className="h-3.5 w-3.5 rounded border-[var(--border-strong)] accent-[var(--accent)]"
                />
                Include API keys, OAuth tokens, and proxy passwords
              </label>
            </div>
            <Button
              variant="secondary"
              size="sm"
              disabled={exporting}
              onClick={() => void handleExportBackup()}
            >
              <Download className="h-3.5 w-3.5" />
              {exporting ? "Exporting…" : "Export JSON"}
            </Button>
          </div>

          {/* Row 2: Restore from Backup */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div>
              <div className="text-sm font-medium text-[var(--text-primary)]">
                Restore from Backup
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Upload a previously saved <code className="font-mono text-[11px]">tesie-backup-*.json</code> file.
              </p>
              <div className="mt-2 flex items-center gap-4 text-xs text-[var(--text-secondary)]">
                <label className="flex cursor-pointer items-center gap-1.5 select-none">
                  <input
                    type="radio"
                    name="import-mode"
                    checked={!replaceOnImport}
                    onChange={() => setReplaceOnImport(false)}
                    className="accent-[var(--accent)]"
                  />
                  Merge items
                </label>
                <label className="flex cursor-pointer items-center gap-1.5 select-none">
                  <input
                    type="radio"
                    name="import-mode"
                    checked={replaceOnImport}
                    onChange={() => setReplaceOnImport(true)}
                    className="accent-[var(--accent)]"
                  />
                  Replace all
                </label>
              </div>
            </div>
            <div>
              <Button
                variant="secondary"
                size="sm"
                disabled={importing}
                onClick={() => fileInputRef.current?.click()}
              >
                <Upload className="h-3.5 w-3.5" />
                {importing ? "Restoring…" : "Import JSON"}
              </Button>
              <input
                ref={fileInputRef}
                type="file"
                accept=".json,application/json"
                className="hidden"
                onChange={(e) => void handleFileSelected(e)}
              />
            </div>
          </div>

          {lastImportSummary && (
            <div className="bg-[var(--success-soft)] px-6 py-2.5 text-xs text-[var(--success)]">
              Restored {lastImportSummary.proxies} proxies, {lastImportSummary.custom_providers} custom providers, {lastImportSummary.combos} combos, and {lastImportSummary.api_keys} API keys.
            </div>
          )}

          {/* Row 3: Automatic SQLite Log Retention & Vacuum */}
          <div className="px-6 py-4">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="max-w-md">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-[var(--text-primary)]">
                    SQLite Log Retention &amp; Auto-Vacuum
                  </span>
                  <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5 text-[11px] font-mono text-[var(--text-secondary)]">
                    {formatBytes(data?.system.db_size_bytes ?? 0)} · {(data?.system.db_log_count ?? 0).toLocaleString()} logs
                  </span>
                </div>
                <p className="mt-0.5 text-xs text-[var(--text-muted)] leading-relaxed">
                  Automatically prunes historical request logs in the background and recovers freed disk space using SQLite incremental vacuum.
                </p>
              </div>

              <div className="flex flex-wrap items-center gap-3">
                <div className="flex items-center gap-1.5 text-xs">
                  <span className="text-[var(--text-muted)]">Keep:</span>
                  <select
                    value={data?.settings.log_retention_days ?? 0}
                    onChange={(e) => {
                      const val = Number(e.target.value);
                      void handleChangeLogRetention(val > 0 ? val : null);
                    }}
                    className={`${INPUT_CLASS} py-1 px-2.5 text-xs font-mono`}
                  >
                    <option value={7}>7 Days</option>
                    <option value={14}>14 Days</option>
                    <option value={30}>30 Days</option>
                    <option value={90}>90 Days</option>
                    <option value={0}>Unlimited</option>
                  </select>
                </div>

                <div className="flex items-center gap-1.5 text-xs">
                  <span className="text-[var(--text-muted)]">Max logs:</span>
                  <select
                    value={data?.settings.log_max_entries ?? 0}
                    onChange={(e) => {
                      const val = Number(e.target.value);
                      void handleChangeMaxLogs(val > 0 ? val : null);
                    }}
                    className={`${INPUT_CLASS} py-1 px-2.5 text-xs font-mono`}
                  >
                    <option value={5000}>5,000</option>
                    <option value={10000}>10,000</option>
                    <option value={25000}>25,000</option>
                    <option value={50000}>50,000</option>
                    <option value={0}>Unlimited</option>
                  </select>
                </div>

                <Button
                  variant="secondary"
                  size="sm"
                  disabled={pruningLogs}
                  onClick={() => void handlePruneLogs()}
                >
                  <Database className="h-3.5 w-3.5" />
                  {pruningLogs ? "Vacuuming…" : "Prune & Vacuum"}
                </Button>
              </div>
            </div>
          </div>

          {/* Row 4: Maintenance Actions */}
          <div className="flex flex-wrap items-center justify-between gap-4 px-6 py-4">
            <div>
              <div className="text-sm font-medium text-[var(--text-primary)]">
                Maintenance &amp; Diagnostics
              </div>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                Reset cooling IP queues, clear historical request logs, or reset compression statistics.
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="secondary"
                size="sm"
                disabled={resettingCooldowns}
                onClick={() => void handleResetCooldowns()}
                title="Clears cooldown state for all proxy IPs and accounts"
              >
                <RotateCcw className={`h-3.5 w-3.5 ${resettingCooldowns ? "animate-spin" : "text-[var(--warning)]"}`} />
                {resettingCooldowns ? "Resetting…" : "Reset Cooldowns"}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={clearingLogs}
                onClick={() => void handleClearLogs()}
                title="Deletes request history records from database"
              >
                <Trash2 className="h-3.5 w-3.5 text-[var(--danger)]" />
                {clearingLogs ? "Clearing…" : "Clear Logs"}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={clearingTokenSaver}
                onClick={() => void handleClearTokenSaver()}
                title="Resets token savings counters"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${clearingTokenSaver ? "animate-spin" : ""}`} />
                Reset Telemetry
              </Button>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
});
