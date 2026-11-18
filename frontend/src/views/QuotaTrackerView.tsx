import { useEffect, useRef, useState } from "react";
import {
  AlertCircle,
  AlertTriangle,
  Check,
  Copy,
  Download,
  LayoutGrid,
  List,
  Loader2,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  ShieldAlert,
  SlidersHorizontal,
  UserCheck,
} from "lucide-react";
import { ProviderIcon } from "../components/ProviderIcon";
import { ProbeSettingsBar } from "../components/ProbeSettingsBar";
import { QuotaPageSkeleton } from "../components/Skeleton";
import {
  Button,
  EmptyState,
  INPUT_CLASS,
  MeterBar,
  Modal,
  SlideOverDrawer,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  AntigravityAccountView,
  AntigravityPoolWindow,
  IpQuotaWithProxies,
  ProviderCatalogItem,
  ProviderQuotaSummary,
  ProxyWithQuota,
  QuotaProbeStatus,
  UpdateIpQuotaLimitsPayload,
  UpdateProviderConfigPayload,
} from "../types";
import {
  formatCompactNumber,
  formatCooldownReason,
  formatCountdown,
  formatTimeAgo,
  getProviderLogo,
  proxyExportUrl,
  proxyModeLabel,
  routingStrategyLabel,
} from "../utils/format";

type IpPoolFilter = "all" | "active" | "cooling";
type AgAccountFilter = "all" | "cooling" | "ready" | "disabled";
type QuotaViewMode = "table" | "list";
type ExportScope = "ready" | "working" | "all";

const VIEW_MODE_STORAGE_KEY = "tesie_quota_view_mode";
const LEGACY_VIEW_MODE_STORAGE_KEY = "tesroute_quota_view_mode";

function readInitialViewMode(): QuotaViewMode {
  try {
    const saved =
      localStorage.getItem(VIEW_MODE_STORAGE_KEY) ||
      localStorage.getItem(LEGACY_VIEW_MODE_STORAGE_KEY);
    if (saved === "list") return "list";
    if (saved === "table" || saved === "cards") return "table";
  } catch {
    // ignore
  }
  return "table";
}

function agPctColor(pct: number): string {
  if (pct <= 10) return "var(--danger)";
  if (pct <= 35) return "var(--warning)";
  return "var(--success)";
}

function QuotaRing({ pct, size = 18 }: { pct: number; size?: number }) {
  const clamped = Math.max(0, Math.min(100, Math.round(pct)));
  const r = 7;
  const c = 2 * Math.PI * r;
  const offset = c * (1 - clamped / 100);
  const stroke = agPctColor(clamped);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      className="shrink-0 -rotate-90"
      aria-hidden="true"
    >
      <circle
        cx="10"
        cy="10"
        r={r}
        fill="none"
        stroke="var(--border-subtle)"
        strokeWidth="2.4"
      />
      <circle
        cx="10"
        cy="10"
        r={r}
        fill="none"
        stroke={stroke}
        strokeWidth="2.4"
        strokeLinecap="round"
        strokeDasharray={c}
        strokeDashoffset={offset}
      />
    </svg>
  );
}

function isAgAccountBanned(acct: AntigravityAccountView): boolean {
  return (
    acct.status === "banned" ||
    (!!acct.last_error &&
      /violation of terms|disabled in this account|account_disabled_tos/i.test(
        acct.last_error,
      ))
  );
}

function isAgAccountCooling(acct: AntigravityAccountView, nowMs: number): boolean {
  if (!acct.enabled || isAgAccountBanned(acct)) return false;
  if (acct.status === "cooldown" && (!acct.cooldown_until || acct.cooldown_until > nowMs)) {
    return true;
  }
  if ((acct.model_cooldowns ?? []).some((mc) => mc.cooldown_until > nowMs)) {
    return true;
  }
  return (acct.quota?.models ?? []).some(
    (mq) => !!mq.failure_cooldown_until && mq.failure_cooldown_until > nowMs,
  );
}

function splitAgMainPools(acct: AntigravityAccountView): {
  geminiWeekly?: AntigravityPoolWindow;
  gemini5h?: AntigravityPoolWindow;
  claudeWeekly?: AntigravityPoolWindow;
  claude5h?: AntigravityPoolWindow;
} {
  const unlockedPools = (acct.quota?.pools ?? []).filter((p) => p.unlocked !== false);
  const pick = (groupNeedles: string[], isWeekly: boolean) =>
    unlockedPools.find((p) => {
      const gn = `${p.group_name} ${p.key}`.toLowerCase();
      const win = `${p.window_label} ${p.key}`.toLowerCase();
      const groupMatch = groupNeedles.some((n) => gn.includes(n));
      const winMatch = isWeekly
        ? win.includes("week")
        : win.includes("5h") || win.includes("session");
      return groupMatch && winMatch;
    });
  return {
    geminiWeekly: pick(["gemini"], true),
    gemini5h: pick(["gemini"], false),
    claudeWeekly: pick(["claude", "gpt"], true),
    claude5h: pick(["claude", "gpt"], false),
  };
}

