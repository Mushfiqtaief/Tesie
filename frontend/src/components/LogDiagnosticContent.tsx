import { useState } from "react";
import { Check, Copy, FileJson, Globe, Hash, Repeat, ShieldAlert } from "lucide-react";
import type { AttemptLeg, UsageLogRecord } from "../types";
import {
  formatCompactNumber,
  formatCooldownReason,
  formatMs,
  formatTps,
  providerDisplayName,
  statusColorStyle,
} from "../utils/format";
import { Button, StatusPill } from "./UiPrimitives";

const ATTEMPT_LEGS_CACHE = new Map<string, AttemptLeg[]>();
const MAX_LEGS_CACHE_SIZE = 500;

export function parseAttemptLegs(rawTrace?: string | null): AttemptLeg[] {
  if (!rawTrace) return [];
  const cached = ATTEMPT_LEGS_CACHE.get(rawTrace);
  if (cached !== undefined) return cached;
  let parsedLegs: AttemptLeg[] = [];
  try {
    const parsed = JSON.parse(rawTrace);
    parsedLegs = Array.isArray(parsed) ? (parsed as AttemptLeg[]) : [];
  } catch {
    parsedLegs = [];
  }
  if (ATTEMPT_LEGS_CACHE.size >= MAX_LEGS_CACHE_SIZE) {
    ATTEMPT_LEGS_CACHE.clear();
  }
  ATTEMPT_LEGS_CACHE.set(rawTrace, parsedLegs);
  return parsedLegs;
}

export function sessionLabelForProvider(providerId: string): string {
  const lower = (providerId || "").toLowerCase();
  if (lower === "antigravity" || lower === "ag") return "Antigravity Session";
  if (lower === "opencode" || lower === "oc") return "OpenCode Session";
  return "Upstream Session";
}

export function extractLegMeta(legs: AttemptLeg[]) {
  const winningLeg = [...legs].reverse().find((l) => l.status >= 200 && l.status < 300);
  const primaryLeg = winningLeg ?? legs[legs.length - 1];
  const anyAccountLeg = primaryLeg?.account_email
    ? primaryLeg
    : [...legs].reverse().find((l) => !!l.account_email || !!l.account_name);
  return {
    accountEmail: anyAccountLeg?.account_email || anyAccountLeg?.account_name || null,
    wireModel: primaryLeg?.wire_model || null,
    reasoningLevel: primaryLeg?.reasoning_level || null,
  };
}

export function buildDiagnosticReport(log: UsageLogRecord) {
  const legs = parseAttemptLegs(log.attempt_trace);
  const meta = extractLegMeta(legs);
  const grossInputTokens = log.prompt_tokens + log.cached_tokens;
  return {
    id: log.id,
    timestamp_iso: new Date(log.created_at).toISOString(),
    request_id: log.request_id,
    session_label: sessionLabelForProvider(log.provider_id),
    session_id: log.session_id ?? null,
    provider_id: log.provider_id,
    model: log.model,
    wire_model: meta.wireModel,
    reasoning_level: meta.reasoningLevel,
    account: meta.accountEmail,
    wire_format: log.wire_format ?? null,
    upstream_url: log.upstream_url ?? null,
    status_code: log.status_code,
    proxy_name: log.proxy_name,
    egress_ip: log.egress_ip,
    attempt_count: log.attempt_count ?? 1,
    attempt_legs: legs,
    metrics: {
      latency_ms: log.latency_ms,
      ttft_ms: log.ttft_ms,
      tokens_per_sec: log.tokens_per_sec,
      uncached_input_tokens: log.prompt_tokens,
      cached_input_tokens: log.cached_tokens,
      gross_input_tokens: grossInputTokens,
      completion_tokens: log.completion_tokens,
      total_tokens: log.total_tokens,
    },
    error_kind: log.error_kind,
    error_summary: formatCooldownReason(log.error_detail) || null,
    error_detail: log.error_detail,
  };
}

