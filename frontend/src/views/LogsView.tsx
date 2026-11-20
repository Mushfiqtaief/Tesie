import { memo, useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  Copy,
  FileJson,
  RefreshCw,
  Repeat,
  Search,
  Sparkles,
  Trash2,
} from "lucide-react";
import { api } from "../api";
import {
  LogDiagnosticContent,
  buildDiagnosticReport,
  extractLegMeta,
  parseAttemptLegs,
} from "../components/LogDiagnosticContent";
import { ProviderIcon } from "../components/ProviderIcon";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  Card,
  EmptyState,
  INPUT_CLASS,
  SlideOverDrawer,
} from "../components/UiPrimitives";
import type { ProxyWithQuota, UsageLogRecord } from "../types";
import {
  formatClockTime,
  formatCompactNumber,
  formatCooldownReason,
  formatMs,
  formatTps,
  getProviderLogo,
  providerDisplayName,
  statusColorStyle,
} from "../utils/format";

export const LogsView = memo(function LogsView({
  proxies,
  refreshSignal,
  onNotify,
}: {
  proxies: ProxyWithQuota[];
  refreshSignal: number;
  onNotify: (msg: string, isErr?: boolean) => void;
}) {
  const [logs, setLogs] = useState<UsageLogRecord[]>([]);
  const [initialLoading, setInitialLoading] = useState(true);
  const [fetching, setFetching] = useState(false);
  const [statusFilter, setStatusFilter] = useState("");
  const [proxyFilter, setProxyFilter] = useState("");
  const [modelFilter, setModelFilter] = useState("");
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [limit, setLimit] = useState(100);
  const [selected, setSelected] = useState<UsageLogRecord | null>(null);
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  useEffect(() => {
    const id = window.setTimeout(() => setDebouncedSearch(search), search ? 250 : 0);
    return () => window.clearTimeout(id);
  }, [search]);

  const loadLogs = useCallback(async () => {
    setFetching(true);
    try {
      const data = await api.getLogs({
        limit,
        status: statusFilter || undefined,
        proxy: proxyFilter || undefined,
        model: modelFilter || undefined,
        q: debouncedSearch.trim() || undefined,
      });
      setLogs(data);
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to load logs", true);
    } finally {
      setFetching(false);
      setInitialLoading(false);
    }
  }, [debouncedSearch, limit, modelFilter, onNotify, proxyFilter, statusFilter]);

  useEffect(() => {
    void loadLogs();
  }, [loadLogs, refreshSignal]);

  function copyWithFeedback(key: string, text: string, toastMsg?: string) {
    void navigator.clipboard.writeText(text);
    setCopiedKey(key);
    if (toastMsg) {
      onNotify(toastMsg);
    }
    setTimeout(() => {
      setCopiedKey((cur) => (cur === key ? null : cur));
    }, 1600);
  }

  function handleCopyFilteredLogs() {
    if (logs.length === 0) return;
    const payload = JSON.stringify(logs.map(buildDiagnosticReport), null, 2);
    copyWithFeedback(
      "batch-json",
      payload,
      `Copied ${logs.length} detailed log ${logs.length === 1 ? "entry" : "entries"} (JSON)`,
    );
  }

  async function handleClear() {
    if (!window.confirm("Delete all stored request logs from SQLite?")) return;
    try {
      const res = await api.clearLogs();
      onNotify(`Cleared ${res.deleted} log entries`);
      setSelected(null);
      await loadLogs();
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to clear logs", true);
    }
  }

  const distinctModels = useMemo(
    () => Array.from(new Set(logs.map((l) => l.model))).sort(),
    [logs],
  );
  const { okCount, errCount } = useMemo(() => {
    let ok = 0;
    for (const l of logs) {
      if (l.status_code >= 200 && l.status_code < 300) ok += 1;
    }
    return { okCount: ok, errCount: logs.length - ok };
  }, [logs]);
  const parsedLogRows = useMemo(
    () =>
      logs.map((log) => {
        const legs = parseAttemptLegs(log.attempt_trace);
        return {
          log,
          st: statusColorStyle(log.status_code),
          ok: log.status_code >= 200 && log.status_code < 300,
          attempts: log.attempt_count ?? 1,
          legs,
          legMeta: extractLegMeta(legs),
          rowCopyId: `row-${log.id}`,
          cleanErr: formatCooldownReason(log.error_detail) || log.error_kind || "",
        };
      }),
    [logs],
  );

  return (
    <div className="space-y-4">
      {initialLoading ? (
        <TableSkeleton rows={8} />
      ) : (
        <Card noPadding className="overflow-hidden">
          {/* Unified Filter & Summary Header */}
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] px-4 py-3">
            <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--text-muted)] tabular-nums">
              <span className="font-mono font-semibold text-[var(--text-primary)]">
                {logs.length}
              </span>{" "}
              entries
              {logs.length > 0 && (
                <>
                  <span>&middot;</span>
                  <span>
                    <span className="font-mono font-semibold text-[var(--success)]">
                      {okCount}
                    </span>{" "}
                    OK
                  </span>
                  {errCount > 0 && (
                    <>
                      <span>&middot;</span>
                      <span>
                        <span className="font-mono font-semibold text-[var(--danger)]">
                          {errCount}
                        </span>{" "}
                        failed / 429
                      </span>
                    </>
                  )}
                </>
              )}
            </div>

            <div className="flex flex-wrap items-center gap-1.5">
              <Button
                variant="secondary"
                size="sm"
                onClick={handleCopyFilteredLogs}
                disabled={logs.length === 0}
                title="Copy all visible logs as structured diagnostic JSON"
              >
                {copiedKey === "batch-json" ? (
                  <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                ) : (
                  <FileJson className="h-3.5 w-3.5 text-[var(--accent)]" />
                )}
                {copiedKey === "batch-json" ? "Copied JSON" : `Copy JSON (${logs.length})`}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void loadLogs()}
                disabled={fetching}
              >
                <RefreshCw className={`h-3.5 w-3.5 ${fetching ? "animate-spin" : ""}`} />
                Refresh
              </Button>
              <Button variant="ghost" size="sm" onClick={() => void handleClear()}>
                <Trash2 className="h-3.5 w-3.5 text-[var(--danger)]" />
                Clear
              </Button>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-2 border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 px-4 py-2.5 sm:grid-cols-2 lg:grid-cols-12">
            <div className="relative sm:col-span-2 lg:col-span-4">
              <Search className="pointer-events-none absolute top-2 left-2.5 h-3.5 w-3.5 text-[var(--text-muted)]" />
              <input
                type="text"
                placeholder="Search request ID, ses_..., model, IP, or error..."
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className={`${INPUT_CLASS} py-1 pl-8`}
              />
            </div>

            <div className="lg:col-span-2">
              <select
                value={statusFilter}
                onChange={(e) => setStatusFilter(e.target.value)}
                className={`${INPUT_CLASS} py-1`}
              >
                <option value="">All Statuses</option>
                <option value="2xx">2xx Success</option>
                <option value="429">429 Rate Limited</option>
                <option value="502">502 Dial / EOF Error</option>
                <option value="403">403 Blocked / Forbidden</option>
                <option value="error">All Errors (!2xx)</option>
              </select>
            </div>

            <div className="lg:col-span-2">
              <select
                value={proxyFilter}
                onChange={(e) => setProxyFilter(e.target.value)}
                className={`${INPUT_CLASS} py-1`}
              >
                <option value="">All Proxies</option>
                {proxies.map((p) => (
                  <option key={p.id} value={p.name}>
                    {p.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="flex items-center gap-2 sm:col-span-2 lg:col-span-4">
              <div className="min-w-0 flex-1">
                <select
                  value={modelFilter}
                  onChange={(e) => setModelFilter(e.target.value)}
                  className={`${INPUT_CLASS} py-1`}
                >
                  <option value="">All Models</option>
                  {distinctModels.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              </div>
              <div className="w-20 shrink-0">
                <select
                  value={limit}
                  onChange={(e) => setLimit(Number(e.target.value))}
                  className={`${INPUT_CLASS} py-1`}
                >
                  <option value={50}>50</option>
                  <option value={100}>100</option>
                  <option value={250}>250</option>
                  <option value={500}>500</option>
                </select>
              </div>
            </div>
          </div>

          {logs.length === 0 ? (
            <div className="p-5">
              <EmptyState
                icon={<Sparkles className="h-5 w-5" />}
                title="No matching request logs"
                description="Send requests via /v1/chat/completions or Playground, or broaden your filters."
              />
            </div>
          ) : (
            <div
              key={`${statusFilter}-${proxyFilter}-${modelFilter}`}
              className="animate-tab-switch smooth-scroll overflow-x-auto"
            >
              <table className="w-full text-left text-xs">
                <thead>
                  <tr className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 text-[11px] font-medium text-[var(--text-muted)]">
                    <th className="py-2.5 pl-4 pr-2 whitespace-nowrap">Time</th>
                    <th className="py-2.5 px-2 whitespace-nowrap">Status</th>
                    <th className="py-2.5 px-2">Model / Wire</th>
                    <th className="py-2.5 px-2">Route / Exit IP</th>
                    <th className="py-2.5 px-2 text-right whitespace-nowrap">Speed</th>
                    <th className="py-2.5 px-2 text-right whitespace-nowrap">TTFT</th>
                    <th className="py-2.5 px-2 text-right whitespace-nowrap">Tokens</th>
                    <th className="py-2.5 px-2 text-right whitespace-nowrap">Latency</th>
                    <th className="py-2.5 px-2">Diagnostic Note</th>
                    <th className="py-2.5 pl-1 pr-3 text-right">Copy</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--border-subtle)] tabular-nums">
                  {parsedLogRows.map(
                    ({
                      log,
                      st,
                      ok,
                      attempts,
                      legMeta,
                      rowCopyId,
                      cleanErr,
                    }) => (
                      <tr
                        key={log.id}
                        onClick={() => setSelected(log)}
                        className="cv-row tr-row-interactive cursor-pointer hover:bg-[var(--bg-hover)]/65"
                      >
                        <td className="py-2.5 pl-4 pr-2 font-mono whitespace-nowrap text-[var(--text-muted)]">
                          {formatClockTime(log.created_at)}
                        </td>
                        <td className="py-2.5 px-2 whitespace-nowrap">
                          <div className="inline-flex items-center gap-1.5">
                            <span
                              className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 font-mono text-[11px] font-medium"
                              style={{
                                backgroundColor: st.bg,
                                color: st.fg,
                                borderColor: st.border,
                              }}
                            >
                              {ok ? (
                                <CheckCircle2 className="h-2.5 w-2.5" />
                              ) : (
                                <AlertTriangle className="h-2.5 w-2.5" />
                              )}
                              {log.status_code}
                            </span>
                            {attempts > 1 && (
                              <span
                                className="inline-flex items-center gap-0.5 rounded border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[var(--warning)]"
                                title={`${attempts} upstream attempts (failover)`}
                              >
                                <Repeat className="h-2.5 w-2.5" />
                                &times;{attempts}
                              </span>
                            )}
                          </div>
                        </td>
                        <td className="py-2.5 px-2">
                          <div className="flex items-center gap-2 min-w-0">
                            <ProviderIcon
                              src={getProviderLogo(log.provider_id)}
                              name={providerDisplayName(log.provider_id)}
                              size={20}
                            />
                            <div className="min-w-0">
                              <div
                                className="max-w-[175px] truncate font-mono font-medium text-[var(--text-primary)]"
                                title={log.model}
                              >
                                {log.model}
                              </div>
                              <div className="flex items-center gap-1.5 text-[10px] text-[var(--text-muted)]">
                                <span>{providerDisplayName(log.provider_id)}</span>
                                {log.wire_format && (
                                  <>
                                    <span>&middot;</span>
                                    <span className="font-mono uppercase">{log.wire_format}</span>
                                  </>
                                )}
                                {legMeta.reasoningLevel && (
                                  <>
                                    <span>&middot;</span>
                                    <span className="font-mono text-[var(--accent)]">
                                      {legMeta.reasoningLevel}
                                    </span>
                                  </>
                                )}
                              </div>
                            </div>
                          </div>
                        </td>
                        <td className="py-2.5 px-2 font-mono">
                          {legMeta.accountEmail && (
                            <div
                              className="max-w-[175px] truncate font-sans text-[11px] font-medium text-[var(--accent)]"
                              title={legMeta.accountEmail}
                            >
                              {legMeta.accountEmail}
                            </div>
                          )}
                          <div
                            className="max-w-[175px] truncate text-[var(--text-secondary)]"
                            title={log.proxy_name}
                          >
                            {log.proxy_name}
                          </div>
                          <div className="text-[11px] text-[var(--token-cache)]">
                            {log.egress_ip || (
                              <span className="text-[var(--text-muted)]">No IP</span>
                            )}
                          </div>
                        </td>
                        <td className="py-2.5 px-2 text-right font-mono whitespace-nowrap font-semibold text-[var(--speed)]">
                          {formatTps(log.tokens_per_sec)}
                        </td>
                        <td className="py-2.5 px-2 text-right font-mono whitespace-nowrap text-[var(--text-secondary)]">
                          {formatMs(log.ttft_ms)}
                        </td>
                        <td
                          className="py-2.5 px-2 text-right font-mono whitespace-nowrap"
                          title={`Uncached In: ${log.prompt_tokens} / Cached Read: ${log.cached_tokens} / Output: ${log.completion_tokens}`}
                        >
                          <span className="text-[var(--token-in)]">
                            {formatCompactNumber(log.prompt_tokens)}
                          </span>{" "}
                          <span className="text-[var(--text-muted)]">/</span>{" "}
                          <span className="text-[var(--token-cache)]">
                            {formatCompactNumber(log.cached_tokens)}
                          </span>{" "}
                          <span className="text-[var(--text-muted)]">/</span>{" "}
                          <span className="text-[var(--token-out)]">
                            {formatCompactNumber(log.completion_tokens)}
                          </span>
                        </td>
                        <td className="py-2.5 px-2 text-right font-mono whitespace-nowrap text-[var(--text-muted)]">
                          {formatMs(log.latency_ms)}
                        </td>
                        <td
                          className="max-w-[190px] truncate py-2.5 px-2 text-[11px]"
                          title={log.error_detail || undefined}
                        >
                          {cleanErr ? (
                            <span className="font-medium text-[var(--danger)]">{cleanErr}</span>
                          ) : log.session_id ? (
                            <span className="font-mono text-[10px] text-[var(--text-muted)]">
                              {log.session_id.slice(0, 14)}&hellip;
                            </span>
                          ) : (
                            <span className="text-[var(--text-muted)]">&mdash;</span>
                          )}
                        </td>
                        <td className="py-2.5 pl-1 pr-3 text-right">
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              copyWithFeedback(
                                rowCopyId,
                                JSON.stringify(buildDiagnosticReport(log), null, 2),
                                `Copied log #${log.id} diagnostic JSON`,
                              );
                            }}
                            className="inline-flex items-center justify-center rounded-md border border-transparent p-1.5 text-[var(--text-muted)] transition-colors hover:border-[var(--border-subtle)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                            title={`Copy log #${log.id} diagnostic JSON`}
                            aria-label={`Copy log #${log.id}`}
                          >
                            {copiedKey === rowCopyId ? (
                              <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                            ) : (
                              <Copy className="h-3.5 w-3.5" />
                            )}
                          </button>
                        </td>
                      </tr>
                    ),
                  )}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      )}

      {/* Right Slide-Over Detailed Diagnostic Inspector */}
      <SlideOverDrawer
        open={!!selected}
        title={selected ? `Request Diagnostic #${selected.id}` : ""}
        subtitle={selected?.request_id}
        onClose={() => setSelected(null)}
      >
        {selected && <LogDiagnosticContent log={selected} onNotify={onNotify} />}
      </SlideOverDrawer>
    </div>
  );
});