export function QuotaTrackerView({
  providers = [],
  providerQuotas,
  ipQuotas,
  proxies = [],
  antigravityAccounts,
  initialProviderId,
  lastQuotaSyncAt,
  quotaProbeStatus,
  probingQuotas,
  loading,
  onNotify,
  onOpenProviderSettings,
  nowMs,
  onRefreshQuotas,
  onProbeQuotasNow,
  onResetIpQuota,
  onResetAllCooling,
  onUpdateIpQuotaLimits,
  onUpdateProviderConfig,
  onRefreshAntigravityQuotas,
  onRefreshSingleAntigravityQuota,
  onUpdateAccountConnection,
}: {
  providers?: ProviderCatalogItem[];
  providerQuotas: ProviderQuotaSummary[];
  ipQuotas: IpQuotaWithProxies[];
  proxies?: ProxyWithQuota[];
  antigravityAccounts?: AntigravityAccountView[];
  initialProviderId?: string;
  lastQuotaSyncAt: number;
  quotaProbeStatus?: QuotaProbeStatus | null;
  probingQuotas?: boolean;
  loading: boolean;
  onNotify?: (message: string, isError?: boolean) => void;
  onOpenProviderSettings: (providerId: string) => void;
  nowMs: number;
  onRefreshQuotas: () => Promise<void>;
  onProbeQuotasNow?: () => Promise<void>;
  onResetIpQuota: (ipKey: string) => Promise<void>;
  onResetAllCooling?: () => Promise<void>;
  onUpdateIpQuotaLimits: (ipKey: string, payload: UpdateIpQuotaLimitsPayload) => Promise<void>;
  onUpdateProviderConfig?: (
    id: string,
    payload: UpdateProviderConfigPayload,
  ) => Promise<void>;
  onRefreshAntigravityQuotas?: () => Promise<void>;
  onRefreshSingleAntigravityQuota?: (id: string) => Promise<void>;
  onUpdateAccountConnection?: (
    id: string,
    payload: { name?: string; enabled?: boolean; reset_cooldown?: boolean },
  ) => Promise<void>;
}) {
  const [selectedProviderId, setSelectedProviderId] = useState<string>(
    initialProviderId || "opencode",
  );
  const [search, setSearch] = useState("");
  const [ipFilter, setIpFilter] = useState<IpPoolFilter>("all");
  const [agFilter, setAgFilter] = useState<AgAccountFilter>("all");
  const [viewMode, setViewMode] = useState<QuotaViewMode>(() => readInitialViewMode());
  const [showAllBuckets, setShowAllBuckets] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [verifyingIpKey, setVerifyingIpKey] = useState<string | null>(null);
  const [verifyingAll, setVerifyingAll] = useState(false);
  const [refreshingAgAll, setRefreshingAgAll] = useState(false);
  const [refreshingAgOne, setRefreshingAgOne] = useState<string | null>(null);
  const [togglingAgId, setTogglingAgId] = useState<string | null>(null);

  // Proxy export drawer
  const [exportOpen, setExportOpen] = useState(false);
  const [exportScope, setExportScope] = useState<ExportScope>("ready");
  const [exportCopied, setExportCopied] = useState(false);
  const exportTextareaRef = useRef<HTMLTextAreaElement>(null);

  // SlideOver limit editor for IP quota buckets
  const [editingIp, setEditingIp] = useState<IpQuotaWithProxies | null>(null);
  const [windowMinutes, setWindowMinutes] = useState("60");
  const [maxReqs, setMaxReqs] = useState("");
  const [maxTokens, setMaxTokens] = useState("");
  const [savingLimits, setSavingLimits] = useState(false);

  useEffect(() => {
    if (initialProviderId) {
      setSelectedProviderId(initialProviderId);
    }
  }, [initialProviderId]);

  useEffect(() => {
    setShowAllBuckets(false);
  }, [search, ipFilter, agFilter, selectedProviderId]);

  function handleChangeViewMode(next: QuotaViewMode) {
    setViewMode(next);
    try {
      localStorage.setItem(VIEW_MODE_STORAGE_KEY, next);
    } catch {
      // ignore
    }
  }

  const agAccounts = antigravityAccounts ?? [];
  const hasAntigravityAccounts = agAccounts.length > 0;

  useEffect(() => {
    if (!hasAntigravityAccounts && selectedProviderId === "antigravity") {
      setSelectedProviderId("opencode");
    }
  }, [hasAntigravityAccounts, selectedProviderId]);

  if (loading && providerQuotas.length === 0 && ipQuotas.length === 0) {
    return <QuotaPageSkeleton />;
  }

  const opencodeSummary: ProviderQuotaSummary =
    providerQuotas.find((p) => p.id === "opencode") ?? {
      id: "opencode",
      name: "OpenCode Free",
      description:
        "Passive-first per-IP quota tracking across the TesieCore global proxy pool with Round-Robin selection and verified cooldown recovery.",
      strategy: "ip_pool",
      unit_label: "Exit IP",
      active_count: ipQuotas.filter((q) => q.status !== "cooldown").length,
      cooling_count: ipQuotas.filter((q) => q.status === "cooldown").length,
    };

  const antigravitySummary: ProviderQuotaSummary =
    providerQuotas.find((p) => p.id === "antigravity") ?? {
      id: "antigravity",
      name: "Antigravity",
      description:
        "Per-Account & Per-Model Cloud Code quotas with disjoint Smart Proxy Partitioning and Sticky Fallback.",
      strategy: "account",
      unit_label: "Google Account",
      active_count: agAccounts.filter((a) => a.enabled && a.status === "ready").length,
      cooling_count: agAccounts.filter((a) => a.enabled && a.status === "cooldown").length,
    };

  const providerTabs = hasAntigravityAccounts
    ? [opencodeSummary, antigravitySummary]
    : [opencodeSummary];
  const isAntigravity = hasAntigravityAccounts && selectedProviderId === "antigravity";
  const activeProvider = isAntigravity ? antigravitySummary : opencodeSummary;
  const activeCatalogProvider = providers.find((p) => p.id === activeProvider.id);
  const activeProbeEnabled =
    activeCatalogProvider?.config.probe_enabled ??
    activeProvider.probe_enabled ??
    true;
  const activeProbeIntervalSec =
    activeCatalogProvider?.config.probe_interval_sec ??
    activeProvider.probe_interval_sec ??
    quotaProbeStatus?.interval_sec ??
    30;

  const isProbingLive = Boolean(probingQuotas || quotaProbeStatus?.probe_running);

  async function handleManualSync() {
    setSyncing(true);
    try {
      await onRefreshQuotas();
    } finally {
      setSyncing(false);
    }
  }

  async function handleVerifyResetSingle(ipKey: string) {
    setVerifyingIpKey(ipKey);
    try {
      await onResetIpQuota(ipKey);
    } finally {
      setVerifyingIpKey(null);
    }
  }

  async function handleVerifyResetAll() {
    if (!onResetAllCooling) return;
    setVerifyingAll(true);
    try {
      await onResetAllCooling();
    } finally {
      setVerifyingAll(false);
    }
  }

  async function handleRefreshAgAllClick() {
    if (!onRefreshAntigravityQuotas) return;
    setRefreshingAgAll(true);
    try {
      await onRefreshAntigravityQuotas();
    } finally {
      setRefreshingAgAll(false);
    }
  }

  async function handleRefreshAgSingleClick(id: string) {
    if (!onRefreshSingleAntigravityQuota) return;
    setRefreshingAgOne(id);
    try {
      await onRefreshSingleAntigravityQuota(id);
    } finally {
      setRefreshingAgOne(null);
    }
  }

  async function handleToggleAgEnabledClick(acct: AntigravityAccountView) {
    if (!onUpdateAccountConnection) return;
    setTogglingAgId(acct.id);
    try {
      await onUpdateAccountConnection(acct.id, { enabled: !acct.enabled });
    } finally {
      setTogglingAgId(null);
    }
  }

  async function handleResetAgCooldownClick(id: string) {
    if (!onUpdateAccountConnection) return;
    setTogglingAgId(id);
    try {
      await onUpdateAccountConnection(id, { reset_cooldown: true });
    } finally {
      setTogglingAgId(null);
    }
  }

  async function handleCopyExport() {
    const text = exportLines.join("\n");
    const flashCopied = () => {
      setExportCopied(true);
      setTimeout(() => setExportCopied(false), 1800);
    };
    try {
      await navigator.clipboard.writeText(text);
      flashCopied();
      onNotify?.(`Copied ${exportLines.length} proxy URLs`);
      return;
    } catch {
      // Clipboard API can be blocked (insecure origin, denied permission) — fall through.
    }
    const ta = exportTextareaRef.current;
    if (ta) {
      ta.focus();
      ta.select();
      if (document.execCommand("copy")) {
        flashCopied();
        onNotify?.(`Copied ${exportLines.length} proxy URLs`);
        return;
      }
    }
    onNotify?.("Clipboard blocked — text is selected, press ⌘/Ctrl+C", true);
  }

  function handleDownloadExport() {
    const blob = new Blob([`${exportLines.join("\n")}\n`], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const stamp = new Date().toISOString().slice(0, 10);
    const a = document.createElement("a");
    a.href = url;
    a.download = `tesie-proxies-${stamp}.txt`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    onNotify?.(`Exported ${exportLines.length} proxy URLs`);
  }

  function openLimitsDrawer(q: IpQuotaWithProxies) {
    setEditingIp(q);
    setWindowMinutes(String(Math.max(1, Math.round(q.window_duration_ms / 60_000))));
    setMaxReqs(q.max_requests_per_window ? String(q.max_requests_per_window) : "");
    setMaxTokens(q.max_tokens_per_window ? String(q.max_tokens_per_window) : "");
  }

  async function handleLimitsSave(e: React.FormEvent) {
    e.preventDefault();
    if (!editingIp) return;
    setSavingLimits(true);
    try {
      const mins = Math.max(1, Number(windowMinutes) || 60);
      await onUpdateIpQuotaLimits(editingIp.ip_key, {
        window_duration_ms: mins * 60_000,
        max_requests_per_window: maxReqs.trim() ? Number(maxReqs) : null,
        max_tokens_per_window: maxTokens.trim() ? Number(maxTokens) : null,
      });
      setEditingIp(null);
    } finally {
      setSavingLimits(false);
    }
  }

  const qLower = search.trim().toLowerCase();

  const exportableProxies = proxies.filter((p) => !p.is_direct && !!proxyExportUrl(p));
  const unexportableCount = proxies.filter((p) => !p.is_direct).length - exportableProxies.length;
  const healthyProxies = exportableProxies.filter((p) => p.enabled && p.alive);

  // Proxies whose exit IP is tagged cooldown by OpenCode Free quota tracking.
  const coolingIpKeys = new Set(
    ipQuotas.filter((q) => q.status === "cooldown").map((q) => q.ip_key),
  );
  const readyProxies = exportableProxies.filter(
    (p) => !coolingIpKeys.has(p.ip_key) && p.enabled && p.alive,
  );

  const exportScopes = {
    ready: { proxies: readyProxies, label: `Ready only (${readyProxies.length})` },
    working: { proxies: healthyProxies, label: `Working only (${healthyProxies.length})` },
    all: { proxies: exportableProxies, label: `Everything (${exportableProxies.length})` },
  } as const;

  const exportLines = Array.from(
    new Set(
      exportScopes[exportScope]
        .proxies.map(proxyExportUrl)
        .filter((line): line is string => !!line),
    ),
  );

  // Cooling stays true as long as `q.status === "cooldown"` (only cleared when real OpenCode usage is verified!)
  const realCoolingCount = ipQuotas.filter((q) => q.status === "cooldown").length;
  const realReadyCount = Math.max(0, ipQuotas.length - realCoolingCount);

  const filteredIpQuotas = ipQuotas
    .filter((q) => {
      const isCooling = q.status === "cooldown";
      if (ipFilter === "active" && isCooling) return false;
      if (ipFilter === "cooling" && !isCooling) return false;
      if (!qLower) return true;
      return (
        q.ip_key.toLowerCase().includes(qLower) ||
        q.proxies.some((p) => p.toLowerCase().includes(qLower)) ||
        formatCooldownReason(q.last_error).toLowerCase().includes(qLower)
      );
    })
    .sort((a, b) => {
      const aCool = a.status === "cooldown" ? 1 : 0;
      const bCool = b.status === "cooldown" ? 1 : 0;
      if (aCool !== bCool) return bCool - aCool;
      const aUsed = a.last_used_at ?? 0;
      const bUsed = b.last_used_at ?? 0;
      if (aUsed !== bUsed) return bUsed - aUsed;
      return a.ip_key.localeCompare(b.ip_key);
    });

  const visibleIpQuotas = showAllBuckets ? filteredIpQuotas : filteredIpQuotas.slice(0, 48);

  const agDisabledCount = agAccounts.filter(
    (acct) => !acct.enabled || isAgAccountBanned(acct),
  ).length;
  const agCoolingCount = agAccounts.filter((acct) => isAgAccountCooling(acct, nowMs)).length;
  const agReadyCount = Math.max(0, agAccounts.length - agDisabledCount - agCoolingCount);

  const filteredAgAccounts = agAccounts.filter((acct) => {
    const disabled = !acct.enabled || isAgAccountBanned(acct);
    const cooling = isAgAccountCooling(acct, nowMs);
    if (agFilter === "cooling" && !cooling) return false;
    if (agFilter === "ready" && (disabled || cooling)) return false;
    if (agFilter === "disabled" && !disabled) return false;

    if (!qLower) return true;
    const matchAccount =
      acct.name.toLowerCase().includes(qLower) ||
      (acct.email || "").toLowerCase().includes(qLower) ||
      (acct.quota?.plan || "").toLowerCase().includes(qLower) ||
      (acct.partition?.primary_proxy?.name || "").toLowerCase().includes(qLower);
    const matchModel = (acct.quota?.models ?? []).some(
      (m) =>
        m.model.toLowerCase().includes(qLower) ||
        (m.display_name || "").toLowerCase().includes(qLower),
    );
    const matchPool = (acct.quota?.pools ?? []).some((p) =>
      p.group_name.toLowerCase().includes(qLower),
    );
    return matchAccount || matchModel || matchPool;
  });

  const scopedCatalogProvider = providers.find((p) => p.id === activeProvider.id);
  const scopedSelectedCount = scopedCatalogProvider?.config.selected_proxy_ids?.length;
  const scopedProxyLabel = proxyModeLabel(
    scopedCatalogProvider?.config.proxy_mode ?? activeProvider.proxy_mode,
    scopedSelectedCount,
  );

  return (
    <div className="space-y-4">
      {/* Unified Quota Command Bar (Provider + Status + Probe Controls + Search/Layout) */}
      <section className="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]">
        {/* Row 1: Provider Switcher / Status + Probe & Recovery Actions */}
        <div className="flex flex-wrap items-center justify-between gap-3 px-5 py-3.5">
          <div className="flex flex-wrap items-center gap-3.5">
            {providerTabs.length > 1 ? (
              <div
                role="tablist"
                aria-label="Quota provider"
                className="inline-flex items-center gap-0.5 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-1"
              >
                {providerTabs.map((pt) => {
                  const selected = pt.id === activeProvider.id;
                  return (
                    <button
                      key={pt.id}
                      type="button"
                      role="tab"
                      aria-selected={selected}
                      onClick={() => {
                        setSelectedProviderId(pt.id);
                        setSearch("");
                      }}
                      className={`inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-[13px] font-medium whitespace-nowrap transition-colors ${
                        selected
                          ? "border border-[var(--border-subtle)] bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-2xs"
                          : "border border-transparent text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      <ProviderIcon src={getProviderLogo(pt.id)} name={pt.name} size={18} />
                      <span>{pt.name}</span>
                      <span className="font-mono text-[11px] text-[var(--text-muted)] tabular-nums">
                        {pt.active_count}
                      </span>
                    </button>
                  );
                })}
              </div>
            ) : (
              <div className="flex items-center gap-2.5">
                <ProviderIcon
                  src={getProviderLogo(activeProvider.id)}
                  name={activeProvider.name}
                  size={24}
                />
                <span className="text-[15px] font-semibold text-[var(--text-primary)]">
                  {activeProvider.name}
                </span>
              </div>
            )}

            <div className="flex flex-wrap items-center gap-2 text-[13px] text-[var(--text-muted)] tabular-nums">
              <span>
                <span className="font-mono font-semibold text-[var(--success)]">
                  {activeProvider.active_count}
                </span>{" "}
                ready
              </span>
              {(isAntigravity ? activeProvider.cooling_count : realCoolingCount) > 0 && (
                <>
                  <span>&middot;</span>
                  <span>
                    <span className="font-mono font-semibold text-[var(--warning)]">
                      {isAntigravity ? activeProvider.cooling_count : realCoolingCount}
                    </span>{" "}
                    {isAntigravity ? "cooling" : "queued"}
                  </span>
                </>
              )}
              <span>&middot;</span>
              <button
                type="button"
                onClick={() => onOpenProviderSettings(activeProvider.id)}
                className="font-medium text-[var(--text-secondary)] hover:text-[var(--accent)] hover:underline"
                title={`Configure ${activeProvider.name} routing and proxy settings`}
              >
                {isAntigravity
                  ? `${routingStrategyLabel(
                      activeCatalogProvider?.config.routing_strategy ??
                        activeProvider.routing_strategy ??
                        activeProvider.selection_strategy,
                    )} · Manage accounts`
                  : `${routingStrategyLabel(
                      activeCatalogProvider?.config.routing_strategy ??
                        activeProvider.routing_strategy ??
                        activeProvider.selection_strategy,
                    )} · ${scopedProxyLabel}`}
              </button>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {onUpdateProviderConfig && (
              <ProbeSettingsBar
                compact
                providerId={activeProvider.id}
                providerName={activeProvider.name}
                probeEnabled={activeProbeEnabled}
                probeIntervalSec={activeProbeIntervalSec}
                onUpdateConfig={(patch) => onUpdateProviderConfig(activeProvider.id, patch)}
                onManualProbe={
                  isAntigravity
                    ? agAccounts.length > 0 && onRefreshAntigravityQuotas
                      ? () => void handleRefreshAgAllClick()
                      : undefined
                    : onProbeQuotasNow
                      ? () => void onProbeQuotasNow()
                      : () => void handleManualSync()
                }
                probing={isAntigravity ? refreshingAgAll : isProbingLive || syncing}
                lastProbeAt={
                  isAntigravity
                    ? activeProvider.probe_status?.last_probe_at ??
                      agAccounts[0]?.quota?.fetched_at ??
                      null
                    : quotaProbeStatus?.last_probe_at ?? lastQuotaSyncAt
                }
                nowMs={nowMs}
              />
            )}

            <Button
              variant="secondary"
              size="sm"
              disabled={exportableProxies.length === 0}
              onClick={() => {
                setExportCopied(false);
                setExportOpen(true);
              }}
              title="Export proxy URLs for use in any other client"
            >
              <Download className="h-4 w-4" />
              Export
            </Button>

            {isAntigravity ? (
              <Button
                variant="primary"
                size="sm"
                onClick={() => onOpenProviderSettings("antigravity")}
              >
                <Plus className="h-4 w-4" />
                Connect account
              </Button>
            ) : (
              realCoolingCount > 0 &&
              onResetAllCooling && (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={verifyingAll}
                  onClick={() => void handleVerifyResetAll()}
                  title="Tests cooling IPs against OpenCode and clears cooldown only when usage is available"
                >
                  {verifyingAll ? (
                    <Loader2 className="h-4 w-4 animate-spin text-[var(--warning)]" />
                  ) : (
                    <RotateCcw className="h-4 w-4 text-[var(--warning)]" />
                  )}
                  {verifyingAll ? "Verifying…" : `Verify queued (${realCoolingCount})`}
                </Button>
              )
            )}
          </div>
        </div>

        {/* Row 2: Search, Status Filter & 4/line Table vs 2/line List Layout Switch */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 px-5 py-2.5">
          <div className="relative w-64 sm:w-80">
            <Search className="pointer-events-none absolute top-2.5 left-3 h-4 w-4 text-[var(--text-muted)]" />
            <input
              type="text"
              placeholder={
                isAntigravity
                  ? "Filter account, NODE, or model…"
                  : "Filter IP, NODE, or cooldown reason…"
              }
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className={`${INPUT_CLASS} py-1.5 pl-9 text-[13px]`}
            />
          </div>

          <div className="flex flex-wrap items-center gap-2.5">
            {isAntigravity ? (
              <div className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5">
                {(
                  [
                    { id: "all", label: `All (${agAccounts.length})` },
                    { id: "cooling", label: `Cooling (${agCoolingCount})` },
                    { id: "ready", label: `Ready (${agReadyCount})` },
                    ...(agDisabledCount > 0
                      ? [{ id: "disabled" as AgAccountFilter, label: `Disabled (${agDisabledCount})` }]
                      : []),
                  ] as Array<{ id: AgAccountFilter; label: string }>
                ).map((f) => (
                  <button
                    key={f.id}
                    type="button"
                    onClick={() => setAgFilter(f.id)}
                    className={`rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                      agFilter === f.id
                        ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                        : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    {f.label}
                  </button>
                ))}
              </div>
            ) : (
              <div className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5">
                {(
                  [
                    { id: "all", label: `All (${ipQuotas.length})` },
                    { id: "cooling", label: `Cooling (${realCoolingCount})` },
                    { id: "active", label: `Ready (${realReadyCount})` },
                  ] as Array<{ id: IpPoolFilter; label: string }>
                ).map((f) => (
                  <button
                    key={f.id}
                    type="button"
                    onClick={() => setIpFilter(f.id)}
                    className={`rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                      ipFilter === f.id
                        ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                        : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    {f.label}
                  </button>
                ))}
              </div>
            )}

            {/* View Toggle: Table (4/line) | List (2/line) */}
            <div
              className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5"
              role="group"
              aria-label="Quota view layout"
            >
              <button
                type="button"
                onClick={() => handleChangeViewMode("table")}
                title={isAntigravity ? "Grid cards view" : "Table view (4 items per line)"}
                className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                  viewMode === "table"
                    ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                }`}
              >
                <LayoutGrid className="h-4 w-4" />
                Table
              </button>
              <button
                type="button"
                onClick={() => handleChangeViewMode("list")}
                title={isAntigravity ? "Expanded row cards" : "List view (2 items per line)"}
                className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                  viewMode === "list"
                    ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                }`}
              >
                <List className="h-4 w-4" />
                List
              </button>
            </div>
          </div>
        </div>
      </section>

      {/* ================= ANTIGRAVITY PER-ACCOUNT QUOTA VIEW (SEPARATE ACCOUNT CARDS) ================= */}
      {isAntigravity ? (
        filteredAgAccounts.length === 0 ? (
          <EmptyState
            icon={<UserCheck className="h-6 w-6" />}
            title="No accounts match your filter"
            description="Clear your search filter or manage connected Google accounts on the Providers page."
          />
        ) : (
          <div
            key={`ag-${viewMode}`}
            className={
              viewMode === "table"
                ? "animate-tab-switch grid grid-cols-1 gap-4 xl:grid-cols-2"
                : "animate-tab-switch space-y-3.5"
            }
          >
            {filteredAgAccounts.map((acct, idx) => {
              const isBanned = isAgAccountBanned(acct);
              const cooling =
                acct.enabled &&
                !isBanned &&
                acct.status === "cooldown" &&
                !!acct.cooldown_until &&
                acct.cooldown_until > nowMs;
              const primaryPx = acct.partition?.primary_proxy ?? null;
              const assigned = acct.partition?.assigned_proxies ?? [];
              const firstNode = assigned[0]?.name;
              const lastNode = assigned[assigned.length - 1]?.name;
              const { geminiWeekly, gemini5h, claudeWeekly, claude5h } = splitAgMainPools(acct);
              const cooldownMap = new Map(
                (acct.model_cooldowns ?? []).map((mc) => [mc.model, mc]),
              );
              const modelsList = (acct.quota?.models ?? []).filter(
                (m) =>
                  m.unlocked !== false &&
                  (!qLower ||
                    m.model.toLowerCase().includes(qLower) ||
                    (m.display_name || "").toLowerCase().includes(qLower) ||
                    acct.name.toLowerCase().includes(qLower) ||
                    (acct.email || "").toLowerCase().includes(qLower)),
              );

              const renderFamilyPanel = (
                title: string,
                weekly?: AntigravityPoolWindow,
                fiveH?: AntigravityPoolWindow,
              ) => (
                <div className="space-y-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/45 p-3.5">
                  <div className="text-[13px] font-semibold text-[var(--text-primary)]">
                    {title}
                  </div>
                  {[
                    { label: "Weekly limit", w: weekly },
                    { label: "5h limit", w: fiveH },
                  ].map(({ label, w }) => {
                    const pct = w
                      ? Math.max(0, Math.min(100, Math.round(w.remaining_percentage)))
                      : null;
                    return (
                      <div
                        key={label}
                        className="flex items-center justify-between gap-3 text-[13px] tabular-nums"
                      >
                        <div className="min-w-0">
                          <div className="text-[var(--text-secondary)]">{label}</div>
                          {w?.reset_at && w.reset_at > nowMs && (
                            <div className="text-[11px] text-[var(--text-muted)]">
                              Resets in {formatCountdown(w.reset_at, nowMs)}
                            </div>
                          )}
                        </div>
                        {pct !== null ? (
                          <div className="flex shrink-0 items-center gap-2 font-mono">
                            <span
                              className="font-semibold"
                              style={{ color: agPctColor(pct) }}
                            >
                              {pct}% left
                            </span>
                            <QuotaRing pct={pct} size={18} />
                          </div>
                        ) : (
                          <span className="font-mono text-xs text-[var(--text-muted)]">
                            Pending sync
                          </span>
                        )}
                      </div>
                    );
                  })}
                </div>
              );

              const cardSurface = isBanned
                ? "border-[color-mix(in_srgb,var(--danger)_34%,var(--border-subtle))] bg-[color-mix(in_srgb,var(--danger-soft)_20%,var(--bg-surface))]"
                : cooling
                  ? "border-[color-mix(in_srgb,var(--warning)_38%,var(--border-subtle))] bg-[color-mix(in_srgb,var(--warning-soft)_22%,var(--bg-surface))]"
                  : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]";

              return (
                <article
                  key={acct.id}
                  className={`flex flex-col justify-between gap-4 rounded-2xl border p-4 sm:p-5 transition-[border-color,background-color] duration-150 ${cardSurface} ${
                    !acct.enabled ? "opacity-80" : ""
                  }`}
                >
                  {/* Account Header Row */}
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-[11px] font-semibold text-[var(--text-muted)]">
                          #{idx + 1}
                        </span>
                        <span className="truncate text-[15px] font-semibold text-[var(--text-primary)]">
                          {acct.email || acct.name}
                        </span>
                        {acct.quota?.plan && (
                          <span className="rounded-md border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-[11px] font-semibold uppercase text-[var(--accent)]">
                            {acct.quota.plan}
                          </span>
                        )}
                        {isBanned ? (
                          <StatusPill tone="danger" dot>
                            Banned &middot; Auto-disabled
                          </StatusPill>
                        ) : !acct.enabled ? (
                          <StatusPill tone="neutral">Disabled</StatusPill>
                        ) : cooling ? (
                          <StatusPill tone="warning" dot pulse>
                            {acct.cooldown_until && acct.cooldown_until > nowMs
                              ? formatCountdown(acct.cooldown_until, nowMs)
                              : "Cooldown"}
                          </StatusPill>
                        ) : (
                          <StatusPill tone="success" dot>
                            Ready
                          </StatusPill>
                        )}
                      </div>

                      <div className="mt-1.5 flex flex-wrap items-center gap-2 font-mono text-xs text-[var(--text-muted)]">
                        <span>
                          Sticky:{" "}
                          <span className="font-semibold text-[var(--accent)]">
                            {primaryPx ? primaryPx.name : "Direct"}
                          </span>
                        </span>
                        {assigned.length > 1 && !assigned[0]?.is_direct && (
                          <span>
                            &middot; Slice {firstNode} &rarr; {lastNode} ({assigned.length} proxies)
                          </span>
                        )}
                        <span>
                          &middot; Synced{" "}
                          {acct.quota?.fetched_at
                            ? formatTimeAgo(acct.quota.fetched_at, nowMs)
                            : "never"}
                        </span>
                      </div>
                    </div>

                    <div className="flex shrink-0 items-center gap-1.5">
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={refreshingAgOne === acct.id}
                        onClick={() => void handleRefreshAgSingleClick(acct.id)}
                        title="Sync Cloud Code quotas"
                      >
                        <RefreshCw
                          className={`h-3.5 w-3.5 ${
                            refreshingAgOne === acct.id ? "animate-spin" : ""
                          }`}
                        />
                        Sync
                      </Button>
                      {cooling && onUpdateAccountConnection && (
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={togglingAgId === acct.id}
                          onClick={() => void handleResetAgCooldownClick(acct.id)}
                          title="Reset account cooldown"
                        >
                          <RotateCcw className="h-3.5 w-3.5 text-[var(--warning)]" />
                          Reset
                        </Button>
                      )}
                      {onUpdateAccountConnection && (
                        <Button
                          variant={acct.enabled ? "secondary" : "primary"}
                          size="sm"
                          disabled={togglingAgId === acct.id}
                          onClick={() => void handleToggleAgEnabledClick(acct)}
                        >
                          {acct.enabled ? "Disable" : "Enable"}
                        </Button>
                      )}
                    </div>
                  </div>

                  {acct.last_error && (
                    <div
                      className={`flex items-start gap-2 rounded-xl px-3.5 py-2.5 text-xs ${
                        isBanned
                          ? "bg-[var(--danger-soft)] text-[var(--danger)]"
                          : "bg-[var(--warning-soft)] text-[var(--warning)]"
                      }`}
                    >
                      {isBanned ? (
                        <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
                      ) : (
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                      )}
                      <span className="min-w-0 break-words">
                        {formatCooldownReason(acct.last_error) || acct.last_error}
                      </span>
                    </div>
                  )}

                  {/* Main Quota Families: Gemini Models | Claude and GPT models */}
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    {renderFamilyPanel("Gemini Models", geminiWeekly, gemini5h)}
                    {renderFamilyPanel("Claude and GPT models", claudeWeekly, claude5h)}
                  </div>

                  {/* Available Models Strip (unlocked only + per-model cooldown countdowns) */}
                  {modelsList.length === 0 ? (
                    <div className="text-xs text-[var(--text-muted)]">
                      Click Sync to load unlocked Cloud Code models for this account.
                    </div>
                  ) : (
                    <div className="flex flex-wrap items-center gap-2 pt-0.5">
                      {modelsList.map((mq) => {
                        const pct = Math.max(
                          0,
                          Math.min(100, Math.round(mq.remaining_percentage)),
                        );
                        const mc = cooldownMap.get(mq.model);
                        const coolUntil = mq.failure_cooldown_until ?? mc?.cooldown_until ?? null;
                        const failCount = mq.failure_count ?? mc?.consecutive_failures ?? 0;
                        const modelCooling = !!coolUntil && coolUntil > nowMs;
                        return (
                          <span
                            key={mq.model}
                            title={
                              modelCooling
                                ? `${mq.display_name || mq.model} — ${failCount} consecutive failure(s), cooling ${formatCountdown(coolUntil, nowMs)}${mq.failure_reason ? ` — ${mq.failure_reason}` : ""}`
                                : `${mq.display_name || mq.model} (${mq.model}) — ${pct}% remaining`
                            }
                            className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs ${
                              modelCooling
                                ? "border-[color-mix(in_srgb,var(--warning)_40%,transparent)] bg-[var(--warning-soft)]"
                                : pct <= 0
                                  ? "border-[color-mix(in_srgb,var(--danger)_30%,transparent)] bg-[var(--danger-soft)]"
                                  : "border-[var(--border-subtle)] bg-[var(--bg-elevated)]/75"
                            }`}
                          >
                            <span className="font-medium text-[var(--text-primary)]">
                              {mq.display_name || mq.model}
                            </span>
                            <span
                              className="font-mono font-semibold tabular-nums"
                              style={{ color: agPctColor(pct) }}
                            >
                              {pct}%
                            </span>
                            {modelCooling && (
                              <span className="font-mono text-[11px] font-semibold text-[var(--warning)]">
                                ({failCount}f &middot; {formatCountdown(coolUntil, nowMs)})
                              </span>
                            )}
                          </span>
                        );
                      })}
                    </div>
                  )}
                </article>
              );
            })}
          </div>
        )
      ) : /* ================= OPENCODE PER-IP QUOTA VIEW (SEPARATE PROXY/IP CARDS) ================= */ filteredIpQuotas.length ===
        0 ? (
        <EmptyState
          title="No matching exit-IP buckets"
          description="Clear your search filter or add proxies in the Proxy Pool."
        />
      ) : (
        /* SEPARATE EXIT-IP CARDS: 4 items/line in Table view, 2 items/line in List view */
        <div
          key={`${viewMode}-${ipFilter}`}
          className={
            viewMode === "list"
              ? "animate-tab-switch grid grid-cols-1 gap-3 md:grid-cols-2"
              : "animate-tab-switch grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4"
          }
        >
          {visibleIpQuotas.map((q) => {
            const cooling = q.status === "cooldown";
            const cleanReason = formatCooldownReason(q.last_error);
            const hasCustomCap =
              (q.max_requests_per_window && q.max_requests_per_window > 0) ||
              (q.max_tokens_per_window && q.max_tokens_per_window > 0);
            const displayIp = q.ip_key.startsWith("unprobed:")
              ? q.ip_key.replace("unprobed:", "")
              : q.ip_key;
            const primaryProxy = q.proxies[0] ?? "Unassigned";
            const extraProxies = Math.max(0, q.proxies.length - 1);
            const usedReqs = q.requests_in_window || q.total_requests;
            const usedToks =
              q.tokens_in_window || q.total_prompt_tokens + q.total_completion_tokens;
            const isVerifyingThis = verifyingIpKey === q.ip_key;
            const cooldownBadgeLabel =
              q.cooldown_until && q.cooldown_until > nowMs
                ? `Queued · ${formatCountdown(q.cooldown_until, nowMs)}`
                : activeProbeEnabled
                  ? `Queued · ${activeProbeIntervalSec}s check`
                  : "Queued";
            const cardSurface = cooling
              ? "border-[color-mix(in_srgb,var(--warning)_40%,var(--border-subtle))] bg-[color-mix(in_srgb,var(--warning-soft)_32%,var(--bg-surface))] hover:border-[color-mix(in_srgb,var(--warning)_60%,var(--border-subtle))]"
              : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-hover)]/35";

            if (viewMode === "list") {
              return (
                <article
                  key={q.ip_key}
                  className={`group flex flex-col justify-center gap-2 rounded-xl border px-4 py-3.5 text-[13px] transition-[border-color,background-color] duration-150 ${cardSurface}`}
                >
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex min-w-0 flex-1 items-center gap-2.5">
                      <span
                        className={`h-2 w-2 shrink-0 rounded-full ${
                          cooling ? "bg-[var(--warning)] animate-pulse" : "bg-[var(--success)]"
                        }`}
                      />
                      <span className="truncate font-mono text-sm font-semibold text-[var(--text-primary)]">
                        {displayIp}
                      </span>
                      <span
                        className="truncate font-mono text-xs text-[var(--text-muted)]"
                        title={q.proxies.join(", ")}
                      >
                        {primaryProxy}
                        {extraProxies > 0 ? ` +${extraProxies}` : ""}
                      </span>
                    </div>

                    <div className="flex shrink-0 items-center gap-2 tabular-nums">
                      {cooling ? (
                        <span className="font-mono text-xs font-semibold text-[var(--warning)]">
                          {cooldownBadgeLabel}
                        </span>
                      ) : (
                        <span className="font-mono text-xs text-[var(--text-muted)]">
                          {usedReqs > 0
                            ? `${usedReqs} req${usedToks > 0 ? ` · ${formatCompactNumber(usedToks)}` : ""}`
                            : formatTimeAgo(q.last_quota_checked_at ?? q.last_used_at, nowMs)}
                        </span>
                      )}

                      <div
                        className={`flex items-center gap-1 transition-opacity ${
                          cooling
                            ? "opacity-100"
                            : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
                        }`}
                      >
                        <button
                          type="button"
                          onClick={() => openLimitsDrawer(q)}
                          className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                          title="Configure window & caps"
                          aria-label={`Configure limits for ${displayIp}`}
                        >
                          <SlidersHorizontal className="h-4 w-4" />
                        </button>

                        {(cooling || q.requests_in_window > 0) && (
                          <button
                            type="button"
                            disabled={isVerifyingThis}
                            onClick={() => void handleVerifyResetSingle(q.ip_key)}
                            className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-[var(--warning)] hover:bg-[var(--bg-elevated)] disabled:opacity-45"
                            title="Verify live OpenCode availability before resetting cooldown"
                          >
                            {isVerifyingThis ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <RotateCcw className="h-3.5 w-3.5" />
                            )}
                            <span>{isVerifyingThis ? "…" : "Verify"}</span>
                          </button>
                        )}
                      </div>
                    </div>
                  </div>

                  {cooling && cleanReason && (
                    <div
                      className="flex items-center gap-1.5 pl-4 text-xs text-[var(--warning)]"
                      title={q.last_error || undefined}
                    >
                      <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">{cleanReason}</span>
                    </div>
                  )}

                  {hasCustomCap && (
                    <div className="space-y-1.5 pl-4 pt-0.5">
                      {q.max_requests_per_window ? (
                        <MeterBar
                          label="Window requests"
                          used={q.requests_in_window}
                          limit={q.max_requests_per_window}
                          tone="accent"
                        />
                      ) : null}
                      {q.max_tokens_per_window ? (
                        <MeterBar
                          label="Window tokens"
                          used={q.tokens_in_window}
                          limit={q.max_tokens_per_window}
                          tone="cache"
                        />
                      ) : null}
                    </div>
                  )}
                </article>
              );
            }

            return (
              <article
                key={q.ip_key}
                className={`group flex flex-col justify-between gap-2 rounded-xl border px-4 py-3.5 text-[13px] transition-[border-color,background-color] duration-150 ${cardSurface}`}
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      className={`h-2 w-2 shrink-0 rounded-full ${
                        cooling ? "bg-[var(--warning)] animate-pulse" : "bg-[var(--success)]"
                      }`}
                    />
                    <span className="truncate font-mono text-sm font-semibold text-[var(--text-primary)]">
                      {displayIp}
                    </span>
                  </div>

                  <div className="flex shrink-0 items-center gap-1 tabular-nums">
                    {cooling && (
                      <span className="font-mono text-xs font-semibold text-[var(--warning)]">
                        {cooldownBadgeLabel}
                      </span>
                    )}

                    <div
                      className={`flex items-center gap-0.5 transition-opacity ${
                        cooling
                          ? "opacity-100"
                          : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
                      }`}
                    >
                      <button
                        type="button"
                        onClick={() => openLimitsDrawer(q)}
                        className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                        title="Configure soft limits"
                        aria-label={`Configure limits for ${displayIp}`}
                      >
                        <SlidersHorizontal className="h-4 w-4" />
                      </button>

                      {((!cooling && q.requests_in_window > 0) || (cooling && !cleanReason)) && (
                        <button
                          type="button"
                          disabled={isVerifyingThis}
                          onClick={() => void handleVerifyResetSingle(q.ip_key)}
                          className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--warning)] disabled:opacity-45"
                          title="Verify live OpenCode availability & reset cooldown"
                          aria-label={`Verify and reset ${displayIp}`}
                        >
                          {isVerifyingThis ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <RotateCcw className="h-4 w-4" />
                          )}
                        </button>
                      )}
                    </div>
                  </div>
                </div>

                <div className="flex items-center justify-between gap-2 pl-4 font-mono text-xs text-[var(--text-muted)] tabular-nums">
                  <span className="truncate" title={q.proxies.join(", ")}>
                    {primaryProxy}
                    {extraProxies > 0 ? ` +${extraProxies}` : ""}
                  </span>

                  <span className="shrink-0">
                    {usedReqs > 0
                      ? `${usedReqs} req${usedToks > 0 ? ` · ${formatCompactNumber(usedToks)}` : ""}`
                      : formatTimeAgo(q.last_quota_checked_at ?? q.last_used_at, nowMs)}
                  </span>
                </div>

                {hasCustomCap && (
                  <div className="space-y-1.5 pl-4 pt-0.5">
                    {q.max_requests_per_window ? (
                      <MeterBar
                        label="Requests"
                        used={q.requests_in_window}
                        limit={q.max_requests_per_window}
                        tone="accent"
                      />
                    ) : null}
                    {q.max_tokens_per_window ? (
                      <MeterBar
                        label="Tokens"
                        used={q.tokens_in_window}
                        limit={q.max_tokens_per_window}
                        tone="cache"
                      />
                    ) : null}
                  </div>
                )}

                {cooling && cleanReason && (
                  <div
                    className="flex items-center justify-between gap-2 pl-4 pt-0.5 text-xs text-[var(--warning)]"
                    title={q.last_error || undefined}
                  >
                    <span className="inline-flex min-w-0 items-center gap-1.5 truncate">
                      <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">{cleanReason}</span>
                    </span>
                    <button
                      type="button"
                      disabled={isVerifyingThis}
                      onClick={() => void handleVerifyResetSingle(q.ip_key)}
                      className="shrink-0 font-semibold underline-offset-2 hover:underline disabled:opacity-45"
                    >
                      {isVerifyingThis ? "Checking…" : "Verify"}
                    </button>
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}

      {!isAntigravity && !showAllBuckets && filteredIpQuotas.length > 48 && (
        <div className="flex justify-center pt-1">
          <Button variant="secondary" onClick={() => setShowAllBuckets(true)}>
            Show all {filteredIpQuotas.length} exit-IP buckets
          </Button>
        </div>
      )}

      {/* Slide-Over IP Limits & Diagnostic Drawer */}
      <SlideOverDrawer
        open={!!editingIp}
        title="IP window & soft caps"
        subtitle={editingIp?.ip_key}
        onClose={() => setEditingIp(null)}
      >
        {editingIp && (
          <form onSubmit={handleLimitsSave} className="space-y-4">
            <div className="space-y-1 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3 text-xs text-[var(--text-muted)]">
              <div>
                <span className="font-medium text-[var(--text-secondary)]">Routes: </span>
                <span className="font-mono">{editingIp.proxies.join(", ") || "None"}</span>
              </div>
              <div>
                <span className="font-medium text-[var(--text-secondary)]">Lifetime: </span>
                <span className="font-mono">
                  {editingIp.total_requests} requests &middot;{" "}
                  {formatCompactNumber(
                    editingIp.total_prompt_tokens + editingIp.total_completion_tokens,
                  )}{" "}
                  tokens &middot; {editingIp.total_rate_limits} rate-limits
                </span>
              </div>
            </div>

            {editingIp.last_error && (
              <div className="space-y-1.5 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
                <div className="text-xs font-medium text-[var(--warning)]">
                  {formatCooldownReason(editingIp.last_error)}
                </div>
                <details className="text-[11px] text-[var(--text-muted)]">
                  <summary className="cursor-pointer select-none hover:text-[var(--text-secondary)]">
                    Raw upstream payload
                  </summary>
                  <pre className="mt-2 overflow-x-auto whitespace-pre-wrap rounded-lg bg-[var(--bg-surface)] p-2 font-mono text-[11px] text-[var(--text-secondary)]">
                    {editingIp.last_error}
                  </pre>
                </details>
              </div>
            )}

            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Rolling window duration (minutes)
              </label>
              <input
                type="number"
                min={1}
                required
                value={windowMinutes}
                onChange={(e) => setWindowMinutes(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>

            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Max requests per window (optional)
              </label>
              <input
                type="number"
                min={0}
                placeholder="Unlimited — rely on upstream 429"
                value={maxReqs}
                onChange={(e) => setMaxReqs(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>

            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Max tokens per window (optional)
              </label>
              <input
                type="number"
                min={0}
                placeholder="Unlimited"
                value={maxTokens}
                onChange={(e) => setMaxTokens(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>

            <div className="flex justify-end gap-2 pt-3">
              <Button variant="secondary" size="md" onClick={() => setEditingIp(null)}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" size="md" disabled={savingLimits}>
                {savingLimits ? "Saving…" : "Save limits"}
              </Button>
            </div>
          </form>
        )}
      </SlideOverDrawer>

      {/* Export working proxies as portable URLs */}
      <Modal
        open={exportOpen}
        onClose={() => setExportOpen(false)}
        title="Export proxies"
        subtitle="One URL per line — paste into any SOCKS/HTTP client, or back into the proxy import box"
        maxWidth="max-w-2xl"
      >
        <div className="space-y-4">
          <div className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5">
            {(Object.keys(exportScopes) as ExportScope[]).map((scope) => (
              <button
                key={scope}
                type="button"
                onClick={() => setExportScope(scope)}
                className={`rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                  exportScope === scope
                    ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                }`}
              >
                {exportScopes[scope].label}
              </button>
            ))}
          </div>

          <textarea
            ref={exportTextareaRef}
            readOnly
            rows={12}
            value={exportLines.join("\n")}
            placeholder="No proxies available to export"
            onFocus={(e) => e.currentTarget.select()}
            className={`${INPUT_CLASS} font-mono text-xs`}
          />

          <p className="text-[11px] text-[var(--text-muted)]">
            <span className="font-medium text-[var(--text-secondary)]">Ready</span> = alive,
            enabled, and not in an OpenCode Free cooldown. Includes credentials exactly as
            stored. Direct routes are skipped
            {unexportableCount > 0
              ? `, along with ${unexportableCount} raw-JSON ${
                  unexportableCount === 1 ? "node" : "nodes"
                } that has no shareable URL`
              : ""}
            ; duplicates are collapsed.
          </p>

          <div className="flex justify-end gap-2">
            <Button
              variant="secondary"
              size="md"
              disabled={exportLines.length === 0}
              onClick={() => void handleCopyExport()}
            >
              {exportCopied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
              {exportCopied ? "Copied" : "Copy all"}
            </Button>
            <Button
              variant="primary"
              size="md"
              disabled={exportLines.length === 0}
              onClick={handleDownloadExport}
            >
              <Download className="h-4 w-4" />
              Download .txt
            </Button>
          </div>
        </div>
      </Modal>
    </div>
  );
}
