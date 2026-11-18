import { useState } from "react";
import {
  Activity,
  ArrowDown,
  ArrowDownLeft,
  ArrowUp,
  ArrowUpRight,
  ChevronRight,
  Cpu,
  Database,
  KeyRound,
  Layers,
  Network,
  Repeat,
  Zap,
} from "lucide-react";
import { LiveTopologyMap } from "../components/LiveTopologyMap";
import { LogDiagnosticContent } from "../components/LogDiagnosticContent";
import { ProviderIcon } from "../components/ProviderIcon";
import { DashboardSkeleton } from "../components/Skeleton";
import { Card, SegmentedControl, SlideOverDrawer } from "../components/UiPrimitives";
import { UsageAreaChart } from "../components/UsageAreaChart";
import type {
  TopologyResponse,
  UsageLogRecord,
  UsagePeriod,
  UsageSummary,
} from "../types";
import {
  formatCompactNumber,
  formatMs,
  formatNumber,
  formatPct,
  formatTimeAgo,
  formatTps,
  getProviderLogo,
  providerDisplayName,
} from "../utils/format";

type PivotTab = "model" | "provider" | "apiKey" | "proxy";
type SortField = "name" | "requests" | "prompt" | "cached" | "completion" | "speed" | "lastUsed";

interface PivotRow {
  id: string;
  name: string;
  secondary?: string;
  requests: number;
  successRate: string;
  prompt: number;
  cached: number;
  completion: number;
  speed: number;
  lastUsed: number | null;
}

