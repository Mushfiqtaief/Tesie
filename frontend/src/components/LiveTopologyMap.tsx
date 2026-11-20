import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Blocks,
  Cpu,
  Eye,
  Maximize2,
  Minus,
  Plus,
  Terminal,
} from "lucide-react";
import type {
  ComboRecord,
  TopologyProviderNode,
  TopologyResponse,
} from "../types";
import {
  formatCompactNumber,
  formatTps,
  getProviderLogo,
} from "../utils/format";
import { ProviderIcon } from "./ProviderIcon";
import { TesieLogo } from "./TesieLogo";
import { Card, StatusPill } from "./UiPrimitives";

type HoveredTopologyItem =
  | { kind: "ingress"; x: number; y: number }
  | { kind: "core"; x: number; y: number }
  | { kind: "provider"; id: string; x: number; y: number };

const CLIENT_INGRESS_X = -195;
const CORE_HUB_X = 0;
const PROVIDER_COL_X = 195;

export const LiveTopologyMap = memo(function LiveTopologyMap({
  topology,
  onSelectProvider,
}: {
  topology: TopologyResponse | null;
  onSelectProvider: (providerId: string) => void;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasLayerRef = useRef<HTMLDivElement | null>(null);
  const [containerSize, setContainerSize] = useState({ width: 820, height: 390 });
  const [hoveredItem, setHoveredItem] = useState<HoveredTopologyItem | null>(null);

  // Interactive pan & zoom state for Flow Topology canvas
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const livePanRef = useRef({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const dragOriginRef = useRef({ x: 0, y: 0 });

  const providers: TopologyProviderNode[] = topology?.providers ?? [];
  const activeRequests = topology?.active_requests ?? [];
  const combos: ComboRecord[] = (topology?.combos ?? []).filter((c) => c.enabled);

  const activeProviderIds = useMemo(() => {
    const s = new Set<string>();
    for (const r of activeRequests) {
      s.add(r.provider_id.toLowerCase().trim());
    }
    return s;
  }, [activeRequests]);

  const lastProvider = (topology?.last_provider || "").toLowerCase().trim();
  const errorProvider = (topology?.error_provider || "").toLowerCase().trim();
  const activeCount = activeRequests.length;
  const activeCombo = combos[0] ?? null;

  // Clean 3-stage icon-only graph geometry (Client Ingress -> Tesie Core -> Providers)
  const flowLayout = useMemo(() => {
    const count = providers.length;
    const verticalGap = count <= 2 ? 124 : Math.max(92, Math.round(240 / Math.max(1, count - 1)));
    const totalSpan = Math.max(0, (count - 1) * verticalGap);

    const providerNodes = providers.map((p, idx) => {
      const y = count <= 1 ? 0 : -totalSpan / 2 + idx * verticalGap;
      const pid = p.id.toLowerCase();
      const isActive = activeProviderIds.has(pid);
      const isError = !isActive && errorProvider === pid;
      const isLast = !isActive && !isError && lastProvider === pid;
      const color = p.color || (pid === "antigravity" ? "#38BDF8" : "#E56A4A");

      const sx = CORE_HUB_X + 37;
      const sy = 0;
      const tx = PROVIDER_COL_X - 33;
      const ty = y;
      const midX = (sx + tx) / 2;
      const path = `M ${sx} ${sy} C ${midX} ${sy}, ${midX} ${ty}, ${tx} ${ty}`;

      return {
        ...p,
        x: PROVIDER_COL_X,
        y,
        sx,
        sy,
        tx,
        ty,
        color,
        isActive,
        isError,
        isLast,
        path,
      };
    });

    const ingressStartX = CLIENT_INGRESS_X + 31;
    const ingressEndX = CORE_HUB_X - 37;
    const ingressPath = `M ${ingressStartX} 0 L ${ingressEndX} 0`;

    return {
      providerNodes,
      ingressStartX,
      ingressEndX,
      ingressPath,
      logicalW: 520,
      logicalH: Math.max(240, totalSpan + 130),
    };
  }, [providers, activeProviderIds, lastProvider, errorProvider]);

  const bridgeConnection = useMemo(() => {
    const ev = topology?.bridge_event;
    if (!ev) return null;
    const transcriber = flowLayout.providerNodes.find(
      (n) => n.id.toLowerCase() === ev.transcriber_provider.toLowerCase(),
    );
    const target = flowLayout.providerNodes.find(
      (n) => n.id.toLowerCase() === ev.target_provider.toLowerCase(),
    );
    if (!transcriber || !target || transcriber.id === target.id) return null;

    const x0 = transcriber.x + 30;
    const y0 = transcriber.y;
    const x1 = target.x + 30;
    const y1 = target.y;
    const arcDist = Math.abs(y1 - y0);
    const arcOffset = Math.max(55, Math.min(110, arcDist * 0.65));
    const cpx = Math.max(x0, x1) + arcOffset;
    const path = `M ${x0} ${y0} C ${cpx} ${y0}, ${cpx} ${y1}, ${x1} ${y1}`;

    return {
      transcriber,
      target,
      path,
      badgeX: cpx + 10,
      badgeY: (y0 + y1) / 2,
      model: ev.transcriber_model,
      kind: ev.kind,
    };
  }, [topology?.bridge_event, flowLayout.providerNodes]);

  const fitView = useCallback(() => {
    const { width, height } = containerSize;
    if (!width || !height) return;
    const s = Math.min(
      (width - 48) / flowLayout.logicalW,
      (height - 48) / flowLayout.logicalH,
    );
    setZoom(Math.min(1.18, Math.max(0.55, s)));
    livePanRef.current = { x: 0, y: 0 };
    setPan({ x: 0, y: 0 });
  }, [containerSize, flowLayout.logicalH, flowLayout.logicalW]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerSize({
          width: entry.contentRect.width || 820,
          height: entry.contentRect.height || 390,
        });
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    fitView();
  }, [fitView, providers.length]);

  function onPointerDown(e: React.PointerEvent<HTMLDivElement>) {
    if ((e.target as HTMLElement).closest("button,[data-node-clickable='true']")) return;
    setHoveredItem(null);
    setDragging(true);
    const curPan = livePanRef.current;
    dragOriginRef.current = { x: e.clientX - curPan.x, y: e.clientY - curPan.y };
    e.currentTarget.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: React.PointerEvent<HTMLDivElement>) {
    if (!dragging) return;
    const nx = e.clientX - dragOriginRef.current.x;
    const ny = e.clientY - dragOriginRef.current.y;
    livePanRef.current = { x: nx, y: ny };
    if (canvasLayerRef.current) {
      canvasLayerRef.current.style.transform = `translate3d(${containerSize.width / 2 + nx}px, ${containerSize.height / 2 + ny}px, 0) scale(${zoom})`;
    }
  }

  function onPointerUp(e: React.PointerEvent<HTMLDivElement>) {
    if (dragging) {
      setDragging(false);
      setPan(livePanRef.current);
      try {
        e.currentTarget.releasePointerCapture(e.pointerId);
      } catch {
        // ignore
      }
    }
  }

  function onWheel(e: React.WheelEvent<HTMLDivElement>) {
    if (!e.ctrlKey && !e.metaKey) return;
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.09 : 0.91;
    setZoom((z) => Math.min(2.0, Math.max(0.45, z * factor)));
  }

  const healthyProxies = topology?.proxy_summary.healthy_proxies ?? 0;
  const totalProxies = topology?.proxy_summary.total_proxies ?? 0;

  const hoveredProviderNode =
    hoveredItem?.kind === "provider"
      ? flowLayout.providerNodes.find((n) => n.id === hoveredItem.id) ?? null
      : null;

  const hoverPopoverPosition = useMemo(() => {
    if (!hoveredItem) return null;
    const POPOVER_W = 284;
    const POPOVER_H = 192;
    const screenX = containerSize.width / 2 + pan.x + hoveredItem.x * zoom;
    const screenY = containerSize.height / 2 + pan.y + hoveredItem.y * zoom;
    const preferRight = screenX + POPOVER_W + 46 * zoom <= containerSize.width - 12;
    const rawLeft = preferRight
      ? screenX + 42 * zoom
      : screenX - POPOVER_W - 42 * zoom;
    const rawTop = screenY - POPOVER_H / 2;
    return {
      left: Math.max(12, Math.min(containerSize.width - POPOVER_W - 12, rawLeft)),
      top: Math.max(12, Math.min(containerSize.height - POPOVER_H - 12, rawTop)),
    };
  }, [hoveredItem, containerSize.width, containerSize.height, pan.x, pan.y, zoom]);

  return (
    <Card
      title="Live routing topology"
      subtitle="Directed live signal routing graph — hover any icon for telemetry"
      actions={
        <div className="flex flex-wrap items-center gap-2">
          {topology?.mihomo ? (
            <StatusPill tone={topology.mihomo_healthy ? "success" : "danger"} dot>
              TesieCore &middot; {healthyProxies}/{totalProxies} proxies
            </StatusPill>
          ) : (
            <StatusPill tone="danger" dot>
              Relay offline
            </StatusPill>
          )}

          {activeCount > 0 ? (
            <StatusPill tone="accent" dot pulse>
              <Activity className="h-3.5 w-3.5" />
              {activeCount} streaming
            </StatusPill>
          ) : (
            <StatusPill tone="neutral">Idle</StatusPill>
          )}
        </div>
      }
      noPadding
      className="flex min-w-0 flex-col overflow-hidden"
      bodyClassName="flex min-h-0 flex-1 flex-col"
    >
      {/* ================= FLOW TOPOLOGY CANVAS (ICON-ONLY NODES + HOVER DETAILS HUD) ================= */}
      <div
        ref={containerRef}
        role="region"
        aria-label="Live routing topology canvas"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onWheel={onWheel}
        className={`relative min-h-[360px] w-full flex-1 select-none overflow-hidden bg-[var(--bg-canvas)]/75 bg-blueprint-grid sm:min-h-[410px] ${
          dragging ? "cursor-grabbing" : "cursor-grab"
        }`}
      >
          <div
            ref={canvasLayerRef}
            className="pointer-events-none absolute inset-0"
            style={{
              transform: `translate3d(${containerSize.width / 2 + pan.x}px, ${containerSize.height / 2 + pan.y}px, 0) scale(${zoom})`,
              transformOrigin: "0 0",
            }}
          >
            {/* Clean Vector Bezier Traces & Terminal Port Dots */}
            <svg className="pointer-events-none absolute inset-0 overflow-visible">
              {/* Ingress -> Core Bridge */}
              <path
                d={flowLayout.ingressPath}
                fill="none"
                stroke={activeCount > 0 ? "#22d3ee" : "var(--border-strong)"}
                strokeWidth={activeCount > 0 ? 2.5 : 1.75}
                strokeOpacity={activeCount > 0 ? 0.9 : 0.6}
                className={activeCount > 0 ? "topology-active-edge" : undefined}
              />
              {activeCount > 0 && (
                <circle r={3.4} fill="#67e8f9" opacity={0.95}>
                  <animateMotion dur="0.9s" repeatCount="indefinite" path={flowLayout.ingressPath} />
                </circle>
              )}
              <circle
                cx={flowLayout.ingressStartX}
                cy={0}
                r={3.5}
                fill={activeCount > 0 ? "#22d3ee" : "var(--border-strong)"}
              />
              <circle
                cx={flowLayout.ingressEndX}
                cy={0}
                r={3.5}
                fill={activeCount > 0 ? "#22d3ee" : "var(--border-strong)"}
              />

              {/* Core -> Provider Upstream Branches */}
              {flowLayout.providerNodes.map((node) => {
                const idleStroke = node.isError
                  ? "var(--danger)"
                  : node.isLast
                    ? "var(--warning)"
                    : "var(--border-strong)";

                return (
                  <g key={`edge-${node.id}`}>
                    {node.isActive ? (
                      <>
                        {/* Soft vector glow halo (composited stroke, no turbulence filter) */}
                        <path
                          d={node.path}
                          fill="none"
                          stroke="#22d3ee"
                          strokeWidth={7}
                          strokeOpacity={0.2}
                          strokeLinecap="round"
                        />
                        <path
                          d={node.path}
                          fill="none"
                          stroke={node.color}
                          strokeWidth={2.5}
                          strokeLinecap="round"
                          className="topology-active-edge"
                        />
                        {[0, 0.48].map((delay, i) => (
                          <circle
                            key={`${node.id}-sig-${i}`}
                            r={3.4}
                            fill="#67e8f9"
                            opacity={0.95}
                          >
                            <animateMotion
                              dur="1.05s"
                              begin={`${delay}s`}
                              repeatCount="indefinite"
                              path={node.path}
                            />
                          </circle>
                        ))}
                      </>
                    ) : (
                      <path
                        d={node.path}
                        fill="none"
                        stroke={idleStroke}
                        strokeWidth={node.isLast || node.isError ? 2.2 : 1.65}
                        strokeOpacity={node.isLast || node.isError ? 0.9 : 0.5}
                        strokeLinecap="round"
                      />
                    )}

                    <circle
                      cx={node.sx}
                      cy={node.sy}
                      r={3.5}
                      fill={node.isActive ? "#22d3ee" : idleStroke}
                    />
                    <circle
                      cx={node.tx}
                      cy={node.ty}
                      r={3.5}
                      fill={
                        node.isActive
                          ? node.color
                          : node.isError
                            ? "var(--danger)"
                            : node.isLast
                              ? "var(--warning)"
                              : "var(--border-strong)"
                      }
                    />
                  </g>
                );
              })}
              {/* Multimodal Bridge Inter-Provider Arc */}
              {bridgeConnection && (
                <g key="multimodal-bridge-connection">
                  <defs>
                    <linearGradient id="multimodalBridgeGrad" x1="0%" y1="0%" x2="100%" y2="100%">
                      <stop offset="0%" stopColor="#38bdf8" />
                      <stop offset="50%" stopColor="#c084fc" />
                      <stop offset="100%" stopColor="#38bdf8" />
                    </linearGradient>
                  </defs>
                  {/* Soft Vector Glow */}
                  <path
                    d={bridgeConnection.path}
                    fill="none"
                    stroke="#38bdf8"
                    strokeWidth={8}
                    strokeOpacity={0.2}
                    strokeLinecap="round"
                  />
                  {/* Animated Dashed Bridge Curve */}
                  <path
                    d={bridgeConnection.path}
                    fill="none"
                    stroke="url(#multimodalBridgeGrad)"
                    strokeWidth={2.75}
                    strokeDasharray="6 4"
                    strokeLinecap="round"
                    className="topology-active-edge"
                  />
                  {/* Flowing Transcribing Particle */}
                  <circle r={4.5} fill="#38bdf8" opacity={0.95}>
                    <animateMotion
                      dur="1.35s"
                      repeatCount="indefinite"
                      path={bridgeConnection.path}
                    />
                  </circle>
                  <circle
                    cx={bridgeConnection.transcriber.x + 30}
                    cy={bridgeConnection.transcriber.y}
                    r={4}
                    fill="#38bdf8"
                  />
                  <circle
                    cx={bridgeConnection.target.x + 30}
                    cy={bridgeConnection.target.y}
                    r={4}
                    fill="#c084fc"
                  />
                </g>
              )}
            </svg>

            {/* Live Multimodal Bridge Floating Indicator */}
            {bridgeConnection && (
              <div
                style={{
                  left: bridgeConnection.badgeX,
                  top: bridgeConnection.badgeY,
                  transform: "translate(0, -50%)",
                }}
                className="pointer-events-none absolute z-20 flex items-center gap-1.5 rounded-full border border-sky-400/40 bg-[var(--bg-canvas)]/95 px-3 py-1 font-mono text-[11px] font-semibold text-sky-300 shadow-lg animate-pulse"
              >
                <Eye className="h-3.5 w-3.5 text-sky-400" />
                <span>
                  Bridge: {bridgeConnection.transcriber.name} → {bridgeConnection.target.name}
                </span>
              </div>
            )}

            {/* Stage 1 (Left): Client Ingress — Pure Icon Tile */}
            <div
              data-node-clickable="true"
              tabIndex={0}
              role="button"
              aria-label="Client Ingress details"
              onMouseEnter={() => setHoveredItem({ kind: "ingress", x: CLIENT_INGRESS_X, y: 0 })}
              onMouseLeave={() =>
                setHoveredItem((cur) => (cur?.kind === "ingress" ? null : cur))
              }
              onFocus={() => setHoveredItem({ kind: "ingress", x: CLIENT_INGRESS_X, y: 0 })}
              onBlur={() =>
                setHoveredItem((cur) => (cur?.kind === "ingress" ? null : cur))
              }
              style={{ left: CLIENT_INGRESS_X, top: 0 }}
              className={`topology-node-interactive pointer-events-auto absolute z-10 flex h-[58px] w-[58px] cursor-pointer items-center justify-center rounded-2xl border-2 bg-[var(--bg-surface)] shadow-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] ${
                hoveredItem?.kind === "ingress" || activeCount > 0
                  ? "border-[var(--accent)] text-[var(--accent)]"
                  : "border-[var(--border-strong)] text-[var(--text-primary)] hover:border-[var(--accent)]"
              }`}
            >
              <Terminal className="h-6 w-6" />
              <span
                className={`absolute top-1.5 right-1.5 h-2 w-2 rounded-full ${
                  activeCount > 0 ? "bg-cyan-400 animate-pulse" : "bg-[var(--accent)]"
                }`}
              />
            </div>

            {/* Stage 2 (Center): Tesie Router Core — Pure Icon Emblem */}
            <div
              data-node-clickable="true"
              tabIndex={0}
              role="button"
              aria-label="Tesie Router Core details"
              onMouseEnter={() => setHoveredItem({ kind: "core", x: CORE_HUB_X, y: 0 })}
              onMouseLeave={() =>
                setHoveredItem((cur) => (cur?.kind === "core" ? null : cur))
              }
              onFocus={() => setHoveredItem({ kind: "core", x: CORE_HUB_X, y: 0 })}
              onBlur={() =>
                setHoveredItem((cur) => (cur?.kind === "core" ? null : cur))
              }
              style={{ left: CORE_HUB_X, top: 0 }}
              className={`topology-node-interactive pointer-events-auto absolute z-10 flex h-[70px] w-[70px] cursor-pointer items-center justify-center rounded-2xl border-2 bg-[var(--bg-surface)] shadow-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] ${
                activeCount > 0
                  ? "border-cyan-400 text-cyan-400"
                  : hoveredItem?.kind === "core"
                    ? "border-[var(--accent)] text-[var(--accent)]"
                    : "border-[var(--accent)]/85 text-[var(--accent)]"
              }`}
            >
              <TesieLogo size={40} />
              <span
                className={`absolute top-2 right-2 h-2.5 w-2.5 rounded-full ${
                  activeCount > 0
                    ? "bg-cyan-400 animate-pulse"
                    : healthyProxies > 0 || !topology?.mihomo
                      ? "bg-[var(--success)]"
                      : "bg-[var(--danger)]"
                }`}
              />
            </div>

            {/* Stage 3 (Right): Upstream Provider Nodes — Pure Provider Icons Only */}
            {flowLayout.providerNodes.map((node) => {
              const isHovered =
                hoveredItem?.kind === "provider" && hoveredItem.id === node.id;
              return (
                <div
                  key={node.id}
                  data-node-clickable="true"
                  role="button"
                  tabIndex={0}
                  aria-label={`${node.name} routing details`}
                  onClick={() => onSelectProvider(node.id)}
                  onMouseEnter={() =>
                    setHoveredItem({ kind: "provider", id: node.id, x: node.x, y: node.y })
                  }
                  onMouseLeave={() =>
                    setHoveredItem((cur) =>
                      cur?.kind === "provider" && cur.id === node.id ? null : cur,
                    )
                  }
                  onFocus={() =>
                    setHoveredItem({ kind: "provider", id: node.id, x: node.x, y: node.y })
                  }
                  onBlur={() =>
                    setHoveredItem((cur) =>
                      cur?.kind === "provider" && cur.id === node.id ? null : cur,
                    )
                  }
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onSelectProvider(node.id);
                    }
                  }}
                  style={{ left: node.x, top: node.y }}
                  className={`topology-node-interactive pointer-events-auto absolute z-10 flex h-[62px] w-[62px] cursor-pointer items-center justify-center rounded-2xl border-2 bg-[var(--bg-surface)] shadow-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)] ${
                    node.isActive
                      ? "border-cyan-400"
                      : node.isError
                        ? "border-[var(--danger)]"
                        : node.isLast
                          ? "border-[var(--warning)]"
                          : isHovered
                            ? "border-[var(--accent)]"
                            : "border-[var(--border-strong)] hover:border-[var(--accent)]"
                  }`}
                >
                  <ProviderIcon
                    src={
                      node.logo ||
                      (node.id.startsWith("cp-") ? undefined : getProviderLogo(node.id))
                    }
                    name={node.name}
                    color={node.color}
                    size={34}
                  />
                  <span
                    className={`absolute top-1.5 right-1.5 h-2.5 w-2.5 rounded-full ${
                      node.isActive
                        ? "bg-cyan-400 animate-pulse"
                        : node.isError
                          ? "bg-[var(--danger)]"
                          : node.isLast
                            ? "bg-[var(--warning)]"
                            : node.active_units > 0
                              ? "bg-[var(--success)]"
                              : "bg-[var(--text-muted)]"
                    }`}
                  />
                </div>
              );
            })}
          </div>

          {/* Floating Non-Clipping Hover Details Popover */}
          {hoveredItem && hoverPopoverPosition && (
            <div
              role="tooltip"
              style={{
                left: hoverPopoverPosition.left,
                top: hoverPopoverPosition.top,
              }}
              className="pointer-events-none absolute z-30 w-[284px] rounded-2xl border border-[var(--border-strong)] bg-[var(--bg-surface)]/98 p-3.5 shadow-xl"
            >
              {hoveredItem.kind === "ingress" && (
                <div className="space-y-2.5">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <Terminal className="h-4 w-4 text-[var(--accent)]" />
                      <span className="text-[13px] font-semibold text-[var(--text-primary)]">
                        Client Ingress
                      </span>
                    </div>
                    <StatusPill tone={activeCount > 0 ? "accent" : "neutral"} dot>
                      {activeCount > 0 ? `${activeCount} live` : "Standby"}
                    </StatusPill>
                  </div>
                  <div className="text-xs text-[var(--text-muted)]">
                    OpenCode &middot; Claude Code &middot; Codex
                  </div>
                  <div className="flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--text-secondary)]">
                    <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5">
                      /v1/chat/completions
                    </span>
                    <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5">
                      /v1/messages
                    </span>
                    <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5">
                      /v1/responses
                    </span>
                  </div>
                  {activeCombo && (
                    <div className="border-t border-[var(--border-subtle)] pt-2 font-mono text-[11px] text-[var(--text-muted)]">
                      Default combo:{" "}
                      <span className="font-semibold text-[var(--text-primary)]">
                        {activeCombo.name}
                      </span>
                    </div>
                  )}
                </div>
              )}

              {hoveredItem.kind === "core" && (
                <div className="space-y-2.5">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <Cpu className="h-4 w-4 text-[var(--accent)]" />
                      <span className="text-[13px] font-semibold text-[var(--text-primary)]">
                        Tesie Router Core
                      </span>
                    </div>
                    <StatusPill
                      tone={topology?.mihomo_healthy ? "success" : "danger"}
                      dot
                    >
                      {topology?.mihomo_healthy ? "Online" : "Offline"}
                    </StatusPill>
                  </div>
                  <div className="grid grid-cols-2 gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/65 px-3 py-2 text-xs tabular-nums">
                    <div>
                      <div className="text-[11px] text-[var(--text-muted)]">Proxy pool</div>
                      <div className="font-mono font-semibold text-[var(--success)]">
                        {healthyProxies}/{totalProxies} ready
                      </div>
                    </div>
                    <div>
                      <div className="text-[11px] text-[var(--text-muted)]">In-flight</div>
                      <div className="font-mono font-semibold text-[var(--text-primary)]">
                        {activeCount} streams
                      </div>
                    </div>
                  </div>
                  <div className="font-mono text-[11px] text-[var(--text-muted)]">
                    {activeCombo
                      ? `Combo: ${activeCombo.name} (${activeCombo.strategy})`
                      : "Direct provider prefix dispatch"}
                  </div>
                </div>
              )}

              {hoveredProviderNode && (() => {
                const node = hoveredProviderNode;
                const activeReqsForNode = activeRequests.filter(
                  (r) => r.provider_id.toLowerCase() === node.id.toLowerCase(),
                );
                const liveProxy = activeReqsForNode[0]?.proxy_name;
                const isIpPool = node.quota_strategy === "ip_pool";
                const totalUnits = Math.max(1, node.active_units + node.cooling_units);
                const readyPct = Math.max(
                  0,
                  Math.min(100, Math.round((node.active_units / totalUnits) * 100)),
                );

                return (
                  <div className="space-y-2.5">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-2">
                        <ProviderIcon
                          src={node.logo || getProviderLogo(node.id)}
                          name={node.name}
                          color={node.color}
                          size={22}
                        />
                        <div className="min-w-0">
                          <div className="truncate text-[13px] font-semibold text-[var(--text-primary)]">
                            {node.name}
                          </div>
                          <div className="font-mono text-[11px] text-[var(--text-muted)]">
                            {isIpPool ? "Round Robin · Per-IP" : "Smart Fallback · Per-Acct"}
                          </div>
                        </div>
                      </div>
                      {node.isActive ? (
                        <StatusPill tone="accent" dot pulse>
                          Live
                        </StatusPill>
                      ) : node.isError ? (
                        <StatusPill tone="danger" dot>
                          Error
                        </StatusPill>
                      ) : node.isLast ? (
                        <StatusPill tone="warning" dot>
                          Last route
                        </StatusPill>
                      ) : (
                        <StatusPill tone="neutral">Standby</StatusPill>
                      )}
                    </div>

                    <div className="space-y-1">
                      <div className="flex items-center justify-between font-mono text-xs tabular-nums">
                        <span className="text-[var(--success)]">
                          {node.active_units} {isIpPool ? "ready IPs" : "ready acct"}
                        </span>
                        <span
                          className={
                            node.cooling_units > 0
                              ? "text-[var(--warning)]"
                              : "text-[var(--text-muted)]"
                          }
                        >
                          {node.cooling_units} cooling
                        </span>
                      </div>
                      <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-elevated)]">
                        <div
                          className="h-full rounded-full transition-[width] duration-300"
                          style={{
                            width: `${node.active_units > 0 ? Math.max(8, readyPct) : 0}%`,
                            backgroundColor: node.isActive ? "#22d3ee" : "var(--success)",
                          }}
                        />
                      </div>
                    </div>

                    <div className="flex items-center justify-between border-t border-[var(--border-subtle)] pt-2 font-mono text-[11px] text-[var(--text-muted)] tabular-nums">
                      <span>
                        {node.total_requests} req &middot;{" "}
                        {formatCompactNumber(node.total_tokens)} tok
                      </span>
                      {liveProxy ? (
                        <span className="font-semibold text-cyan-400">{liveProxy}</span>
                      ) : node.avg_tokens_per_sec > 0 ? (
                        <span className="font-semibold text-[var(--speed)]">
                          {formatTps(node.avg_tokens_per_sec)}
                        </span>
                      ) : (
                        <span>Click to configure</span>
                      )}
                    </div>
                  </div>
                );
              })()}
            </div>
          )}

          {/* Bottom-left Zoom Controls + Readout */}
          <div className="absolute bottom-3 left-3 z-20 flex items-center gap-2">
            <div className="flex overflow-hidden rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]/95 shadow-sm">
              <button
                type="button"
                onClick={() => setZoom((z) => Math.min(2.0, z * 1.16))}
                className="flex items-center justify-center border-r border-[var(--border-subtle)] p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
                title="Zoom in"
                aria-label="Zoom in"
              >
                <Plus className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={() => setZoom((z) => Math.max(0.45, z / 1.16))}
                className="flex items-center justify-center border-r border-[var(--border-subtle)] p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
                title="Zoom out"
                aria-label="Zoom out"
              >
                <Minus className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={fitView}
                className="flex items-center justify-center p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
                title="Fit topology to view"
                aria-label="Fit view"
              >
                <Maximize2 className="h-3.5 w-3.5" />
              </button>
            </div>
            <span className="hidden rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)]/90 px-2.5 py-1 font-mono text-[11px] text-[var(--text-muted)] tabular-nums sm:inline">
              {Math.round(zoom * 100)}%
            </span>
          </div>

          {/* Bottom-right Subtle Interaction Hint + Legend */}
          <div className="pointer-events-none absolute right-3 bottom-3 z-20 hidden items-center gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]/95 px-3 py-1.5 text-[11px] text-[var(--text-muted)] sm:flex">
            <span>Hover icon for details</span>
            <span className="text-[var(--border-strong)]">&middot;</span>
            <span className="inline-flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-full bg-cyan-400" />
              Streaming
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-full bg-[var(--warning)]" />
              Last route
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-full bg-[var(--danger)]" />
              Error
            </span>
          </div>
      </div>

      {/* Active / Primary Combo Cascade Strip */}
      {activeCombo && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 px-4 py-2.5 text-xs">
          <div className="flex flex-wrap items-center gap-2">
            <span className="inline-flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
              <Blocks className="h-3.5 w-3.5 text-[var(--accent)]" />
              Combo route: <span className="font-mono">{activeCombo.name}</span>
            </span>
            <span className="rounded bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
              {activeCombo.strategy}
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-1.5 font-mono text-[11px]">
            {activeCombo.models.map((m, idx) => (
              <span key={`${m}-${idx}`} className="inline-flex items-center gap-1.5">
                {idx > 0 && <span className="text-[var(--text-muted)]">&rarr;</span>}
                <span className="rounded-md border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2 py-0.5 text-[var(--text-secondary)]">
                  <span className="mr-1 text-[var(--text-muted)]">{idx + 1}.</span>
                  {m}
                </span>
              </span>
            ))}
          </div>
        </div>
      )}
    </Card>
  );
});
