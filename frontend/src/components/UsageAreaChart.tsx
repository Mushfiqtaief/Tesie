import { useId, useMemo, useState } from "react";
import type { UsageTimeBucket } from "../types";
import { formatCompactNumber, formatTps } from "../utils/format";
import { Card, SegmentedControl } from "./UiPrimitives";

type ChartMode = "tokens" | "requests" | "speed";

interface Point2D {
  x: number;
  y: number;
}

/**
 * Computes a clean human-readable ceiling above `rawMax` so lines have ~8% breathing
 * room above peak values and all horizontal grid ticks land on legible intervals.
 */
function computeNiceMax(rawMax: number, allowSubFour = false): number {
  if (!Number.isFinite(rawMax) || rawMax <= 0) return allowSubFour ? 4 : 4;
  const padded = rawMax * 1.08;
  const mag = Math.pow(10, Math.floor(Math.log10(padded)));
  const norm = padded / mag;
  let niceNorm = 10;
  if (norm <= 1.0) niceNorm = 1.0;
  else if (norm <= 1.2) niceNorm = 1.2;
  else if (norm <= 1.5) niceNorm = 1.5;
  else if (norm <= 2.0) niceNorm = 2.0;
  else if (norm <= 2.5) niceNorm = 2.5;
  else if (norm <= 3.0) niceNorm = 3.0;
  else if (norm <= 4.0) niceNorm = 4.0;
  else if (norm <= 5.0) niceNorm = 5.0;
  else if (norm <= 6.0) niceNorm = 6.0;
  else if (norm <= 8.0) niceNorm = 8.0;
  const res = niceNorm * mag;
  return allowSubFour ? res : Math.max(4, Math.round(res));
}

/**
 * Fritsch–Carlson monotone cubic Hermite interpolation so spikes transition smoothly
 * without ever overshooting below baseline (`baselineY`) or above local peaks.
 */
function buildMonotoneLinePath(points: Point2D[], topY: number, baselineY: number): string {
  const n = points.length;
  if (n === 0) return "";
  if (n === 1) {
    return `M ${points[0].x.toFixed(1)} ${points[0].y.toFixed(1)}`;
  }

  const dx: number[] = new Array(n - 1);
  const dy: number[] = new Array(n - 1);
  const slopes: number[] = new Array(n - 1);

  for (let i = 0; i < n - 1; i++) {
    dx[i] = Math.max(0.001, points[i + 1].x - points[i].x);
    dy[i] = points[i + 1].y - points[i].y;
    slopes[i] = dy[i] / dx[i];
  }

  const tangents: number[] = new Array(n).fill(0);
  tangents[0] = slopes[0];
  tangents[n - 1] = slopes[n - 2];

  for (let i = 1; i < n - 1; i++) {
    if (slopes[i - 1] * slopes[i] <= 0) {
      tangents[i] = 0;
    } else {
      const m0 = slopes[i - 1];
      const m1 = slopes[i];
      const w1 = 2 * dx[i] + dx[i - 1];
      const w2 = dx[i] + 2 * dx[i - 1];
      tangents[i] = (w1 + w2) / (w1 / m0 + w2 / m1);
    }
  }

  for (let i = 0; i < n - 1; i++) {
    if (Math.abs(slopes[i]) < 1e-6) {
      tangents[i] = 0;
      tangents[i + 1] = 0;
    } else {
      const a = tangents[i] / slopes[i];
      const b = tangents[i + 1] / slopes[i];
      const sq = a * a + b * b;
      if (sq > 9) {
        const tau = 3 / Math.sqrt(sq);
        tangents[i] = tau * a * slopes[i];
        tangents[i + 1] = tau * b * slopes[i];
      }
    }
  }

  const clampY = (y: number) => Math.max(topY, Math.min(baselineY, y));
  const cmds: string[] = [`M ${points[0].x.toFixed(1)} ${clampY(points[0].y).toFixed(1)}`];

  for (let i = 0; i < n - 1; i++) {
    const p0 = points[i];
    const p1 = points[i + 1];
    const h = dx[i] / 3;
    const c1x = p0.x + h;
    const c1y = clampY(p0.y + tangents[i] * h);
    const c2x = p1.x - h;
    const c2y = clampY(p1.y - tangents[i + 1] * h);
    cmds.push(
      `C ${c1x.toFixed(1)} ${c1y.toFixed(1)}, ${c2x.toFixed(1)} ${c2y.toFixed(1)}, ${p1.x.toFixed(1)} ${clampY(p1.y).toFixed(1)}`,
    );
  }
  return cmds.join(" ");
}