export function DashboardView({
  period,
  onChangePeriod,
  usage,
  topology,
  loading,
  nowMs,
  onInspectProviderQuota,
  onOpenAllLogs,
}: {
  period: UsagePeriod;
  onChangePeriod: (p: UsagePeriod) => void;
  usage: UsageSummary | null;
  topology: TopologyResponse | null;
  loading: boolean;
  nowMs: number;
  onInspectProviderQuota: (providerId: string) => void;
  onOpenAllLogs: () => void;
}) {
  const [pivot, setPivot] = useState<PivotTab>("model");
  const [sortField, setSortField] = useState<SortField>("requests");
  const [sortAsc, setSortAsc] = useState(false);
  const [selectedLog, setSelectedLog] = useState<UsageLogRecord | null>(null);

  if (!usage && loading) {
    return <DashboardSkeleton />;
  }

  const totalRequests = usage?.total_requests ?? 0;
  const successRequests = usage?.success_requests ?? 0;
  const rateLimited = usage?.rate_limited_requests ?? 0;
  const promptTokens = usage?.prompt_tokens ?? 0;
  const cachedTokens = usage?.cached_tokens ?? 0;
  const completionTokens = usage?.completion_tokens ?? 0;
  const grossInputTokens = promptTokens + cachedTokens;

  const providerMetaById = new Map(
    (topology?.providers ?? []).map((p) => [p.id.toLowerCase(), p]),
  );

  function resolveProviderMeta(providerId: string, modelSpec?: string) {
    const rawId = (providerId || "").trim();
    const lower = rawId.toLowerCase();
    const fromTopology = providerMetaById.get(lower);
    const isCombo = lower === "combo" && !fromTopology;
    const name = fromTopology?.name || providerDisplayName(rawId || "opencode");
    const logo =
      fromTopology?.logo ||
      (lower.startsWith("cp-")
        ? undefined
        : getProviderLogo(rawId || modelSpec || "opencode"));
    const color = fromTopology?.color;
    return { name, logo, color, isCombo };
  }

  // Enforce 2xx-only in Dashboard `Recent requests` (never show failed/429 requests here)
  const succeededRecent = (usage?.recent ?? [])
    .filter((r) => r.status_code >= 200 && r.status_code < 300)
    .slice(0, 40);

  function toggleSort(field: SortField) {
    if (sortField === field) {
      setSortAsc((a) => !a);
    } else {
      setSortField(field);
      setSortAsc(field === "name");
    }
  }

  const rawRows: PivotRow[] = (() => {
    if (!usage) return [];
    switch (pivot) {
      case "model":
        return usage.by_model.map((m) => ({
          id: m.model,
          name: m.model,
          secondary: resolveProviderMeta(m.provider_id, m.model).name,
          requests: m.total_requests,
          successRate: formatPct(m.success_requests, m.total_requests),
          prompt: m.prompt_tokens,
          cached: m.cached_tokens,
          completion: m.completion_tokens,
          speed: m.avg_tokens_per_sec,
          lastUsed: m.last_used_at,
        }));
      case "provider":
        return usage.by_provider.map((p) => ({
          id: p.provider_id,
          name: resolveProviderMeta(p.provider_id).name,
          secondary: p.provider_id,
          requests: p.total_requests,
          successRate: formatPct(p.success_requests, p.total_requests),
          prompt: p.prompt_tokens,
          cached: p.cached_tokens,
          completion: p.completion_tokens,
          speed: p.avg_tokens_per_sec,
          lastUsed: p.last_used_at,
        }));
      case "apiKey":
        return usage.by_api_key.map((k) => ({
          id: k.key_name,
          name: k.key_name,
          secondary: k.key_prefix ? `${k.key_prefix}...` : "Unauthenticated",
          requests: k.total_requests,
          successRate: formatPct(k.success_requests, k.total_requests),
          prompt: k.prompt_tokens,
          cached: k.cached_tokens,
          completion: k.completion_tokens,
          speed: 0,
          lastUsed: k.last_used_at,
        }));
      case "proxy":
        return usage.by_proxy.map((px) => ({
          id: px.proxy_name,
          name: px.proxy_name,
          secondary: px.last_egress_ip || "—",
          requests: px.total_requests,
          successRate: formatPct(px.success_requests, px.total_requests),
          prompt: px.prompt_tokens,
          cached: px.cached_tokens,
          completion: px.completion_tokens,
          speed: px.avg_tokens_per_sec,
          lastUsed: px.last_used_at,
        }));
    }
  })();

  const sortedRows = [...rawRows].sort((a, b) => {
    const dir = sortAsc ? 1 : -1;
    switch (sortField) {
      case "name":
        return a.name.localeCompare(b.name) * dir;
      case "requests":
        return (a.requests - b.requests) * dir;
      case "prompt":
        return (a.prompt - b.prompt) * dir;
      case "cached":
        return (a.cached - b.cached) * dir;
      case "completion":
        return (a.completion - b.completion) * dir;
      case "speed":
        return (a.speed - b.speed) * dir;
      case "lastUsed":
        return ((a.lastUsed ?? 0) - (b.lastUsed ?? 0)) * dir;
    }
  });

  const maxProviderTokens = Math.max(
    1,
    ...(usage?.by_provider.map((p) => p.total_tokens) ?? [0]),
  );
  const maxModelTokens = Math.max(
    1,
    ...(usage?.by_model.map((m) => m.total_tokens) ?? [0]),
  );

  function renderSortIcon(field: SortField) {
    if (sortField !== field) return null;
    return sortAsc ? (
      <ArrowUp className="inline h-3 w-3 ml-1" />
    ) : (
      <ArrowDown className="inline h-3 w-3 ml-1" />
    );
  }

  return (
    <div className="space-y-5">
      {/* Period Switcher Row */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="text-xs font-medium text-[var(--text-muted)]">
          Gateway telemetry window
        </span>

        <SegmentedControl
          size="sm"
          value={period}
          onChange={onChangePeriod}
          options={[
            { value: "today", label: "Today" },
            { value: "24h", label: "24h" },
            { value: "7d", label: "7D" },
            { value: "30d", label: "30D" },
            { value: "all", label: "All" },
          ]}
        />
      </div>

      {/* 5-Column Universal Telemetry Metric Strip */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-5">
        <div className="tr-card-interactive rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3.5 hover:border-[var(--border-strong)]">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Total requests</span>
            <Activity className="h-3.5 w-3.5 text-[var(--accent)]" />
          </div>
          <div className="mt-1.5 font-mono text-2xl font-semibold tracking-tight text-[var(--text-primary)] tabular-nums">
            {formatNumber(totalRequests)}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-muted)] tabular-nums">
            {formatPct(successRequests, totalRequests)} OK &middot; {rateLimited} rate-limited
          </div>
        </div>

        <div className="tr-card-interactive rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3.5 hover:border-[var(--border-strong)]">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Input tokens</span>
            <ArrowUpRight className="h-3.5 w-3.5 text-[var(--token-in)]" />
          </div>
          <div className="mt-1.5 font-mono text-2xl font-semibold tracking-tight text-[var(--token-in)] tabular-nums">
            {formatCompactNumber(promptTokens)}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-muted)] tabular-nums">
            {formatCompactNumber(grossInputTokens)} gross context
          </div>
        </div>

        <div className="tr-card-interactive rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3.5 hover:border-[var(--border-strong)]">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Cached tokens</span>
            <Database className="h-3.5 w-3.5 text-[var(--token-cache)]" />
          </div>
          <div className="mt-1.5 font-mono text-2xl font-semibold tracking-tight text-[var(--token-cache)] tabular-nums">
            {formatCompactNumber(cachedTokens)}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-muted)] tabular-nums">
            {formatPct(cachedTokens, grossInputTokens)} cache read hit
          </div>
        </div>

        <div className="tr-card-interactive rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3.5 hover:border-[var(--border-strong)]">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Output tokens</span>
            <ArrowDownLeft className="h-3.5 w-3.5 text-[var(--token-out)]" />
          </div>
          <div className="mt-1.5 font-mono text-2xl font-semibold tracking-tight text-[var(--token-out)] tabular-nums">
            {formatCompactNumber(completionTokens)}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-muted)] tabular-nums">
            {formatCompactNumber(usage?.total_tokens ?? 0)} total combined
          </div>
        </div>

        <div className="tr-card-interactive rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3.5 hover:border-[var(--border-strong)]">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Generation speed</span>
            <Zap className="h-3.5 w-3.5 text-[var(--speed)]" />
          </div>
          <div className="mt-1.5 font-mono text-2xl font-semibold tracking-tight text-[var(--speed)] tabular-nums">
            {usage && usage.avg_tokens_per_sec > 0
              ? `${usage.avg_tokens_per_sec.toFixed(1)} tok/s`
              : "—"}
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-muted)] tabular-nums">
            Peak {usage ? formatTps(usage.peak_tokens_per_sec) : "—"} &middot; TTFT{" "}
            {usage ? formatMs(usage.avg_ttft_ms) : "—"}
          </div>
        </div>
      </div>

      {/* Live Routing Topology + Recent Completed Requests Feed (2xx Only) */}
      <div className="grid min-w-0 grid-cols-1 items-stretch gap-4 lg:grid-cols-[minmax(0,1.6fr)_minmax(360px,1fr)]">
        <LiveTopologyMap
          topology={topology}
          onSelectProvider={onInspectProviderQuota}
        />

        <Card
          title="Recent requests"
          subtitle="Completed 2xx generations only"
          actions={
            <button
              type="button"
              onClick={onOpenAllLogs}
              className="inline-flex items-center gap-0.5 text-xs font-medium text-[var(--accent)] hover:underline"
            >
              All logs
              <ChevronRight className="h-3.5 w-3.5" />
            </button>
          }
          noPadding
          className="flex min-w-0 flex-col overflow-hidden"
          bodyClassName="flex min-h-0 flex-1 flex-col"
        >
          {succeededRecent.length === 0 ? (
            <div className="flex min-h-[320px] flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-xs text-[var(--text-muted)]">
              <Activity className="h-5 w-5 text-[var(--text-muted)]/60" />
              <span>No completed requests in this period yet.</span>
            </div>
          ) : (
            <>
              <div className="relative max-h-[420px] min-h-[320px] flex-1 lg:max-h-none lg:min-h-0">
                <div className="smooth-scroll h-full overflow-y-auto overflow-x-hidden lg:absolute lg:inset-0">
                  <table className="w-full table-fixed border-collapse text-xs">
                    <thead className="sticky top-0 z-10 border-b border-[var(--border-subtle)] bg-[var(--bg-surface)]/95 text-[11px] font-medium text-[var(--text-muted)] backdrop-blur-sm">
                      <tr>
                        <th className="py-2.5 pl-4 pr-2 text-left">Route &amp; Model</th>
                        <th className="w-[96px] py-2.5 px-2 text-right whitespace-nowrap">In / Out</th>
                        <th className="w-[58px] py-2.5 px-2 text-right whitespace-nowrap">tok/s</th>
                        <th className="w-[66px] py-2.5 pl-2 pr-4 text-right whitespace-nowrap">When</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-[var(--border-subtle)]/80 tabular-nums">
                      {succeededRecent.map((r) => {
                        const attempts = r.attempt_count ?? 1;
                        const meta = resolveProviderMeta(r.provider_id, r.model);
                        const proxyLabel =
                          r.proxy_name && r.proxy_name !== "DIRECT" ? ` · ${r.proxy_name}` : "";
                        return (
                          <tr
                            key={r.id}
                            tabIndex={0}
                            onClick={() => setSelectedLog(r)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                setSelectedLog(r);
                              }
                            }}
                            className="tr-row-interactive cursor-pointer transition-colors duration-150 hover:bg-[var(--bg-hover)]/75 focus:outline-none focus-visible:bg-[var(--bg-hover)]"
                          >
                            <td className="py-2.5 pl-4 pr-2">
                              <div className="flex items-center gap-2.5 min-w-0">
                                <ProviderIcon
                                  src={meta.logo}
                                  name={meta.name}
                                  color={meta.color}
                                  isCombo={meta.isCombo}
                                  size={26}
                                />
                                <div className="min-w-0 flex-1">
                                  <div className="flex items-center gap-1.5 min-w-0">
                                    <span
                                      className="truncate font-mono text-[11.5px] font-medium text-[var(--text-primary)]"
                                      title={r.model}
                                    >
                                      {r.model}
                                    </span>
                                    {attempts > 1 && (
                                      <span
                                        className="inline-flex shrink-0 items-center gap-0.5 rounded bg-[var(--bg-elevated)] px-1 py-0.5 font-mono text-[9.5px] text-[var(--warning)]"
                                        title={`${attempts} failover attempts`}
                                      >
                                        <Repeat className="h-2.5 w-2.5" />
                                        {attempts}
                                      </span>
                                    )}
                                  </div>
                                  <div
                                    className="truncate text-[10.5px] text-[var(--text-muted)]"
                                    title={`${meta.name}${proxyLabel}`}
                                  >
                                    {meta.name}
                                    {proxyLabel}
                                  </div>
                                </div>
                              </div>
                            </td>
                            <td
                              className="py-2.5 px-2 text-right font-mono text-[11px] whitespace-nowrap"
                              title={`Uncached In: ${r.prompt_tokens} · Cached Read: ${r.cached_tokens} · Out: ${r.completion_tokens}`}
                            >
                              <span className="text-[var(--token-in)]">
                                {formatCompactNumber(r.prompt_tokens)}&uarr;
                              </span>{" "}
                              <span className="text-[var(--token-out)]">
                                {formatCompactNumber(r.completion_tokens)}&darr;
                              </span>
                            </td>
                            <td className="py-2.5 px-2 text-right font-mono text-[11px] font-medium whitespace-nowrap text-[var(--speed)]">
                              {r.tokens_per_sec ? `${r.tokens_per_sec.toFixed(0)}` : "—"}
                            </td>
                            <td className="py-2.5 pl-2 pr-4 text-right text-[11px] whitespace-nowrap text-[var(--text-muted)]">
                              {formatTimeAgo(r.created_at, nowMs)}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </div>
              <div className="flex items-center justify-between gap-2 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 px-4 py-2.5 text-xs text-[var(--text-muted)] tabular-nums">
                <span>
                  Showing{" "}
                  <strong className="font-medium text-[var(--text-secondary)]">
                    {succeededRecent.length}
                  </strong>{" "}
                  latest 2xx
                </span>
                <span>Click any row to inspect trace</span>
              </div>
            </>
          )}
        </Card>
      </div>

      {/* Interactive Time-Series Chart */}
      <UsageAreaChart buckets={usage?.timeseries ?? []} />

      {/* 2-Column Provider & Model Distribution */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card title="Usage by provider">
          {!usage || usage.by_provider.length === 0 ? (
            <p className="text-xs text-[var(--text-muted)]">No provider traffic in this window.</p>
          ) : (
            <div className="space-y-3.5">
              {usage.by_provider.map((p) => {
                const pct = Math.min(100, Math.round((p.total_tokens / maxProviderTokens) * 100));
                const meta = resolveProviderMeta(p.provider_id);
                return (
                  <div key={p.provider_id} className="space-y-1.5">
                    <div className="flex items-center justify-between text-xs tabular-nums">
                      <span className="inline-flex items-center gap-2 font-medium text-[var(--text-primary)]">
                        <ProviderIcon
                          src={meta.logo}
                          name={meta.name}
                          color={meta.color}
                          isCombo={meta.isCombo}
                          size={18}
                        />
                        {meta.name}
                      </span>
                      <span className="font-mono text-[var(--text-secondary)]">
                        {formatCompactNumber(p.total_tokens)} tok &middot;{" "}
                        <span className="text-[var(--speed)]">
                          {formatTps(p.avg_tokens_per_sec)}
                        </span>
                      </span>
                    </div>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--bg-elevated)] border border-[var(--border-subtle)]">
                      <div
                        className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-300"
                        style={{ width: `${Math.max(pct, p.total_tokens > 0 ? 6 : 0)}%` }}
                      />
                    </div>
                    <div className="flex justify-between text-[11px] text-[var(--text-muted)] tabular-nums">
                      <span>
                        {p.success_requests}/{p.total_requests} OK ({p.rate_limited_requests} 429)
                      </span>
                      <span>TTFT {formatMs(p.avg_ttft_ms)}</span>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </Card>

        <Card title="Top models">
          {!usage || usage.by_model.length === 0 ? (
            <p className="text-xs text-[var(--text-muted)]">No model usage in this window.</p>
          ) : (
            <div className="space-y-3.5">
              {usage.by_model.slice(0, 6).map((m) => {
                const pct = Math.min(100, Math.round((m.total_tokens / maxModelTokens) * 100));
                return (
                  <div key={m.model} className="space-y-1.5">
                    <div className="flex items-center justify-between gap-2 text-xs tabular-nums">
                      <span className="inline-flex min-w-0 items-center gap-2 truncate font-mono font-medium text-[var(--text-primary)]">
                        <Cpu className="h-3.5 w-3.5 shrink-0 text-[var(--token-cache)]" />
                        <span className="truncate">{m.model}</span>
                      </span>
                      <span className="shrink-0 font-mono text-[var(--text-secondary)]">
                        {formatCompactNumber(m.total_tokens)} tok &middot;{" "}
                        <span className="text-[var(--speed)]">
                          {formatTps(m.avg_tokens_per_sec)}
                        </span>
                      </span>
                    </div>
                    <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--bg-elevated)] border border-[var(--border-subtle)]">
                      <div
                        className="h-full rounded-full bg-[var(--token-cache)] transition-[width] duration-300"
                        style={{ width: `${Math.max(pct, m.total_tokens > 0 ? 6 : 0)}%` }}
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </Card>
      </div>

      {/* Multi-Pivot Breakdown Table */}
      <Card
        title="Detailed breakdown"
        actions={
          <SegmentedControl
            size="sm"
            value={pivot}
            onChange={setPivot}
            options={[
              { value: "model", label: "By Model", icon: <Cpu className="h-3 w-3" /> },
              { value: "provider", label: "By Provider", icon: <Layers className="h-3 w-3" /> },
              { value: "apiKey", label: "By API Key", icon: <KeyRound className="h-3 w-3" /> },
              { value: "proxy", label: "By Proxy / IP", icon: <Network className="h-3 w-3" /> },
            ]}
          />
        }
        noPadding
      >
        {sortedRows.length === 0 ? (
          <div className="p-6 text-xs text-[var(--text-muted)]">
            No usage rows recorded for the selected period.
          </div>
        ) : (
          <div key={pivot} className="animate-tab-switch smooth-scroll overflow-x-auto">
            <table className="w-full text-left text-xs">
              <thead>
                <tr className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[11px] font-medium text-[var(--text-muted)]">
                  <th
                    onClick={() => toggleSort("name")}
                    className="cursor-pointer py-2.5 pl-5 pr-3 whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    {pivot === "model"
                      ? "Model"
                      : pivot === "provider"
                        ? "Provider"
                        : pivot === "apiKey"
                          ? "API Key"
                          : "Proxy / Egress IP"}
                    {renderSortIcon("name")}
                  </th>
                  <th
                    onClick={() => toggleSort("requests")}
                    className="cursor-pointer py-2.5 px-3 text-right whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    Requests{renderSortIcon("requests")}
                  </th>
                  <th
                    onClick={() => toggleSort("prompt")}
                    className="cursor-pointer py-2.5 px-3 text-right whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    Input{renderSortIcon("prompt")}
                  </th>
                  <th
                    onClick={() => toggleSort("cached")}
                    className="cursor-pointer py-2.5 px-3 text-right whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    Cached{renderSortIcon("cached")}
                  </th>
                  <th
                    onClick={() => toggleSort("completion")}
                    className="cursor-pointer py-2.5 px-3 text-right whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    Output{renderSortIcon("completion")}
                  </th>
                  <th
                    onClick={() => toggleSort("speed")}
                    className="cursor-pointer py-2.5 px-3 text-right whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    Avg Speed{renderSortIcon("speed")}
                  </th>
                  <th
                    onClick={() => toggleSort("lastUsed")}
                    className="cursor-pointer py-2.5 pl-3 pr-5 text-right whitespace-nowrap hover:text-[var(--text-primary)]"
                  >
                    Last Used{renderSortIcon("lastUsed")}
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-subtle)] tabular-nums">
                {sortedRows.map((row) => (
                  <tr
                    key={row.id}
                    className="tr-row-interactive hover:bg-[var(--bg-hover)]/60"
                  >
                    <td className="py-2.5 pl-5 pr-3">
                      <div className="font-mono font-medium text-[var(--text-primary)]">
                        {row.name}
                      </div>
                      {row.secondary && (
                        <div className="text-[11px] text-[var(--text-muted)]">
                          {row.secondary}
                        </div>
                      )}
                    </td>
                    <td className="py-2.5 px-3 text-right font-mono whitespace-nowrap text-[var(--text-secondary)]">
                      {formatNumber(row.requests)}{" "}
                      <span className="text-[11px] text-[var(--text-muted)]">
                        ({row.successRate})
                      </span>
                    </td>
                    <td className="py-2.5 px-3 text-right font-mono whitespace-nowrap text-[var(--token-in)]">
                      {formatCompactNumber(row.prompt)}
                    </td>
                    <td className="py-2.5 px-3 text-right font-mono whitespace-nowrap text-[var(--token-cache)]">
                      {formatCompactNumber(row.cached)}
                    </td>
                    <td className="py-2.5 px-3 text-right font-mono whitespace-nowrap text-[var(--token-out)]">
                      {formatCompactNumber(row.completion)}
                    </td>
                    <td className="py-2.5 px-3 text-right font-mono whitespace-nowrap font-semibold text-[var(--speed)]">
                      {formatTps(row.speed)}
                    </td>
                    <td className="py-2.5 pl-3 pr-5 text-right whitespace-nowrap text-[var(--text-muted)]">
                      {formatTimeAgo(row.lastUsed, nowMs)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>

      {/* Slide-over inspection for clicked Recent Request row */}
      <SlideOverDrawer
        open={!!selectedLog}
        title={selectedLog ? `Completed Request #${selectedLog.id}` : ""}
        subtitle={selectedLog?.request_id}
        onClose={() => setSelectedLog(null)}
      >
        {selectedLog && <LogDiagnosticContent log={selectedLog} />}
      </SlideOverDrawer>
    </div>
  );
}
