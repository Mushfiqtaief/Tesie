export function formatNumber(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "0";
  return new Intl.NumberFormat().format(n);
}

export function formatCompactNumber(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`;
  return new Intl.NumberFormat().format(n);
}

export function formatTps(tps: number | null | undefined): string {
  if (tps === null || tps === undefined || tps <= 0) return "—";
  return `${tps.toFixed(1)} tok/s`;
}

export function formatMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || ms < 0) return "—";
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)}s`;
  return `${Math.round(ms)}ms`;
}

export function formatCountdown(targetMs: number | null | undefined, nowMs: number): string {
  if (!targetMs || targetMs <= nowMs) return "Ready";
  const diffSec = Math.ceil((targetMs - nowMs) / 1000);
  if (diffSec < 60) return `${diffSec}s`;
  const mins = Math.floor(diffSec / 60);
  const secs = diffSec % 60;
  if (mins < 60) return `${mins}m ${secs}s`;
  const hrs = Math.floor(mins / 60);
  const remMin = mins % 60;
  return `${hrs}h ${remMin}m`;
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (!ms || ms <= 0) return "—";
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = ms / 3_600_000;
  return Number.isInteger(hr) ? `${hr}h` : `${hr.toFixed(1)}h`;
}

export function formatTimeAgo(tsMs: number | null | undefined, nowMs: number): string {
  if (!tsMs) return "never";
  const diffSec = Math.max(0, Math.floor((nowMs - tsMs) / 1000));
  if (diffSec < 5) return "just now";
  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  return `${Math.floor(diffSec / 86400)}d ago`;
}

