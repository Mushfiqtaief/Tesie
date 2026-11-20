import { memo, useDeferredValue, useEffect, useMemo, useState } from "react";
import { useLiveClock } from "../hooks/useLiveClock";
import {
  Activity,
  AlertTriangle,
  CheckSquare,
  Clock,
  Eye,
  EyeOff,
  Hash,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Square,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  Card,
  EmptyState,
  INPUT_CLASS,
  Modal,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  CreateProxyPayload,
  ProxyWithQuota,
  UpdateProxyPayload,
} from "../types";
import { formatTimeAgo, maskProxyUrl } from "../utils/format";

type ProxyFilter = "all" | "healthy" | "timeout";

function parseNodeIndex(name: string): number | null {
  const m = name.trim().match(/^node\s*[-_#]?\s*(\d+)$/i);
  if (!m) return null;
  const n = Number(m[1]);
  return Number.isFinite(n) ? n : null;
}

function formatProxyTypeLabel(rawType: string): string {
  const clean = (rawType || "socks5").trim().toLowerCase();
  return clean === "shadowsocks" ? "SS" : clean.toUpperCase();
}

function LatencyCell({
  alive,
  latencyMs,
}: {
  alive: boolean;
  latencyMs: number | null;
}) {
  if (!alive) {
    return (
      <StatusPill tone="danger" dot>
        Timeout
      </StatusPill>
    );
  }
  if (latencyMs === null || latencyMs <= 0) {
    return <span className="font-mono text-xs text-[var(--text-muted)]">Pending…</span>;
  }

  let colorClass = "text-[var(--success)]";
  if (latencyMs >= 450) {
    colorClass = "text-[var(--danger)]";
  } else if (latencyMs >= 180) {
    colorClass = "text-[var(--warning)]";
  }

  return (
    <span className={`font-mono text-xs font-semibold tabular-nums ${colorClass}`}>
      {latencyMs}ms
    </span>
  );
}

const AutoTestCountdown = memo(function AutoTestCountdown({
  lastAutoTestAt,
  testIntervalSec,
}: {
  lastAutoTestAt: number | null;
  testIntervalSec: number;
}) {
  const nowMs = useLiveClock(1000, lastAutoTestAt !== null && testIntervalSec > 0);
  const nextAutoTestInSec =
    lastAutoTestAt !== null
      ? Math.max(0, Math.ceil((lastAutoTestAt + testIntervalSec * 1000 - nowMs) / 1000))
      : null;
  return <>{nextAutoTestInSec !== null ? ` (${nextAutoTestInSec}s)` : ""}</>;
});

export const ProxyPoolView = memo(function ProxyPoolView({
  proxies,
  testIntervalSec,
  lastAutoTestAt,
  loading,
  nowMs: nowMsProp,
  testingAll,
  onCreateProxy,
  onUpdateProxy,
  onImportProxies,
  onDeleteProxy,
  onBatchDeleteProxies,
  onNormalizeProxyNames,
  onTestAll,
  onTestOne,
  onUpdateTestInterval,
  onOpenProviders,
}: {
  proxies: ProxyWithQuota[];
  testIntervalSec: number;
  lastAutoTestAt: number | null;
  loading: boolean;
  nowMs?: number;
  testingAll: boolean;
  onCreateProxy: (payload: CreateProxyPayload) => Promise<void>;
  onUpdateProxy: (id: string, payload: UpdateProxyPayload) => Promise<void>;
  onImportProxies: (urlsText: string) => Promise<void>;
  onDeleteProxy: (id: string, name: string) => Promise<void>;
  onBatchDeleteProxies: (ids: string[]) => Promise<void>;
  onNormalizeProxyNames?: () => Promise<void>;
  onTestAll: () => Promise<void>;
  onTestOne: (id: string) => Promise<void>;
  onUpdateTestInterval: (seconds: number) => Promise<void>;
  onOpenProviders: () => void;
}) {
  const internalNowMs = useLiveClock(15_000, nowMsProp === undefined);
  const nowMs = nowMsProp ?? internalNowMs;

  const [addOpen, setAddOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [editingProxy, setEditingProxy] = useState<ProxyWithQuota | null>(null);
  const [revealUrls, setRevealUrls] = useState(false);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const deferredSearch = useDeferredValue(search);
  const [filter, setFilter] = useState<ProxyFilter>("all");
  const [showAllRows, setShowAllRows] = useState(false);

  // Multi-select batch deletion state
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [batchDeleting, setBatchDeleting] = useState(false);
  const [normalizing, setNormalizing] = useState(false);

  // Configurable TesieCore auto-test interval (default 15s)
  const [intervalInput, setIntervalInput] = useState(String(testIntervalSec || 15));
  const [savingInterval, setSavingInterval] = useState(false);

  useEffect(() => {
    setIntervalInput(String(testIntervalSec || 15));
  }, [testIntervalSec]);

  // Prune selectedIds if proxies list changed
  useEffect(() => {
    const validIds = new Set(proxies.filter((p) => !p.is_direct).map((p) => p.id));
    setSelectedIds((prev) => {
      const next = new Set<string>();
      for (const id of prev) {
        if (validIds.has(id)) next.add(id);
      }
      return next.size === prev.size ? prev : next;
    });
  }, [proxies]);

  const nextDefaultNodeName = useMemo(() => {
    let maxIdx = 0;
    for (const p of proxies) {
      if (p.is_direct) continue;
      const n = parseNodeIndex(p.name);
      if (n !== null && n > maxIdx) maxIdx = n;
    }
    return `NODE - ${maxIdx + 1}`;
  }, [proxies]);

  // Add proxy modal form (structured fields + raw URL/JSON option)
  const [addInputMode, setAddInputMode] = useState<"fields" | "url" | "json">("fields");
  const [addName, setAddName] = useState("");
  const [addType, setAddType] = useState("socks5");
  const [addHost, setAddHost] = useState("");
  const [addPort, setAddPort] = useState("1080");
  const [addUsername, setAddUsername] = useState("");
  const [addPassword, setAddPassword] = useState("");
  const [showAddPassword, setShowAddPassword] = useState(false);
  const [proxyUrl, setProxyUrl] = useState("");
  const [rawJson, setRawJson] = useState("");
  const [submitting, setSubmitting] = useState(false);

  // Edit proxy modal form
  const [editName, setEditName] = useState("");
  const [editType, setEditType] = useState("socks5");
  const [editHost, setEditHost] = useState("");
  const [editPort, setEditPort] = useState("");
  const [editUsername, setEditUsername] = useState("");
  const [editPassword, setEditPassword] = useState("");
  const [showEditPassword, setShowEditPassword] = useState(false);
  const [savingEdit, setSavingEdit] = useState(false);

  // Bulk import form
  const [bulkText, setBulkText] = useState("");

  const { externalNodes, healthyCount, timeoutCount, distinctIps, avgDelay } = useMemo(() => {
    const ext = proxies.filter((p) => !p.is_direct);
    let healthy = 0;
    const ipSet = new Set<string>();
    let delaySum = 0;
    let delayCount = 0;
    for (const p of ext) {
      if (p.alive) {
        healthy += 1;
        if (p.latency_ms !== null && p.latency_ms > 0) {
          delaySum += p.latency_ms;
          delayCount += 1;
        }
      }
      if (p.ip && p.ip.length > 0) ipSet.add(p.ip);
    }
    return {
      externalNodes: ext,
      healthyCount: healthy,
      timeoutCount: ext.length - healthy,
      distinctIps: ipSet.size,
      avgDelay: delayCount > 0 ? Math.round(delaySum / delayCount) : null,
    };
  }, [proxies]);

  const filtered = useMemo(() => {
    const qLower = deferredSearch.trim().toLowerCase();
    return externalNodes.filter((p) => {
      if (filter === "healthy" && !p.alive) return false;
      if (filter === "timeout" && p.alive) return false;
      if (!qLower) return true;
      return (
        p.name.toLowerCase().includes(qLower) ||
        p.proxy_type.toLowerCase().includes(qLower) ||
        (p.ip || "").toLowerCase().includes(qLower) ||
        (p.host || "").toLowerCase().includes(qLower) ||
        String(p.port || "").includes(qLower)
      );
    });
  }, [deferredSearch, externalNodes, filter]);

  const visible = useMemo(
    () => (showAllRows ? filtered : filtered.slice(0, 80)),
    [filtered, showAllRows],
  );
  const allVisibleSelected = useMemo(
    () => visible.length > 0 && visible.every((p) => selectedIds.has(p.id)),
    [selectedIds, visible],
  );

  if (loading && proxies.length === 0) {
    return <TableSkeleton rows={8} />;
  }

  function openAddModal() {
    setAddName(nextDefaultNodeName);
    setAddType("socks5");
    setAddHost("");
    setAddPort("1080");
    setAddUsername("");
    setAddPassword("");
    setShowAddPassword(false);
    setProxyUrl("");
    setRawJson("");
    setAddInputMode("fields");
    setAddOpen(true);
  }

  function openEditModal(p: ProxyWithQuota) {
    setEditingProxy(p);
    setEditName(p.name);
    setEditType(
      ["socks5", "socks4", "http", "https", "ss"].includes(p.proxy_type.toLowerCase())
        ? p.proxy_type.toLowerCase()
        : "socks5",
    );
    setEditHost(p.host || p.ip || "");
    setEditPort(p.port ? String(p.port) : "1080");
    setEditUsername(p.username || "");
    setEditPassword(p.password || "");
    setShowEditPassword(false);
  }

  async function handleAddSubmit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    try {
      const cleanName = addName.trim() || nextDefaultNodeName;
      if (addInputMode === "fields") {
        await onCreateProxy({
          name: cleanName,
          proxy_type: addType,
          host: addHost.trim(),
          port: Number(addPort) || 1080,
          username: addUsername.trim() || undefined,
          password: addPassword || undefined,
          enabled: true,
        });
      } else if (addInputMode === "url") {
        await onCreateProxy({
          name: cleanName,
          proxy_url: proxyUrl.trim(),
          enabled: true,
        });
      } else {
        await onCreateProxy({
          name: cleanName,
          raw_outbound_json: rawJson.trim(),
          enabled: true,
        });
      }
      setAddOpen(false);
    } finally {
      setSubmitting(false);
    }
  }

  async function handleEditSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!editingProxy) return;
    setSavingEdit(true);
    try {
      await onUpdateProxy(editingProxy.id, {
        name: editName.trim() || editingProxy.name,
        proxy_type: editType,
        host: editHost.trim(),
        port: Number(editPort) || 1080,
        username: editUsername.trim() ? editUsername.trim() : null,
        password: editPassword ? editPassword : null,
      });
      setEditingProxy(null);
    } finally {
      setSavingEdit(false);
    }
  }

  async function handleImportSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!bulkText.trim()) return;
    setSubmitting(true);
    try {
      await onImportProxies(bulkText);
      setBulkText("");
      setImportOpen(false);
    } finally {
      setSubmitting(false);
    }
  }

  async function handleTestSingle(id: string) {
    setTestingId(id);
    setTestError(null);
    try {
      await onTestOne(id);
    } catch (err) {
      setTestError(err instanceof Error ? err.message : "Proxy delay test failed");
    } finally {
      setTestingId(null);
    }
  }

  async function applyInterval(sec: number) {
    const clamped = Math.max(5, Math.min(3600, Math.round(sec)));
    setIntervalInput(String(clamped));
    if (clamped === testIntervalSec) return;
    setSavingInterval(true);
    try {
      await onUpdateTestInterval(clamped);
    } finally {
      setSavingInterval(false);
    }
  }

  function toggleRow(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function toggleSelectVisible() {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (allVisibleSelected) {
        for (const p of visible) next.delete(p.id);
      } else {
        for (const p of visible) next.add(p.id);
      }
      return next;
    });
  }

  function selectAllFiltered() {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const p of filtered) next.add(p.id);
      return next;
    });
  }

  async function handleBatchDelete() {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;
    if (
      !window.confirm(
        `Delete ${ids.length} selected prox${ids.length === 1 ? "y" : "ies"} from the global pool?`,
      )
    ) {
      return;
    }
    setBatchDeleting(true);
    try {
      await onBatchDeleteProxies(ids);
      setSelectedIds(new Set());
    } finally {
      setBatchDeleting(false);
    }
  }

  async function handleNormalizeNames() {
    if (!onNormalizeProxyNames) return;
    setNormalizing(true);
    try {
      await onNormalizeProxyNames();
    } finally {
      setNormalizing(false);
    }
  }

  return (
    <div className="space-y-4">
      {testError && (
        <div className="flex items-start justify-between gap-3 rounded-xl border border-[color-mix(in_srgb,var(--danger)_40%,transparent)] bg-[var(--danger-soft)] px-4 py-3 text-xs text-[var(--danger)]">
          <div className="flex min-w-0 items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="min-w-0">
              <div className="font-semibold">Proxy Delay Test Error</div>
              <div className="mt-0.5 break-words">{testError}</div>
            </div>
          </div>
          <button
            type="button"
            onClick={() => setTestError(null)}
            className="rounded p-1 hover:bg-[var(--bg-surface)]/40"
            aria-label="Dismiss error"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}

      {/* Unified Pool Header, Auto-Test Controls, Search & Table Surface */}
      <Card noPadding className="overflow-hidden">
        {/* Top Row: Calm Inline Pool Telemetry & Actions */}
        <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
          <div className="flex flex-wrap items-center gap-2 text-xs text-[var(--text-muted)] tabular-nums">
            <span className="text-sm font-semibold text-[var(--text-primary)]">
              TesieCore Pool
            </span>
            <span>&middot;</span>
            <span>
              <span className="font-mono font-semibold text-[var(--success)]">
                {healthyCount}
              </span>{" "}
              healthy
            </span>
            {timeoutCount > 0 && (
              <>
                <span>&middot;</span>
                <span>
                  <span className="font-mono font-semibold text-[var(--danger)]">
                    {timeoutCount}
                  </span>{" "}
                  timeout
                </span>
              </>
            )}
            <span>&middot;</span>
            <span className="font-mono">{distinctIps} IPs</span>
            {avgDelay !== null && (
              <>
                <span>&middot;</span>
                <span className="font-mono">{avgDelay}ms avg</span>
              </>
            )}
            <span>&middot;</span>
            <button
              type="button"
              onClick={onOpenProviders}
              className="font-medium text-[var(--text-secondary)] hover:text-[var(--accent)] hover:underline"
            >
              Provider routing
            </button>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div className="flex items-center gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2 py-1 text-xs">
              <Clock className="h-3 w-3 text-[var(--text-muted)]" />
              <span className="text-[11px] text-[var(--text-muted)]">Auto</span>
              <input
                type="number"
                min={5}
                max={3600}
                value={intervalInput}
                disabled={savingInterval}
                onChange={(e) => setIntervalInput(e.target.value)}
                onBlur={() => {
                  const n = Number(intervalInput);
                  if (Number.isFinite(n) && n >= 5) {
                    void applyInterval(n);
                  } else {
                    setIntervalInput(String(testIntervalSec));
                  }
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    const n = Number(intervalInput);
                    if (Number.isFinite(n) && n >= 5) {
                      void applyInterval(n);
                    }
                  }
                }}
                className="w-11 rounded border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-1 py-0.5 text-center font-mono text-[11px] text-[var(--text-primary)] tabular-nums focus:border-[var(--accent)] focus:outline-none"
                aria-label="TesieCore auto-test interval in seconds"
              />
              <span className="font-mono text-[10px] text-[var(--text-muted)]">
                s
                <AutoTestCountdown
                  lastAutoTestAt={lastAutoTestAt}
                  testIntervalSec={testIntervalSec}
                />
              </span>
            </div>

            {onNormalizeProxyNames && externalNodes.length > 0 && (
              <Button
                variant="ghost"
                size="sm"
                disabled={normalizing}
                onClick={() => void handleNormalizeNames()}
                title="Re-sequence external proxy names to NODE - 1, NODE - 2…"
              >
                <Hash className="h-3.5 w-3.5" />
                Re-number
              </Button>
            )}

            <Button
              variant="secondary"
              size="sm"
              disabled={testingAll || externalNodes.length === 0}
              onClick={() => void onTestAll()}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${testingAll ? "animate-spin" : ""}`} />
              {testingAll ? "Testing…" : "Test all"}
            </Button>

            <Button variant="secondary" size="sm" onClick={() => setImportOpen(true)}>
              <Upload className="h-3.5 w-3.5" />
              Import
            </Button>

            <Button variant="primary" size="sm" onClick={openAddModal}>
              <Plus className="h-3.5 w-3.5" />
              Add proxy
            </Button>
          </div>
        </div>

        {/* Sub-Toolbar: Search, Filter & Details Toggle */}
        <div className="flex flex-wrap items-center justify-between gap-2.5 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 px-4 py-2">
          <div className="relative w-64 sm:w-80">
            <Search className="pointer-events-none absolute top-2 left-2.5 h-3.5 w-3.5 text-[var(--text-muted)]" />
            <input
              type="text"
              placeholder="Search NODE - 1, IP, port, or type…"
              value={search}
              onChange={(e) => {
                setSearch(e.target.value);
                setShowAllRows(false);
              }}
              className={`${INPUT_CLASS} py-1 pl-8`}
            />
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <div className="inline-flex rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5">
              {(
                [
                  { id: "all", label: `All (${externalNodes.length})` },
                  { id: "healthy", label: `Healthy (${healthyCount})` },
                  { id: "timeout", label: `Timeout (${timeoutCount})` },
                ] as Array<{ id: ProxyFilter; label: string }>
              ).map((f) => (
                <button
                  key={f.id}
                  type="button"
                  onClick={() => {
                    setFilter(f.id);
                    setShowAllRows(false);
                  }}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    filter === f.id
                      ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                      : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  {f.label}
                </button>
              ))}
            </div>

            <Button
              variant="ghost"
              size="sm"
              onClick={() => setRevealUrls((r) => !r)}
              title={revealUrls ? "Hide credentials & URLs" : "Show endpoint URLs"}
            >
              {revealUrls ? (
                <>
                  <EyeOff className="h-3.5 w-3.5" />
                  Hide URLs
                </>
              ) : (
                <>
                  <Eye className="h-3.5 w-3.5" />
                  Show URLs
                </>
              )}
            </Button>
          </div>
        </div>

        {/* Contextual Batch Selection Action Strip */}
        {selectedIds.size > 0 && (
          <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-subtle)] bg-[var(--accent-soft)]/35 px-4 py-2">
            <div className="flex flex-wrap items-center gap-2.5 text-xs">
              <StatusPill tone="accent" dot>
                {selectedIds.size} selected
              </StatusPill>
              {selectedIds.size < filtered.length && (
                <button
                  type="button"
                  onClick={selectAllFiltered}
                  className="inline-flex items-center gap-1 font-medium text-[var(--accent)] hover:underline"
                >
                  <CheckSquare className="h-3.5 w-3.5" />
                  Select all {filtered.length} filtered
                </button>
              )}
              <button
                type="button"
                onClick={() => setSelectedIds(new Set())}
                className="inline-flex items-center gap-1 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
              >
                <Square className="h-3.5 w-3.5" />
                Clear selection
              </button>
            </div>

            <Button
              variant="danger"
              size="sm"
              disabled={batchDeleting}
              onClick={() => void handleBatchDelete()}
            >
              <Trash2 className="h-3.5 w-3.5" />
              {batchDeleting ? "Deleting…" : `Delete selected (${selectedIds.size})`}
            </Button>
          </div>
        )}

        {filtered.length === 0 ? (
          <div className="border-t border-[var(--border-subtle)] p-6">
            <EmptyState
              title="No matching proxies"
              description="Add a proxy node or batch-import URLs — Tesie names them NODE - 1, NODE - 2… automatically."
              action={
                <Button variant="primary" size="sm" onClick={openAddModal}>
                  <Plus className="h-3.5 w-3.5" />
                  Add proxy
                </Button>
              }
            />
          </div>
        ) : (
          <div
            key={filter}
            className="animate-tab-switch overflow-x-auto border-t border-[var(--border-subtle)]"
          >
            <table className="w-full text-left text-xs">
              <thead>
                <tr className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 text-[11px] font-medium text-[var(--text-muted)]">
                  <th className="w-9 py-2.5 pl-4 pr-1">
                    <input
                      type="checkbox"
                      checked={allVisibleSelected}
                      onChange={toggleSelectVisible}
                      aria-label="Select visible proxies"
                      className="h-3.5 w-3.5 rounded accent-[var(--accent)]"
                    />
                  </th>
                  <th className="py-2.5 px-2">Name</th>
                  <th className="py-2.5 px-3">Type</th>
                  <th className="py-2.5 px-3">Endpoint / Egress IP</th>
                  <th className="py-2.5 px-3 text-right">Latency</th>
                  <th className="py-2.5 pl-3 pr-4 text-right">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-subtle)] tabular-nums">
                {visible.map((p) => {
                  const typeLabel = formatProxyTypeLabel(p.proxy_type);
                  const isTesting = testingId === p.id;
                  const checked = selectedIds.has(p.id);
                  const hostPort = p.host
                    ? `${p.host}${p.port ? `:${p.port}` : ""}`
                    : p.ip || "—";

                  return (
                    <tr
                      key={p.id}
                      className={`cv-row group tr-row-interactive ${
                        checked
                          ? "bg-[var(--accent-soft)]/30"
                          : "hover:bg-[var(--bg-hover)]/60"
                      }`}
                    >
                      <td className="py-2.5 pl-4 pr-1">
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={() => toggleRow(p.id)}
                          aria-label={`Select ${p.name}`}
                          className="h-3.5 w-3.5 rounded accent-[var(--accent)]"
                        />
                      </td>

                      <td className="py-2.5 px-2">
                        <div className="flex items-center gap-2.5">
                          <span
                            className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                              p.alive ? "bg-[var(--success)]" : "bg-[var(--danger)]"
                            }`}
                            title={
                              p.alive
                                ? "Healthy in TesieCore"
                                : "Failed TesieCore check — bypassed until healthy"
                            }
                          />
                          <div className="min-w-0">
                            <div className="flex items-center gap-1.5">
                              <span className="font-mono font-semibold text-[var(--text-primary)]">
                                {p.name}
                              </span>
                              {p.username && (
                                <span className="font-mono text-[10px] text-[var(--text-muted)]">
                                  · auth
                                </span>
                              )}
                            </div>
                            {revealUrls && (
                              <div className="mt-0.5 max-w-md truncate font-mono text-[11px] text-[var(--text-muted)]">
                                {maskProxyUrl(p.proxy_url)}
                              </div>
                            )}
                          </div>
                        </div>
                      </td>

                      <td className="py-2.5 px-3 font-mono text-[11px] whitespace-nowrap text-[var(--text-secondary)]">
                        {typeLabel}
                      </td>

                      <td className="py-2.5 px-3 font-mono whitespace-nowrap">
                        <div className="text-[var(--text-primary)]">{hostPort}</div>
                        {p.egress_ip && p.egress_ip !== p.host && (
                          <div className="text-[10px] text-[var(--text-muted)]">
                            Exit {p.egress_ip}
                          </div>
                        )}
                      </td>

                      <td className="py-2.5 px-3 text-right whitespace-nowrap">
                        <div>
                          <LatencyCell alive={p.alive} latencyMs={p.latency_ms} />
                        </div>
                        {p.last_probed_at && (
                          <div className="text-[10px] text-[var(--text-muted)]">
                            {formatTimeAgo(p.last_probed_at, nowMs)}
                          </div>
                        )}
                      </td>

                      <td className="py-2.5 pl-3 pr-4 text-right whitespace-nowrap">
                        <div className="inline-flex items-center gap-0.5">
                          <button
                            type="button"
                            disabled={isTesting}
                            onClick={() => void handleTestSingle(p.id)}
                            className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)] disabled:opacity-50"
                            title="Run TesieCore latency test on this proxy"
                          >
                            <Activity
                              className={`h-3 w-3 ${isTesting ? "animate-pulse text-[var(--accent)]" : ""}`}
                            />
                            Test
                          </button>
                          <button
                            type="button"
                            onClick={() => openEditModal(p)}
                            className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                            title="Edit proxy name, IP, port, type & credentials"
                          >
                            <Pencil className="h-3 w-3" />
                            Edit
                          </button>
                          <button
                            type="button"
                            onClick={() => void onDeleteProxy(p.id, p.name)}
                            className="rounded-md p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--danger-soft)] hover:text-[var(--danger)]"
                            title="Delete proxy from pool"
                            aria-label={`Delete ${p.name}`}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>

            {!showAllRows && filtered.length > 80 && (
              <div className="border-t border-[var(--border-subtle)] p-3 text-center">
                <Button variant="secondary" size="sm" onClick={() => setShowAllRows(true)}>
                  Show all {filtered.length} proxies
                </Button>
              </div>
            )}
          </div>
        )}
      </Card>

      {/* Add Proxy Modal */}
      <Modal
        open={addOpen}
        onClose={() => setAddOpen(false)}
        title="Add Proxy Node"
        subtitle="Add a SOCKS4, SOCKS5, HTTP, HTTPS, or Shadowsocks node to the TesieCore global pool"
      >
        <form onSubmit={handleAddSubmit} className="space-y-4">
          <div className="inline-flex rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5 text-xs">
            {(
              [
                { id: "fields", label: "Host & Port" },
                { id: "url", label: "Proxy URL" },
                { id: "json", label: "Outbound JSON" },
              ] as const
            ).map((tab) => (
              <button
                key={tab.id}
                type="button"
                onClick={() => setAddInputMode(tab.id)}
                className={`rounded-md px-3 py-1 font-medium transition-colors ${
                  addInputMode === tab.id
                    ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Node name
              </label>
              <input
                type="text"
                placeholder={nextDefaultNodeName}
                value={addName}
                onChange={(e) => setAddName(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>

            {addInputMode === "fields" && (
              <div>
                <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                  Protocol type
                </label>
                <select
                  value={addType}
                  onChange={(e) => setAddType(e.target.value)}
                  className={`${INPUT_CLASS} font-mono`}
                >
                  <option value="socks5">SOCKS5</option>
                  <option value="socks4">SOCKS4</option>
                  <option value="http">HTTP</option>
                  <option value="https">HTTPS</option>
                  <option value="ss">Shadowsocks (SS)</option>
                </select>
              </div>
            )}
          </div>

          {addInputMode === "fields" ? (
            <>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                <div className="sm:col-span-2">
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    IP address or hostname
                  </label>
                  <input
                    required
                    type="text"
                    placeholder="198.51.100.12"
                    value={addHost}
                    onChange={(e) => setAddHost(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Port
                  </label>
                  <input
                    required
                    type="number"
                    min={1}
                    max={65535}
                    placeholder="1080"
                    value={addPort}
                    onChange={(e) => setAddPort(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
              </div>

              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    {addType === "ss" ? "Cipher (optional)" : "Username (optional)"}
                  </label>
                  <input
                    type="text"
                    placeholder={addType === "ss" ? "aes-256-gcm" : "Leave blank if none"}
                    value={addUsername}
                    onChange={(e) => setAddUsername(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Password (optional)
                  </label>
                  <div className="relative">
                    <input
                      type={showAddPassword ? "text" : "password"}
                      placeholder="Leave blank if none"
                      value={addPassword}
                      onChange={(e) => setAddPassword(e.target.value)}
                      className={`${INPUT_CLASS} pr-9 font-mono`}
                    />
                    <button
                      type="button"
                      onClick={() => setShowAddPassword((s) => !s)}
                      className="absolute top-2 right-2 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                      aria-label="Toggle password visibility"
                    >
                      {showAddPassword ? (
                        <EyeOff className="h-4 w-4" />
                      ) : (
                        <Eye className="h-4 w-4" />
                      )}
                    </button>
                  </div>
                </div>
              </div>
            </>
          ) : addInputMode === "url" ? (
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Proxy URL
              </label>
              <input
                required
                type="text"
                placeholder="socks5://user:pass@198.51.100.12:1080"
                value={proxyUrl}
                onChange={(e) => setProxyUrl(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
          ) : (
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                TesieCore outbound JSON
              </label>
              <textarea
                required
                rows={4}
                placeholder='{"type":"socks5","server":"198.51.100.12","port":1080}'
                value={rawJson}
                onChange={(e) => setRawJson(e.target.value)}
                className={`${INPUT_CLASS} font-mono text-xs`}
              />
            </div>
          )}

          <div className="flex justify-end gap-2 pt-2">
            <Button type="button" variant="secondary" onClick={() => setAddOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={submitting}>
              {submitting ? "Adding…" : "Add to pool"}
            </Button>
          </div>
        </form>
      </Modal>

      {/* Edit Proxy Modal */}
      <Modal
        open={editingProxy !== null}
        onClose={() => setEditingProxy(null)}
        title={`Edit ${editingProxy?.name ?? "Proxy"}`}
        subtitle="Update proxy name, protocol type, IP/hostname, port, or credentials"
      >
        <form onSubmit={handleEditSubmit} className="space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Node name
              </label>
              <input
                required
                type="text"
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Protocol type
              </label>
              <select
                value={editType}
                onChange={(e) => setEditType(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              >
                <option value="socks5">SOCKS5</option>
                <option value="socks4">SOCKS4</option>
                <option value="http">HTTP</option>
                <option value="https">HTTPS</option>
                <option value="ss">Shadowsocks (SS)</option>
              </select>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <div className="sm:col-span-2">
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                IP address or hostname
              </label>
              <input
                required
                type="text"
                value={editHost}
                onChange={(e) => setEditHost(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Port
              </label>
              <input
                required
                type="number"
                min={1}
                max={65535}
                value={editPort}
                onChange={(e) => setEditPort(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                {editType === "ss" ? "Cipher (optional)" : "Username (optional)"}
              </label>
              <input
                type="text"
                placeholder="None"
                value={editUsername}
                onChange={(e) => setEditUsername(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Password (optional)
              </label>
              <div className="relative">
                <input
                  type={showEditPassword ? "text" : "password"}
                  placeholder="None"
                  value={editPassword}
                  onChange={(e) => setEditPassword(e.target.value)}
                  className={`${INPUT_CLASS} pr-9 font-mono`}
                />
                <button
                  type="button"
                  onClick={() => setShowEditPassword((s) => !s)}
                  className="absolute top-2 right-2 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                  aria-label="Toggle password visibility"
                >
                  {showEditPassword ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </button>
              </div>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-2">
            <Button type="button" variant="secondary" onClick={() => setEditingProxy(null)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={savingEdit}>
              {savingEdit ? "Saving…" : "Save changes"}
            </Button>
          </div>
        </form>
      </Modal>

      {/* Batch Import Modal */}
      <Modal
        open={importOpen}
        onClose={() => setImportOpen(false)}
        title="Batch Import Proxies"
        subtitle={`Paste one proxy URL per line — nodes will be named sequentially starting at ${nextDefaultNodeName}`}
      >
        <form onSubmit={handleImportSubmit} className="space-y-4">
          <textarea
            required
            rows={8}
            placeholder={
              "socks4://103.17.37.196:33801\nsocks5://user:pass@203.0.113.10:1080\nhttp://user:pass@203.0.113.11:8080"
            }
            value={bulkText}
            onChange={(e) => setBulkText(e.target.value)}
            className={`${INPUT_CLASS} font-mono text-xs`}
          />
          <p className="text-[11px] text-[var(--text-muted)]">
            Every imported proxy is automatically named{" "}
            <span className="font-mono font-semibold text-[var(--text-secondary)]">
              {nextDefaultNodeName}, NODE - {(parseNodeIndex(nextDefaultNodeName) ?? 1) + 1}…
            </span>{" "}
            so pool names remain clean and consistent.
          </p>
          <div className="flex justify-end gap-2 pt-1">
            <Button type="button" variant="secondary" onClick={() => setImportOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={submitting}>
              {submitting ? "Importing…" : "Import proxies"}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  );
});