function buildMonotoneAreaPath(points: Point2D[], topY: number, baselineY: number): string {
  if (points.length === 0) return "";
  const line = buildMonotoneLinePath(points, topY, baselineY);
  const lastX = points[points.length - 1].x.toFixed(1);
  const firstX = points[0].x.toFixed(1);
  return `${line} L ${lastX} ${baselineY.toFixed(1)} L ${firstX} ${baselineY.toFixed(1)} Z`;
}

export function UsageAreaChart({ buckets }: { buckets: UsageTimeBucket[] }) {
  const uid = useId().replace(/:/g, "");
  const [mode, setMode] = useState<ChartMode>("tokens");
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);
  const [showInput, setShowInput] = useState(true);
  const [showCached, setShowCached] = useState(true);
  const [showOutput, setShowOutput] = useState(true);

  const w = 920;
  const h = 232;
  const padL = 56;
  const padR = 16;
  const padT = 14;
  const padB = 30;
  const plotW = w - padL - padR;
  const plotH = h - padT - padB;
  const baselineY = padT + plotH;

  const data = buckets;
  const n = data.length;

  const totals = useMemo(() => {
    let prompt = 0;
    let cached = 0;
    let completion = 0;
    let requests = 0;
    let rateLimited = 0;
    let speedSum = 0;
    let speedCount = 0;
    let peakSpeed = 0;

    for (const d of data) {
      prompt += d.prompt_tokens;
      cached += d.cached_tokens;
      completion += d.completion_tokens;
      requests += d.requests;
      rateLimited += d.rate_limited_requests;
      if (d.avg_tokens_per_sec > 0) {
        speedSum += d.avg_tokens_per_sec;
        speedCount += 1;
        if (d.avg_tokens_per_sec > peakSpeed) {
          peakSpeed = d.avg_tokens_per_sec;
        }
      }
    }
    return {
      prompt,
      cached,
      completion,
      requests,
      rateLimited,
      avgSpeed: speedCount > 0 ? speedSum / speedCount : 0,
      peakSpeed,
    };
  }, [data]);

  function toggleSeries(key: "in" | "cache" | "out") {
    const activeCount = (showInput ? 1 : 0) + (showCached ? 1 : 0) + (showOutput ? 1 : 0);
    if (key === "in") {
      if (showInput && activeCount <= 1) return;
      setShowInput((v) => !v);
    } else if (key === "cache") {
      if (showCached && activeCount <= 1) return;
      setShowCached((v) => !v);
    } else {
      if (showOutput && activeCount <= 1) return;
      setShowOutput((v) => !v);
    }
  }

  // Strictly compute ceiling across ALL visible series so no trace ever exceeds the top axis
  const rawMax = useMemo(() => {
    let m = 0;
    for (const d of data) {
      if (mode === "tokens") {
        if (showInput) m = Math.max(m, d.prompt_tokens);
        if (showCached) m = Math.max(m, d.cached_tokens);
        if (showOutput) m = Math.max(m, d.completion_tokens);
      } else if (mode === "requests") {
        m = Math.max(m, d.requests, d.rate_limited_requests);
      } else {
        m = Math.max(m, d.avg_tokens_per_sec);
      }
    }
    return m;
  }, [data, mode, showInput, showCached, showOutput]);

  const maxVal = useMemo(() => computeNiceMax(rawMax, mode === "speed"), [rawMax, mode]);

  function xFor(i: number): number {
    if (n <= 1) return padL + plotW / 2;
    return padL + (i / (n - 1)) * plotW;
  }

  function yFor(v: number): number {
    return padT + plotH - (Math.max(0, v) / maxVal) * plotH;
  }

  const paths = useMemo(() => {
    const toPoints = (vals: number[]): Point2D[] =>
      vals.map((v, i) => ({ x: xFor(i), y: yFor(v) }));

    if (mode === "tokens") {
      const inVals = data.map((d) => d.prompt_tokens);
      const cacheVals = data.map((d) => d.cached_tokens);
      const outVals = data.map((d) => d.completion_tokens);
      const inPts = toPoints(inVals);
      const cachePts = toPoints(cacheVals);
      const outPts = toPoints(outVals);
      return {
        inLine: buildMonotoneLinePath(inPts, padT, baselineY),
        inArea: buildMonotoneAreaPath(inPts, padT, baselineY),
        cacheLine: buildMonotoneLinePath(cachePts, padT, baselineY),
        cacheArea: buildMonotoneAreaPath(cachePts, padT, baselineY),
        outLine: buildMonotoneLinePath(outPts, padT, baselineY),
        outArea: buildMonotoneAreaPath(outPts, padT, baselineY),
      };
    }

    if (mode === "requests") {
      const reqPts = toPoints(data.map((d) => d.requests));
      const rlPts = toPoints(data.map((d) => d.rate_limited_requests));
      return {
        reqLine: buildMonotoneLinePath(reqPts, padT, baselineY),
        reqArea: buildMonotoneAreaPath(reqPts, padT, baselineY),
        rlLine: buildMonotoneLinePath(rlPts, padT, baselineY),
      };
    }

    const spdPts = toPoints(data.map((d) => d.avg_tokens_per_sec));
    return {
      spdLine: buildMonotoneLinePath(spdPts, padT, baselineY),
      spdArea: buildMonotoneAreaPath(spdPts, padT, baselineY),
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data, mode, maxVal, n]);

  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((frac) => maxVal * frac);
  const labelStride = n <= 8 ? 1 : n <= 16 ? 2 : 4;
  const hovered = hoverIdx !== null && data[hoverIdx] ? data[hoverIdx] : null;
  const anyNonZero = data.some((d) =>
    mode === "tokens"
      ? d.prompt_tokens > 0 || d.cached_tokens > 0 || d.completion_tokens > 0
      : mode === "requests"
        ? d.requests > 0 || d.rate_limited_requests > 0
        : d.avg_tokens_per_sec > 0,
  );

  const clipId = `tr-chart-clip-${uid}`;
  const gradInId = `tr-grad-in-${uid}`;
  const gradCacheId = `tr-grad-cache-${uid}`;
  const gradOutId = `tr-grad-out-${uid}`;
  const gradReqId = `tr-grad-req-${uid}`;
  const gradSpdId = `tr-grad-spd-${uid}`;

  function handleKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (n === 0) return;
    if (e.key === "ArrowRight") {
      e.preventDefault();
      setHoverIdx((cur) => (cur === null ? 0 : Math.min(n - 1, cur + 1)));
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      setHoverIdx((cur) => (cur === null ? n - 1 : Math.max(0, cur - 1)));
    } else if (e.key === "Escape") {
      setHoverIdx(null);
    }
  }

  const hoverFlipLeft = hoverIdx !== null && n > 1 && hoverIdx >= Math.floor(n * 0.55);

  return (
    <Card
      title="Traffic & token throughput over time"
      subtitle="Uncached input, cached read, output volume, requests, and generation speed"
      actions={
        <SegmentedControl
          size="sm"
          value={mode}
          onChange={setMode}
          options={[
            { value: "tokens", label: "Tokens" },
            { value: "requests", label: "Requests" },
            { value: "speed", label: "Speed (tok/s)" },
          ]}
        />
      }
    >
      {data.length === 0 || !anyNonZero ? (
        <div className="flex h-52 items-center justify-center rounded-lg border border-dashed border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-xs text-[var(--text-muted)]">
          No telemetry recorded in the selected window yet.
        </div>
      ) : (
        <div className="space-y-3">
          {/* Interactive Series Filter & Aggregate Totals Bar */}
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs tabular-nums">
            {mode === "tokens" ? (
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => toggleSeries("in")}
                  aria-pressed={showInput}
                  title="Click to toggle Uncached input series"
                  className={`inline-flex items-center gap-2 rounded-lg border px-2.5 py-1 transition-[border-color,background-color,opacity,transform] duration-150 active:scale-[0.98] ${
                    showInput
                      ? "border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[var(--text-primary)]"
                      : "border-transparent bg-transparent text-[var(--text-muted)] opacity-55 hover:opacity-80"
                  }`}
                >
                  <span className="h-2 w-2 rounded-full bg-[var(--token-in)]" />
                  <span>Uncached input</span>
                  <span className="font-mono font-semibold text-[var(--token-in)]">
                    {formatCompactNumber(totals.prompt)}
                  </span>
                </button>

                <button
                  type="button"
                  onClick={() => toggleSeries("cache")}
                  aria-pressed={showCached}
                  title="Click to toggle Cached read series"
                  className={`inline-flex items-center gap-2 rounded-lg border px-2.5 py-1 transition-[border-color,background-color,opacity,transform] duration-150 active:scale-[0.98] ${
                    showCached
                      ? "border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[var(--text-primary)]"
                      : "border-transparent bg-transparent text-[var(--text-muted)] opacity-55 hover:opacity-80"
                  }`}
                >
                  <span
                    className="h-0.5 w-3 border-t-2 border-dashed border-[var(--token-cache)]"
                    aria-hidden="true"
                  />
                  <span>Cached read</span>
                  <span className="font-mono font-semibold text-[var(--token-cache)]">
                    {formatCompactNumber(totals.cached)}
                  </span>
                </button>

                <button
                  type="button"
                  onClick={() => toggleSeries("out")}
                  aria-pressed={showOutput}
                  title="Click to toggle Output tokens series"
                  className={`inline-flex items-center gap-2 rounded-lg border px-2.5 py-1 transition-[border-color,background-color,opacity,transform] duration-150 active:scale-[0.98] ${
                    showOutput
                      ? "border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[var(--text-primary)]"
                      : "border-transparent bg-transparent text-[var(--text-muted)] opacity-55 hover:opacity-80"
                  }`}
                >
                  <span className="h-2 w-2 rounded-full bg-[var(--token-out)]" />
                  <span>Output</span>
                  <span className="font-mono font-semibold text-[var(--token-out)]">
                    {formatCompactNumber(totals.completion)}
                  </span>
                </button>
              </div>
            ) : mode === "requests" ? (
              <div className="flex flex-wrap items-center gap-3 text-[var(--text-secondary)]">
                <span className="inline-flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1">
                  <span className="h-2 w-2 rounded-full bg-[var(--accent)]" />
                  <span>Total requests</span>
                  <span className="font-mono font-semibold text-[var(--text-primary)]">
                    {formatCompactNumber(totals.requests)}
                  </span>
                </span>
                <span className="inline-flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1">
                  <span
                    className="h-0.5 w-3 border-t-2 border-dashed border-[var(--warning)]"
                    aria-hidden="true"
                  />
                  <span>Rate-limited (429)</span>
                  <span className="font-mono font-semibold text-[var(--warning)]">
                    {formatCompactNumber(totals.rateLimited)}
                  </span>
                </span>
              </div>
            ) : (
              <div className="flex flex-wrap items-center gap-3 text-[var(--text-secondary)]">
                <span className="inline-flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1">
                  <span className="h-2 w-2 rounded-full bg-[var(--speed)]" />
                  <span>Average speed</span>
                  <span className="font-mono font-semibold text-[var(--speed)]">
                    {formatTps(totals.avgSpeed)}
                  </span>
                </span>
                <span className="inline-flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1">
                  <span className="text-[var(--text-muted)]">Bucket peak</span>
                  <span className="font-mono font-semibold text-[var(--text-primary)]">
                    {formatTps(totals.peakSpeed)}
                  </span>
                </span>
              </div>
            )}

            <span className="hidden font-mono text-[11px] text-[var(--text-muted)] sm:inline">
              Hover or use arrow keys to inspect buckets
            </span>
          </div>

          {/* Plot Canvas Container with Zero-CLS Floating Readout HUD */}
          <div
            tabIndex={0}
            role="region"
            aria-label={`Time-series ${mode} throughput chart`}
            onKeyDown={handleKeyDown}
            onMouseLeave={() => setHoverIdx(null)}
            className="relative rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-canvas)]/55 p-2 focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]"
          >
            <svg viewBox={`0 0 ${w} ${h}`} className="block h-auto w-full select-none overflow-hidden">
              <defs>
                <clipPath id={clipId}>
                  <rect x={padL} y={padT} width={plotW} height={plotH} rx="2" />
                </clipPath>

                <linearGradient id={gradInId} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--token-in)" stopOpacity="0.26" />
                  <stop offset="100%" stopColor="var(--token-in)" stopOpacity="0.01" />
                </linearGradient>
                <linearGradient id={gradCacheId} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--token-cache)" stopOpacity="0.14" />
                  <stop offset="100%" stopColor="var(--token-cache)" stopOpacity="0.0" />
                </linearGradient>
                <linearGradient id={gradOutId} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--token-out)" stopOpacity="0.22" />
                  <stop offset="100%" stopColor="var(--token-out)" stopOpacity="0.01" />
                </linearGradient>
                <linearGradient id={gradReqId} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.24" />
                  <stop offset="100%" stopColor="var(--accent)" stopOpacity="0.01" />
                </linearGradient>
                <linearGradient id={gradSpdId} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="var(--speed)" stopOpacity="0.24" />
                  <stop offset="100%" stopColor="var(--speed)" stopOpacity="0.01" />
                </linearGradient>
              </defs>

              {/* Horizontal gridlines & Y ticks */}
              {yTicks.map((tick, idx) => {
                const y = yFor(tick);
                const label =
                  mode === "speed"
                    ? `${Math.round(tick)}`
                    : tick === 0
                      ? "0"
                      : formatCompactNumber(Math.round(tick));
                return (
                  <g key={idx}>
                    <line
                      x1={padL}
                      x2={w - padR}
                      y1={y}
                      y2={y}
                      stroke="var(--border-subtle)"
                      strokeDasharray={idx === 0 ? undefined : "3 4"}
                      opacity={idx === 0 ? 0.9 : 0.7}
                    />
                    <text
                      x={padL - 9}
                      y={y + 3.5}
                      textAnchor="end"
                      fill="var(--text-muted)"
                      fontSize="10"
                      fontFamily="var(--font-mono)"
                    >
                      {label}
                    </text>
                  </g>
                );
              })}

              {/* Clipped Area + Monotone Cubic Curves */}
              <g clipPath={`url(#${clipId})`}>
                {mode === "tokens" && (
                  <>
                    {showCached && (
                      <>
                        <path d={paths.cacheArea} fill={`url(#${gradCacheId})`} />
                        <path
                          d={paths.cacheLine}
                          fill="none"
                          stroke="var(--token-cache)"
                          strokeWidth="1.85"
                          strokeDasharray="5 3"
                          strokeLinecap="round"
                        />
                      </>
                    )}
                    {showInput && (
                      <>
                        <path d={paths.inArea} fill={`url(#${gradInId})`} />
                        <path
                          d={paths.inLine}
                          fill="none"
                          stroke="var(--token-in)"
                          strokeWidth="2.15"
                          strokeLinecap="round"
                        />
                      </>
                    )}
                    {showOutput && (
                      <>
                        <path d={paths.outArea} fill={`url(#${gradOutId})`} />
                        <path
                          d={paths.outLine}
                          fill="none"
                          stroke="var(--token-out)"
                          strokeWidth="2"
                          strokeLinecap="round"
                        />
                      </>
                    )}
                  </>
                )}

                {mode === "requests" && (
                  <>
                    <path d={paths.reqArea} fill={`url(#${gradReqId})`} />
                    <path
                      d={paths.reqLine}
                      fill="none"
                      stroke="var(--accent)"
                      strokeWidth="2.15"
                      strokeLinecap="round"
                    />
                    <path
                      d={paths.rlLine}
                      fill="none"
                      stroke="var(--warning)"
                      strokeWidth="1.85"
                      strokeDasharray="5 3"
                      strokeLinecap="round"
                    />
                  </>
                )}

                {mode === "speed" && (
                  <>
                    <path d={paths.spdArea} fill={`url(#${gradSpdId})`} />
                    <path
                      d={paths.spdLine}
                      fill="none"
                      stroke="var(--speed)"
                      strokeWidth="2.15"
                      strokeLinecap="round"
                    />
                  </>
                )}
              </g>

              {/* Active Bucket Crosshair + Series Markers */}
              {hoverIdx !== null && hovered && (
                <g>
                  <line
                    x1={xFor(hoverIdx)}
                    x2={xFor(hoverIdx)}
                    y1={padT}
                    y2={baselineY}
                    stroke="var(--border-strong)"
                    strokeWidth="1.25"
                    strokeDasharray="3 3"
                  />

                  {mode === "tokens" && showCached && (
                    <circle
                      cx={xFor(hoverIdx)}
                      cy={yFor(hovered.cached_tokens)}
                      r="4"
                      fill="var(--token-cache)"
                      stroke="var(--bg-surface)"
                      strokeWidth="2"
                    />
                  )}
                  {mode === "tokens" && showInput && (
                    <circle
                      cx={xFor(hoverIdx)}
                      cy={yFor(hovered.prompt_tokens)}
                      r="4.25"
                      fill="var(--token-in)"
                      stroke="var(--bg-surface)"
                      strokeWidth="2"
                    />
                  )}
                  {mode === "tokens" && showOutput && (
                    <circle
                      cx={xFor(hoverIdx)}
                      cy={yFor(hovered.completion_tokens)}
                      r="4"
                      fill="var(--token-out)"
                      stroke="var(--bg-surface)"
                      strokeWidth="2"
                    />
                  )}
                  {mode === "requests" && (
                    <>
                      <circle
                        cx={xFor(hoverIdx)}
                        cy={yFor(hovered.requests)}
                        r="4.25"
                        fill="var(--accent)"
                        stroke="var(--bg-surface)"
                        strokeWidth="2"
                      />
                      {hovered.rate_limited_requests > 0 && (
                        <circle
                          cx={xFor(hoverIdx)}
                          cy={yFor(hovered.rate_limited_requests)}
                          r="3.75"
                          fill="var(--warning)"
                          stroke="var(--bg-surface)"
                          strokeWidth="2"
                        />
                      )}
                    </>
                  )}
                  {mode === "speed" && (
                    <circle
                      cx={xFor(hoverIdx)}
                      cy={yFor(hovered.avg_tokens_per_sec)}
                      r="4.25"
                      fill="var(--speed)"
                      stroke="var(--bg-surface)"
                      strokeWidth="2"
                    />
                  )}
                </g>
              )}

              {/* X-axis labels */}
              {(() => {
                const lastX = xFor(n - 1);
                const minGapPx = 64;
                const indices: number[] = [];
                let prevX = -Infinity;
                for (let i = 0; i < n; i++) {
                  const cx = xFor(i);
                  const isLast = i === n - 1;
                  if (isLast) {
                    if (indices.length > 0 && cx - prevX < minGapPx) {
                      indices.pop();
                    }
                    indices.push(i);
                    break;
                  }
                  if (
                    i === 0 ||
                    (i % labelStride === 0 && cx - prevX >= minGapPx && lastX - cx >= minGapPx)
                  ) {
                    indices.push(i);
                    prevX = cx;
                  }
                }
                return indices.map((i) => {
                  const d = data[i];
                  if (!d) return null;
                  const activeLabel = hoverIdx === i;
                  return (
                    <text
                      key={`xl-${d.timestamp}-${i}`}
                      x={xFor(i)}
                      y={h - 7}
                      textAnchor="middle"
                      fill={activeLabel ? "var(--text-primary)" : "var(--text-muted)"}
                      fontWeight={activeLabel ? "600" : "400"}
                      fontSize="10"
                      fontFamily="var(--font-mono)"
                    >
                      {d.label}
                    </text>
                  );
                });
              })()}

              {/* Invisible Hit Columns across plot width */}
              {data.map((_, i) => {
                const colW = n > 1 ? plotW / n : plotW;
                return (
                  <rect
                    key={`hit-${i}`}
                    x={Math.max(padL, xFor(i) - colW / 2)}
                    y={padT}
                    width={colW}
                    height={plotH}
                    fill="transparent"
                    className="cursor-crosshair"
                    onMouseEnter={() => setHoverIdx(i)}
                  />
                );
              })}
            </svg>

            {/* Floating Zero-CLS Bucket Telemetry HUD */}
            {hovered && hoverIdx !== null && (
              <div
                style={{
                  left: hoverFlipLeft
                    ? undefined
                    : `clamp(64px, ${((xFor(hoverIdx) / w) * 100 + 2).toFixed(1)}%, calc(100% - 220px))`,
                  right: hoverFlipLeft
                    ? `clamp(16px, ${(100 - (xFor(hoverIdx) / w) * 100 + 2).toFixed(1)}%, calc(100% - 220px))`
                    : undefined,
                }}
                className="pointer-events-none absolute top-3 z-10 w-48 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface)]/95 px-3 py-2 text-[11px] shadow-md backdrop-blur-sm tabular-nums"
              >
                <div className="mb-1.5 flex items-center justify-between border-b border-[var(--border-subtle)] pb-1">
                  <span className="font-mono font-semibold text-[var(--text-primary)]">
                    {hovered.label}
                  </span>
                  <span className="font-mono text-[10px] text-[var(--text-muted)]">
                    {hovered.requests} req
                    {hovered.rate_limited_requests > 0
                      ? ` · ${hovered.rate_limited_requests} 429`
                      : ""}
                  </span>
                </div>
                <div className="space-y-1 font-mono text-[11px]">
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Uncached In</span>
                    <span className="font-semibold text-[var(--token-in)]">
                      {formatCompactNumber(hovered.prompt_tokens)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Cached Read</span>
                    <span className="font-semibold text-[var(--token-cache)]">
                      {formatCompactNumber(hovered.cached_tokens)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Output</span>
                    <span className="font-semibold text-[var(--token-out)]">
                      {formatCompactNumber(hovered.completion_tokens)}
                    </span>
                  </div>
                  {hovered.avg_tokens_per_sec > 0 && (
                    <div className="flex items-center justify-between border-t border-[var(--border-subtle)] pt-1">
                      <span className="text-[var(--text-muted)]">Speed</span>
                      <span className="font-semibold text-[var(--speed)]">
                        {formatTps(hovered.avg_tokens_per_sec)}
                      </span>
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </Card>
  );
}