export function buildSingleLineSummary(log: UsageLogRecord): string {
  const ts = new Date(log.created_at).toISOString();
  const attempts = log.attempt_count ?? 1;
  const legs = parseAttemptLegs(log.attempt_trace);
  const meta = extractLegMeta(legs);
  const acct = meta.accountEmail ? ` | acct=${meta.accountEmail}` : "";
  const level = meta.reasoningLevel ? ` [${meta.reasoningLevel}]` : "";
  const route = `${log.proxy_name}${log.egress_ip ? ` (${log.egress_ip})` : ""}`;
  const perf =
    log.status_code >= 200 && log.status_code < 300
      ? `${log.prompt_tokens}in/${log.cached_tokens}cache/${log.completion_tokens}out | ${formatTps(log.tokens_per_sec)} | ${formatMs(log.latency_ms)}`
      : `${log.error_kind || "error"}: ${(log.error_detail || "").replace(/\s+/g, " ").slice(0, 180)} | ${formatMs(log.latency_ms)}`;
  return `[${ts}] HTTP ${log.status_code} | ${log.model}${level}${log.wire_format ? ` (${log.wire_format})` : ""}${acct} | ${route} | attempts=${attempts}${log.session_id ? ` | session=${log.session_id}` : ""} | ${perf}`;
}

