import { memo, useDeferredValue, useMemo, useState } from "react";
import {
  CheckSquare,
  Globe,
  LayoutGrid,
  List,
  Network,
  Search,
  Square,
} from "lucide-react";
import type { ProxyWithQuota } from "../types";
import { Button, INPUT_CLASS } from "./UiPrimitives";

export interface ProxySelectionValue {
  proxyMode: "all" | "selected" | "direct";
  selectedProxyIds: string[];
}

export const ProviderProxySelector = memo(function ProviderProxySelector({
  proxies,
  value,
  onChange,
  strategyHint,
  onOpenProxyPool,
}: {
  proxies: ProxyWithQuota[];
  value: ProxySelectionValue;
  onChange: (next: ProxySelectionValue) => void;
  strategyHint: string;
  onOpenProxyPool: () => void;
}) {
  const [filterText, setFilterText] = useState("");
  const deferredFilterText = useDeferredValue(filterText);
  const [layoutMode, setLayoutMode] = useState<"table" | "list">("table");
  const [visibleLimit, setVisibleLimit] = useState(60);

  const externalProxies = useMemo(
    () => proxies.filter((p) => !p.is_direct),
    [proxies],
  );
  const allIds = useMemo(() => externalProxies.map((p) => p.id), [externalProxies]);
  const allIdSet = useMemo(() => new Set<string>(allIds), [allIds]);

  const selectedSet = useMemo(() => {
    if (value.proxyMode === "direct") return new Set<string>();
    if (value.proxyMode === "all") return new Set<string>(allIds);
    return new Set<string>(value.selectedProxyIds.filter((id) => allIdSet.has(id)));
  }, [value.proxyMode, value.selectedProxyIds, allIds, allIdSet]);

  const filteredProxies = useMemo(() => {
    const q = deferredFilterText.trim().toLowerCase();
    if (!q) return externalProxies;
    return externalProxies.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        (p.ip || "").toLowerCase().includes(q) ||
        (p.host || "").toLowerCase().includes(q) ||
        p.proxy_type.toLowerCase().includes(q),
    );
  }, [externalProxies, deferredFilterText]);

  const visibleProxies = useMemo(
    () => filteredProxies.slice(0, visibleLimit),
    [filteredProxies, visibleLimit],
  );

  function applySelectedSet(nextSet: Set<string>, preferAllToken = false) {
    if (nextSet.size === 0) {
      onChange({ proxyMode: "direct", selectedProxyIds: [] });
      return;
    }
    if (preferAllToken && nextSet.size === externalProxies.length) {
      onChange({ proxyMode: "all", selectedProxyIds: [] });
      return;
    }
    if (nextSet.size === externalProxies.length && externalProxies.length > 0) {
      onChange({ proxyMode: "all", selectedProxyIds: [] });
      return;
    }
    const ordered = allIds.filter((id) => nextSet.has(id));
    onChange({ proxyMode: "selected", selectedProxyIds: ordered });
  }

  function handleSelectAllPool() {
    if (externalProxies.length === 0) {
      onChange({ proxyMode: "direct", selectedProxyIds: [] });
      return;
    }
    onChange({ proxyMode: "all", selectedProxyIds: [] });
  }

  function handleClearToDirect() {
    onChange({ proxyMode: "direct", selectedProxyIds: [] });
  }

  function handleSelectAllFiltered() {
    const next = new Set(selectedSet);
    for (const p of filteredProxies) {
      next.add(p.id);
    }
    applySelectedSet(next);
  }

  function handleDeselectFiltered() {
    const next = new Set(selectedSet);
    for (const p of filteredProxies) {
      next.delete(p.id);
    }
    applySelectedSet(next);
  }

  function toggleOne(id: string) {
    const next = new Set(selectedSet);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    applySelectedSet(next);
  }

  const selectedCount = selectedSet.size;
  const isDirectFallback = selectedCount === 0 || externalProxies.length === 0;
  const isAllSelected =
    externalProxies.length > 0 &&
    (value.proxyMode === "all" || selectedCount === externalProxies.length);
  const filteredSelectedCount = useMemo(
    () => filteredProxies.reduce((acc, p) => acc + (selectedSet.has(p.id) ? 1 : 0), 0),
    [filteredProxies, selectedSet],
  );
  const healthySelected = useMemo(
    () => externalProxies.reduce((acc, p) => acc + (selectedSet.has(p.id) && p.alive ? 1 : 0), 0),
    [externalProxies, selectedSet],
  );

  return (
    <div className="space-y-3">
      {/* Selection Status & Controls Row */}
      <div className="flex flex-wrap items-center justify-between gap-2.5 text-[13px]">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          {isDirectFallback ? (
            <span className="inline-flex items-center gap-1.5 font-medium text-[var(--warning)]">
              <Globe className="h-4 w-4" />
              Direct (No Proxy)
            </span>
          ) : (
            <span className="inline-flex items-center gap-1.5 font-medium text-[var(--text-primary)] tabular-nums">
              <Network className="h-4 w-4 text-[var(--success)]" />
              {isAllSelected
                ? `All ${externalProxies.length} proxies`
                : `${selectedCount} of ${externalProxies.length} selected`}
              <span className="font-mono text-xs text-[var(--success)]">
                ({healthySelected} healthy)
              </span>
            </span>
          )}
          <span className="hidden text-xs text-[var(--text-muted)] md:inline">
            &middot; {strategyHint}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          <div className="inline-flex rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5">
            <button
              type="button"
              disabled={externalProxies.length === 0}
              onClick={handleSelectAllPool}
              className={`rounded-md px-3 py-1 text-xs font-medium transition-colors disabled:opacity-45 ${
                isAllSelected
                  ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-2xs"
                  : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
              }`}
            >
              All ({externalProxies.length})
            </button>
            <button
              type="button"
              onClick={handleClearToDirect}
              title="Deselect all proxies to route directly without proxying"
              className={`rounded-md px-3 py-1 text-xs font-medium transition-colors ${
                isDirectFallback
                  ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-2xs"
                  : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
              }`}
            >
              None (Direct)
            </button>
          </div>
        </div>
      </div>

      {externalProxies.length === 0 ? (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-dashed border-[var(--border-strong)] bg-[var(--bg-elevated)]/40 px-4 py-3.5 text-[13px] text-[var(--text-muted)]">
          <span>
            No external proxies in the global pool yet — requests will use{" "}
            <strong className="text-[var(--text-primary)]">Direct Connection</strong>.
          </span>
          <button
            type="button"
            onClick={onOpenProxyPool}
            className="font-medium text-[var(--accent)] hover:underline"
          >
            Add proxies in Proxy Pool
          </button>
        </div>
      ) : (
        <div className="overflow-hidden rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/25">
          {/* Search & Filter Batch Controls Header */}
          <div className="flex flex-wrap items-center justify-between gap-2.5 border-b border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3.5 py-2.5">
            <div className="relative min-w-[210px] flex-1 sm:max-w-xs">
              <Search className="pointer-events-none absolute top-2.5 left-3 h-3.5 w-3.5 text-[var(--text-muted)]" />
              <input
                type="text"
                placeholder="Filter NODE, IP, or type…"
                value={filterText}
                onChange={(e) => {
                  setFilterText(e.target.value);
                  setVisibleLimit(60);
                }}
                className={`${INPUT_CLASS} py-1.5 pl-8.5 text-[13px]`}
              />
            </div>

            <div className="flex flex-wrap items-center gap-2 text-xs">
              <span className="font-mono text-xs text-[var(--text-muted)] tabular-nums">
                {filteredSelectedCount}/{filteredProxies.length}
              </span>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={filteredProxies.length === 0}
                onClick={handleSelectAllFiltered}
              >
                <CheckSquare className="h-3.5 w-3.5 text-[var(--accent)]" />
                Select filtered
              </Button>
              {filteredSelectedCount > 0 && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={handleDeselectFiltered}
                >
                  <Square className="h-3.5 w-3.5" />
                  Unselect
                </Button>
              )}

              <div className="inline-flex rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5">
                <button
                  type="button"
                  onClick={() => setLayoutMode("table")}
                  title="Table grid view (4 per line)"
                  className={`inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    layoutMode === "table"
                      ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-2xs"
                      : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  <LayoutGrid className="h-3.5 w-3.5" />
                  Table
                </button>
                <button
                  type="button"
                  onClick={() => setLayoutMode("list")}
                  title="List view (2 per line)"
                  className={`inline-flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                    layoutMode === "list"
                      ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-2xs"
                      : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                  }`}
                >
                  <List className="h-3.5 w-3.5" />
                  List
                </button>
              </div>
            </div>
          </div>

          {/* Scrollable Separate Proxy Card Grid (4/row Table | 2/row List) */}
          <div className="max-h-72 overflow-y-auto p-3">
            {filteredProxies.length === 0 ? (
              <div className="rounded-xl border border-dashed border-[var(--border-subtle)] bg-[var(--bg-surface)] py-7 text-center text-[13px] text-[var(--text-muted)]">
                No proxies match &ldquo;{filterText}&rdquo;
              </div>
            ) : (
              <>
                <div
                  className={
                    layoutMode === "list"
                      ? "grid grid-cols-1 gap-2.5 md:grid-cols-2"
                      : "grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-4"
                  }
                >
                  {visibleProxies.map((px) => {
                    const checked = selectedSet.has(px.id);
                    return (
                      <label
                        key={px.id}
                        className={`cv-card flex cursor-pointer items-center justify-between gap-2.5 rounded-xl border px-3.5 py-2.5 text-[13px] transition-[border-color,background-color] duration-150 select-none ${
                          checked
                            ? "border-[color-mix(in_srgb,var(--accent)_45%,var(--border-subtle))] bg-[color-mix(in_srgb,var(--accent-soft)_38%,var(--bg-surface))] shadow-2xs"
                            : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-hover)]/45"
                        }`}
                      >
                        <div className="flex min-w-0 items-center gap-2.5">
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() => toggleOne(px.id)}
                            className="h-4 w-4 shrink-0 rounded accent-[var(--accent)]"
                          />
                          <div className="min-w-0">
                            <div className="flex items-center gap-1.5">
                              <span
                                className={`h-2 w-2 shrink-0 rounded-full ${
                                  px.alive ? "bg-[var(--success)]" : "bg-[var(--danger)]"
                                }`}
                              />
                              <span className="truncate font-mono text-[13px] font-medium text-[var(--text-primary)]">
                                {px.name}
                              </span>
                            </div>
                            <div className="truncate font-mono text-[11px] text-[var(--text-muted)]">
                              {px.ip || px.host || "unprobed"} · {px.proxy_type.toUpperCase()}
                            </div>
                          </div>
                        </div>

                        <span
                          className={`shrink-0 font-mono text-[11px] tabular-nums ${
                            !px.alive
                              ? "text-[var(--danger)]"
                              : px.latency_ms
                                ? "text-[var(--success)]"
                                : "text-[var(--text-muted)]"
                          }`}
                        >
                          {!px.alive ? "timeout" : px.latency_ms ? `${px.latency_ms}ms` : "—"}
                        </span>
                      </label>
                    );
                  })}
                </div>
                {filteredProxies.length > visibleProxies.length && (
                  <div className="mt-2.5 flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 text-xs text-[var(--text-muted)]">
                    <span className="tabular-nums">
                      Showing {visibleProxies.length} of {filteredProxies.length} proxies
                    </span>
                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() => setVisibleLimit((v) => v + 60)}
                      >
                        Show +60 more
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() => setVisibleLimit(filteredProxies.length)}
                      >
                        Show all ({filteredProxies.length})
                      </Button>
                    </div>
                  </div>
                )}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
});