export function formatClockTime(tsMs: number): string {
  try {
    return new Date(tsMs).toLocaleTimeString([], {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return String(tsMs);
  }
}

export function formatPct(part: number, total: number): string {
  if (!total || total <= 0) return "0%";
  return `${((part / total) * 100).toFixed(1)}%`;
}

export function maskProxyUrl(url: string | null | undefined): string {
  if (!url) return "Direct interface";
  try {
    const parsed = new URL(url);
    if (parsed.password) {
      parsed.password = "••••";
    }
    if (parsed.username && parsed.username.length > 6) {
      parsed.username = `${parsed.username.slice(0, 3)}•••`;
    }
    return parsed.toString().replace(/%E2%80%A2/gi, "•");
  } catch {
    return url.replace(/:([^:@/]+)@/, ":••••@");
  }
}

/**
 * Serializes a stored proxy node back into a shareable `scheme://[user:pass@]host:port`
 * URL, falling back to host/port when only structured fields are stored. The output
 * round-trips through the batch import endpoint.
 */
export function proxyExportUrl(proxy: {
  is_direct: boolean;
  proxy_url?: string | null;
  proxy_type: string;
  host?: string | null;
  port?: number | null;
  username?: string | null;
  password?: string | null;
}): string | null {
  if (proxy.is_direct) return null;
  const stored = proxy.proxy_url?.trim();
  if (stored) return stored;

  const host = proxy.host?.trim();
  if (!host || !proxy.port) return null;
  const rawType = proxy.proxy_type.trim().toLowerCase();
  const scheme =
    rawType === "socks4" || rawType === "socks4a"
      ? "socks4"
      : rawType.startsWith("socks") || rawType === "ss"
        ? rawType
        : rawType === "https"
          ? "https"
          : "http";
  const auth = proxy.username
    ? `${encodeURIComponent(proxy.username)}${
        proxy.password ? `:${encodeURIComponent(proxy.password)}` : ""
      }@`
    : "";
  return `${scheme}://${auth}${host.includes(":") ? `[${host}]` : host}:${proxy.port}`;
}

/**
 * Converts verbose upstream error JSON/strings into a concise, human-readable
 * cooldown summary so quota rows/cards stay crisp instead of dumping raw JSON.
 */
export function formatCooldownReason(raw: string | null | undefined): string {
  if (!raw || !raw.trim()) return "";
  let cleaned = raw.trim();

  // Strip optional leading HTTP code prefix like `[429] `
  let statusPrefix: string | null = null;
  const codeMatch = cleaned.match(/^\[(\d{3})\]\s*(.*)$/s);
  if (codeMatch) {
    statusPrefix = codeMatch[1];
    cleaned = codeMatch[2].trim();
  }

  // Try parsing structured upstream JSON error payloads
  if (cleaned.startsWith("{")) {
    try {
      const parsed = JSON.parse(cleaned) as Record<string, unknown>;
      const errObj =
        typeof parsed.error === "object" && parsed.error !== null
          ? (parsed.error as Record<string, unknown>)
          : parsed;
      const errType = typeof errObj.type === "string" ? errObj.type : "";
      const errCode = typeof errObj.code === "string" ? errObj.code : "";
      const phase = typeof parsed.phase === "string" ? parsed.phase : "";
      const detailStr = typeof parsed.detail === "string" ? parsed.detail : "";
      const errMsg =
        typeof errObj.message === "string"
          ? errObj.message
          : typeof parsed.error === "string"
            ? parsed.error
            : "";

      const combined = `${errType} ${errCode} ${phase} ${errMsg} ${detailStr}`.toLowerCase();
      if (phase === "eof" || combined.includes("upstream_dial_failed") && combined.includes("eof")) {
        return "Proxy TLS/TCP tunnel dropped (EOF — marked idle)";
      }
      if (phase === "tls_verify_failed" || combined.includes("x509:") || combined.includes("certificate")) {
        return "Proxy TLS certificate check failed (marked idle)";
      }
      if (phase === "proxy_connect_failed") {
        return "Proxy authentication / SOCKS5 connect failed";
      }
      if (combined.includes("ip") && (combined.includes("blocked") || combined.includes("suspicious"))) {
        return "Exit IP temporarily flagged by upstream (403)";
      }
      if (
        errType === "FreeUsageLimitError" ||
        combined.includes("free usage limit") ||
        combined.includes("free_usage_limit")
      ) {
        return "Free tier IP quota reached";
      }
      if (
        combined.includes("account_disabled_tos") ||
        combined.includes("violation of terms") ||
        combined.includes("disabled in this account")
      ) {
        return "Account auto-disabled: Terms of Service violation";
      }
      if (combined.includes("rate limit") || combined.includes("too many requests") || statusPrefix === "429") {
        return "Rate limit reached (429)";
      }
      if (combined.includes("insufficient_quota") || combined.includes("billing") || combined.includes("credit")) {
        return "Account credit / quota exhausted";
      }
      if (combined.includes("unauthorized") || combined.includes("invalid api key") || statusPrefix === "401") {
        return "Invalid or expired API key";
      }
      if (combined.includes("forbidden") || statusPrefix === "403") {
        return "Upstream access restricted (403)";
      }
      if (errMsg) {
        // Strip promotional trailing clauses (e.g. "; subscribe to OpenCode Go...")
        const firstClause = errMsg.split(/;\s*subscribe|\.\s*Free models work/i)[0].trim();
        return firstClause.length > 68 ? `${firstClause.slice(0, 65)}...` : firstClause;
      }
      if (errType) return errType;
    } catch {
      // fall through to text heuristics
    }
  }

  const lower = cleaned.toLowerCase();
  if (
    lower.includes("account_disabled_tos") ||
    lower.includes("violation of terms") ||
    lower.includes("disabled in this account")
  ) {
    return "Account auto-disabled: Terms of Service violation";
  }
  if (lower.startsWith("no quota available for ")) {
    return cleaned.length > 96 ? `${cleaned.slice(0, 93)}...` : cleaned;
  }
  if (lower.includes("all accounts for ") && lower.includes("cooling down")) {
    return cleaned.length > 96 ? `${cleaned.slice(0, 93)}...` : cleaned;
  }
  if (lower.includes("freeusagelimiterror") || lower.includes("free usage limit")) {
    return "Free tier IP quota reached";
  }
  if (lower.includes("soft request cap") || lower.includes("requests limit")) {
    return "Window request cap reached";
  }
  if (lower.includes("soft token cap") || lower.includes("tokens limit")) {
    return "Window token cap reached";
  }
  if (lower.includes("timed out") || lower.includes("timeout")) {
    return "Upstream gateway timeout";
  }
  if (lower.includes("connection refused") || lower.includes("connect error") || lower.includes("socks")) {
    return "Proxy relay unreachable";
  }
  if (statusPrefix === "429" || lower.includes("rate limit")) {
    return "Rate limit reached (429)";
  }
  if (statusPrefix === "401" || statusPrefix === "403") {
    return `Upstream rejected (${statusPrefix})`;
  }
  if (statusPrefix && statusPrefix.startsWith("5")) {
    return `Upstream unavailable (${statusPrefix})`;
  }

  return cleaned.length > 68 ? `${cleaned.slice(0, 65)}...` : cleaned;
}

export function providerDisplayName(id: string): string {
  switch (id) {
    case "opencode":
      return "OpenCode Free";
    case "antigravity":
    case "ag":
      return "Antigravity";
    case "combo":
      return "Combo Chain";
    default:
      return id;
  }
}

export function getProviderLogo(idOrModel: string): string {
  const lower = idOrModel.toLowerCase();
  if (lower === "antigravity" || lower === "ag" || lower.startsWith("ag/")) {
    return "/providers/antigravity.png";
  }
  if (lower === "opencode" || lower.startsWith("oc/")) {
    return "/providers/opencode.png";
  }
  if (lower === "ovh" || lower.startsWith("ovh/")) {
    return "/providers/ovh.png";
  }
  if (
    lower === "cp-lf" ||
    lower === "lf" ||
    lower.startsWith("lf/") ||
    lower.includes("logfare") ||
    lower.includes("logflare")
  ) {
    return "/providers/logfare.png";
  }
  if (lower.includes("claude") || lower.includes("anthropic")) {
    return "/providers/claude.png";
  }
  if (lower.includes("gemini")) {
    return "/providers/gemini.png";
  }
  if (lower.includes("codex") || lower.includes("gpt") || lower.includes("openai")) {
    return "/providers/openai.png";
  }
  return "";
}

export function routingStrategyLabel(strategy?: string | null): string {
  switch ((strategy || "").trim().toLowerCase()) {
    case "round-robin":
    case "round_robin":
    case "rr":
      return "Round-Robin";
    case "random":
    case "rand":
      return "Random";
    case "fallback":
    case "smart_fallback":
    case "smart-fallback":
    default:
      return "Fallback";
  }
}

export function proxyModeLabel(mode: string | null | undefined, selectedCount?: number): string {
  switch (mode) {
    case "all":
    case "pool_strict":
    case "pool_with_direct":
    case "pool":
      return "All Pool Proxies";
    case "selected":
      if (selectedCount === 1) return "1 Selected Proxy";
      return selectedCount && selectedCount > 1
        ? `${selectedCount} Selected Proxies`
        : "Direct (No Proxy)";
    case "direct":
    case "none":
      return "Direct (No Proxy)";
    default:
      return "All Pool Proxies";
  }
}

export function statusColorStyle(code: number): {
  bg: string;
  fg: string;
  border: string;
} {
  if (code >= 200 && code < 300) {
    return {
      bg: "var(--success-soft)",
      fg: "var(--success)",
      border: "color-mix(in srgb, var(--success) 30%, transparent)",
    };
  }
  if (code === 429) {
    return {
      bg: "var(--warning-soft)",
      fg: "var(--warning)",
      border: "color-mix(in srgb, var(--warning) 30%, transparent)",
    };
  }
  return {
    bg: "var(--danger-soft)",
    fg: "var(--danger)",
    border: "color-mix(in srgb, var(--danger) 30%, transparent)",
  };
}