export function LogDiagnosticContent({
  log,
  onNotify,
}: {
  log: UsageLogRecord;
  onNotify?: (msg: string, isErr?: boolean) => void;
}) {
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  function copyWithFeedback(key: string, text: string, toastMsg?: string) {
    void navigator.clipboard.writeText(text);
    setCopiedKey(key);
    if (toastMsg && onNotify) {
      onNotify(toastMsg);
    }
    setTimeout(() => {
      setCopiedKey((cur) => (cur === key ? null : cur));
    }, 1600);
  }

  const st = statusColorStyle(log.status_code);
  const legs = parseAttemptLegs(log.attempt_trace);
  const meta = extractLegMeta(legs);
  const reportJson = JSON.stringify(buildDiagnosticReport(log), null, 2);
  const summaryLine = buildSingleLineSummary(log);
  const grossContextTokens = log.prompt_tokens + log.cached_tokens;

  return (
    <div className="space-y-4 text-xs">
      {/* Status + Copy Action Bar */}
      <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
        <div className="flex flex-wrap items-center gap-2">
          <span
            className="inline-flex items-center gap-1 rounded-md border px-2 py-0.5 font-mono font-semibold"
            style={{
              backgroundColor: st.bg,
              color: st.fg,
              borderColor: st.border,
            }}
          >
            HTTP {log.status_code}
          </span>
          {log.wire_format && <StatusPill tone="neutral">{log.wire_format}</StatusPill>}
          {meta.reasoningLevel && (
            <StatusPill tone="accent">level: {meta.reasoningLevel}</StatusPill>
          )}
          {(log.attempt_count ?? 1) > 1 && (
            <StatusPill tone="warning">{log.attempt_count} attempts</StatusPill>
          )}
          {log.error_kind && (
            <span className="font-mono font-medium text-[var(--danger)]">{log.error_kind}</span>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          <Button
            variant="secondary"
            size="sm"
            onClick={() =>
              copyWithFeedback("drawer-summary", summaryLine, "Copied single-line summary")
            }
          >
            {copiedKey === "drawer-summary" ? (
              <Check className="h-3.5 w-3.5 text-[var(--success)]" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
            Copy Summary
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={() =>
              copyWithFeedback("drawer-json", reportJson, "Copied full diagnostic JSON")
            }
          >
            {copiedKey === "drawer-json" ? (
              <Check className="h-3.5 w-3.5 text-[var(--success)]" />
            ) : (
              <FileJson className="h-3.5 w-3.5" />
            )}
            Copy JSON
          </Button>
        </div>
      </div>

      {/* Primary Metrics Grid */}
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
          <div className="text-[11px] text-[var(--text-muted)]">Model &amp; Provider</div>
          <div className="mt-1 font-mono font-semibold text-[var(--text-primary)]">
            {log.model}
          </div>
          <div className="mt-0.5 text-[11px] text-[var(--text-muted)]">
            {providerDisplayName(log.provider_id)}
            {meta.wireModel && meta.wireModel !== log.model && (
              <span className="ml-1 font-mono text-[var(--text-secondary)]">
                &rarr; {meta.wireModel}
              </span>
            )}
          </div>
        </div>
        <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
          <div className="text-[11px] text-[var(--text-muted)]">
            {meta.accountEmail ? "Account & Outbound Route" : "Outbound & Exit IP"}
          </div>
          {meta.accountEmail && (
            <div className="mt-1 truncate font-medium text-[var(--accent)]" title={meta.accountEmail}>
              {meta.accountEmail}
            </div>
          )}
          <div
            className={`${meta.accountEmail ? "mt-0.5" : "mt-1"} font-mono font-medium text-[var(--text-primary)]`}
          >
            {log.proxy_name}
          </div>
          <div className="mt-0.5 font-mono text-[11px] text-[var(--token-cache)]">
            {log.egress_ip || "No IP recorded"}
          </div>
        </div>
        <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
          <div className="text-[11px] text-[var(--text-muted)]">Generation Speed</div>
          <div className="mt-1 font-mono text-base font-semibold text-[var(--speed)]">
            {formatTps(log.tokens_per_sec)}
          </div>
        </div>
        <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
          <div className="text-[11px] text-[var(--text-muted)]">TTFT / Total Duration</div>
          <div className="mt-1 font-mono font-medium text-[var(--text-primary)]">
            {formatMs(log.ttft_ms)} / {formatMs(log.latency_ms)}
          </div>
        </div>
      </div>

      {/* Token Breakdown: Uncached Input vs Cached Read vs Output vs Net/Processed Totals */}
      <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
        <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-[var(--text-muted)]">
          <span>Token Breakdown</span>
          <span className="font-mono text-[10px]">
            Gross context {formatCompactNumber(grossContextTokens)} &middot; All tokens{" "}
            {formatCompactNumber(grossContextTokens + log.completion_tokens)}
          </span>
        </div>
        <div className="mt-2 grid grid-cols-4 gap-2 font-mono tabular-nums">
          <div>
            <div className="text-[11px] text-[var(--text-muted)]">Uncached In</div>
            <div className="font-semibold text-[var(--token-in)]">{log.prompt_tokens}</div>
          </div>
          <div>
            <div className="text-[11px] text-[var(--text-muted)]">Cached Read</div>
            <div className="font-semibold text-[var(--token-cache)]">{log.cached_tokens}</div>
          </div>
          <div>
            <div className="text-[11px] text-[var(--text-muted)]">Output</div>
            <div className="font-semibold text-[var(--token-out)]">{log.completion_tokens}</div>
          </div>
          <div>
            <div className="text-[11px] text-[var(--text-muted)]" title="Uncached In + Output">
              Net (In+Out)
            </div>
            <div className="font-semibold text-[var(--text-primary)]">{log.total_tokens}</div>
          </div>
        </div>
      </div>

      {/* Upstream Route, Session & Request Identity */}
      <div className="space-y-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
        <div className="flex items-center justify-between gap-2">
          <span className="inline-flex items-center gap-1 text-[11px] text-[var(--text-muted)]">
            <Hash className="h-3 w-3" />
            Request ID
          </span>
          <button
            type="button"
            onClick={() => copyWithFeedback("req-id", log.request_id, "Copied request ID")}
            className="inline-flex items-center gap-1 font-mono text-[11px] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
          >
            <span className="max-w-[260px] truncate">{log.request_id}</span>
            {copiedKey === "req-id" ? (
              <Check className="h-3 w-3 text-[var(--success)]" />
            ) : (
              <Copy className="h-3 w-3" />
            )}
          </button>
        </div>

        {log.session_id && (
          <div className="flex items-center justify-between gap-2 border-t border-[var(--border-subtle)] pt-2">
            <span className="text-[11px] text-[var(--text-muted)]">
              {sessionLabelForProvider(log.provider_id)}
            </span>
            <button
              type="button"
              onClick={() =>
                copyWithFeedback("ses-id", log.session_id || "", "Copied session ID")
              }
              className="inline-flex items-center gap-1 font-mono text-[11px] text-[var(--token-cache)] hover:underline"
            >
              <span>{log.session_id}</span>
              {copiedKey === "ses-id" ? (
                <Check className="h-3 w-3 text-[var(--success)]" />
              ) : (
                <Copy className="h-3 w-3" />
              )}
            </button>
          </div>
        )}

        {log.upstream_url && (
          <div className="border-t border-[var(--border-subtle)] pt-2">
            <div className="flex items-center justify-between gap-2">
              <span className="inline-flex items-center gap-1 text-[11px] text-[var(--text-muted)]">
                <Globe className="h-3 w-3" />
                Upstream Endpoint
              </span>
              <button
                type="button"
                onClick={() =>
                  copyWithFeedback("up-url", log.upstream_url || "", "Copied upstream URL")
                }
                className="inline-flex items-center gap-1 text-[11px] text-[var(--accent)] hover:underline"
              >
                {copiedKey === "up-url" ? (
                  <Check className="h-3 w-3 text-[var(--success)]" />
                ) : (
                  <Copy className="h-3 w-3" />
                )}
                Copy URL
              </button>
            </div>
            <div className="mt-1 break-all font-mono text-[11px] text-[var(--text-secondary)]">
              {log.upstream_url}
            </div>
          </div>
        )}

        <div className="flex items-center justify-between border-t border-[var(--border-subtle)] pt-2">
          <span className="text-[11px] text-[var(--text-muted)]">Timestamp</span>
          <span className="font-mono text-[11px] text-[var(--text-secondary)]">
            {new Date(log.created_at).toISOString()}
          </span>
        </div>
      </div>

      {/* Attempt Chain / Failover Timeline */}
      {legs.length > 0 && (
        <div className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3">
          <div className="mb-2 flex items-center justify-between">
            <span className="inline-flex items-center gap-1.5 text-[11px] font-medium text-[var(--text-secondary)]">
              <Repeat className="h-3.5 w-3.5 text-[var(--accent)]" />
              Upstream Attempt &amp; Cooldown Chain ({legs.length}{" "}
              {legs.length === 1 ? "leg" : "legs"})
            </span>
          </div>
          <div className="space-y-2">
            {legs.map((leg, idx) => {
              const legStyle = statusColorStyle(leg.status);
              const isBannedLeg = leg.cooldown_applied?.toLowerCase().includes("auto-disabled");
              return (
                <div
                  key={`${leg.attempt}-${leg.proxy}-${idx}`}
                  className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 font-mono text-[11px]"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                      <span className="text-[var(--text-muted)]">#{leg.attempt}</span>
                      {leg.account_email && (
                        <span className="font-sans font-semibold text-[var(--accent)]">
                          {leg.account_email}
                        </span>
                      )}
                      <span className="truncate font-semibold text-[var(--text-primary)]">
                        {leg.proxy}
                      </span>
                      {leg.egress_ip && (
                        <span className="text-[var(--token-cache)]">({leg.egress_ip})</span>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <span
                        className="rounded border px-1.5 py-0.2 text-[10px] font-semibold"
                        style={{
                          backgroundColor: legStyle.bg,
                          color: legStyle.fg,
                          borderColor: legStyle.border,
                        }}
                      >
                        {leg.status}
                      </span>
                      <span className="text-[var(--text-muted)]">{formatMs(leg.latency_ms)}</span>
                    </div>
                  </div>
                  <div className="mt-1 flex flex-wrap items-center justify-between gap-2 text-[10px] text-[var(--text-muted)]">
                    <span className="flex flex-wrap items-center gap-2">
                      <span>phase: {leg.phase}</span>
                      {leg.wire_model && (
                        <span className="text-[var(--text-secondary)]">
                          wire: {leg.wire_model}
                          {leg.reasoning_level ? ` (${leg.reasoning_level})` : ""}
                        </span>
                      )}
                    </span>
                    {leg.session_id && <span>{leg.session_id}</span>}
                  </div>
                  {leg.cooldown_applied && (
                    <div
                      className={`mt-1.5 inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium ${
                        isBannedLeg
                          ? "bg-[var(--danger-soft)] text-[var(--danger)]"
                          : "bg-[var(--warning-soft)] text-[var(--warning)]"
                      }`}
                    >
                      <ShieldAlert className="h-3 w-3 shrink-0" />
                      <span>Action: {leg.cooldown_applied}</span>
                    </div>
                  )}
                  {leg.detail && (
                    <div className="mt-1 break-words text-[10px] text-[var(--danger)]">
                      {leg.detail}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Error Detail Payload + Copy Error */}
      {log.error_detail && (
        <div className="rounded-lg border border-[color-mix(in_srgb,var(--danger)_30%,transparent)] bg-[var(--danger-soft)] p-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <div>
              <div className="font-semibold text-[var(--danger)]">
                Upstream / Relay Error Payload
              </div>
              {formatCooldownReason(log.error_detail) && (
                <div className="text-[11px] text-[var(--text-secondary)]">
                  {formatCooldownReason(log.error_detail)}
                </div>
              )}
            </div>
            <Button
              variant="secondary"
              size="sm"
              onClick={() =>
                copyWithFeedback("err-raw", log.error_detail || "", "Copied raw error payload")
              }
            >
              {copiedKey === "err-raw" ? (
                <Check className="h-3.5 w-3.5 text-[var(--success)]" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
              Copy Error
            </Button>
          </div>
          <pre className="smooth-scroll max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-[var(--bg-surface)]/80 p-2.5 font-mono text-[11px] text-[var(--text-primary)]">
            {log.error_detail}
          </pre>
        </div>
      )}
    </div>
  );
}
