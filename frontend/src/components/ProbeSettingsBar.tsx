import { useEffect, useRef, useState } from "react";
import { Check, Clock, Loader2, RefreshCw } from "lucide-react";
import { Button } from "./UiPrimitives";
import { formatTimeAgo } from "../utils/format";

const MIN_PROBE_SEC = 5;
const MAX_PROBE_SEC = 86_400;

export function ProbeSettingsBar({
  providerId,
  providerName,
  probeEnabled,
  probeIntervalSec,
  onUpdateConfig,
  onManualProbe,
  probing = false,
  lastProbeAt = null,
  nowMs = Date.now(),
  compact = false,
}: {
  providerId: string;
  providerName: string;
  probeEnabled: boolean;
  probeIntervalSec: number;
  onUpdateConfig: (payload: {
    probe_enabled?: boolean;
    probe_interval_sec?: number;
  }) => Promise<void>;
  onManualProbe?: () => void;
  probing?: boolean;
  lastProbeAt?: number | null;
  nowMs?: number;
  compact?: boolean;
}) {
  const safePropInterval = Math.max(MIN_PROBE_SEC, Math.round(probeIntervalSec || 30));
  const [intervalText, setIntervalText] = useState(String(safePropInterval));
  const [validationMsg, setValidationMsg] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [toggling, setToggling] = useState(false);

  const lastCommittedRef = useRef<number>(safePropInterval);
  const debounceRef = useRef<number | null>(null);
  const savedBadgeTimerRef = useRef<number | null>(null);
  const onUpdateRef = useRef(onUpdateConfig);
  onUpdateRef.current = onUpdateConfig;

  useEffect(() => {
    lastCommittedRef.current = safePropInterval;
    setIntervalText(String(safePropInterval));
    setValidationMsg(null);
  }, [providerId, safePropInterval]);

  useEffect(() => {
    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
      if (savedBadgeTimerRef.current) window.clearTimeout(savedBadgeTimerRef.current);
    };
  }, []);

  function handleIntervalInputChange(raw: string) {
    // Strictly digits only — strip non-numeric characters immediately
    const digitsOnly = raw.replace(/\D/g, "");
    setIntervalText(digitsOnly);

    if (debounceRef.current) {
      window.clearTimeout(debounceRef.current);
      debounceRef.current = null;
    }

    if (!digitsOnly) {
      setValidationMsg(`${MIN_PROBE_SEC}–${MAX_PROBE_SEC}s`);
      return;
    }

    const parsed = Number(digitsOnly);
    if (!Number.isFinite(parsed) || parsed < MIN_PROBE_SEC) {
      setValidationMsg(`Min ${MIN_PROBE_SEC}s`);
      return;
    }
    if (parsed > MAX_PROBE_SEC) {
      setValidationMsg(`Max ${MAX_PROBE_SEC}s`);
      return;
    }

    setValidationMsg(null);
    if (parsed === lastCommittedRef.current) {
      return;
    }

    debounceRef.current = window.setTimeout(async () => {
      setSaveState("saving");
      try {
        await onUpdateRef.current({ probe_interval_sec: parsed });
        lastCommittedRef.current = parsed;
        setSaveState("saved");
        if (savedBadgeTimerRef.current) window.clearTimeout(savedBadgeTimerRef.current);
        savedBadgeTimerRef.current = window.setTimeout(() => {
          setSaveState("idle");
        }, 2000);
      } catch {
        setSaveState("idle");
      }
    }, 450);
  }

  async function handleToggleProbe() {
    setToggling(true);
    try {
      await onUpdateConfig({ probe_enabled: !probeEnabled });
    } finally {
      setToggling(false);
    }
  }

  const isAntigravity = providerId === "antigravity";
  const helpTooltip = isAntigravity
    ? "Background sync from Antigravity's quota endpoint. Exhausted nodes cool down for 30m."
    : "Background recovery check for expired exit IPs (0 idle token waste). Exhausted IPs cool down for 30m.";

  const controls = (
    <div className="flex flex-wrap items-center gap-2">
      {/* Unified Probe Toggle + Auto-Saving Interval Cluster */}
      <div
        className="inline-flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1.5 text-[13px]"
        title={helpTooltip}
      >
        <button
          type="button"
          disabled={toggling}
          onClick={() => void handleToggleProbe()}
          aria-pressed={probeEnabled}
          className="inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)] disabled:opacity-45"
          title={probeEnabled ? "Disable background quota probes" : "Enable background quota probes"}
        >
          <span
            className={`h-2 w-2 rounded-full ${
              probeEnabled
                ? probing
                  ? "bg-[var(--success)] animate-pulse"
                  : "bg-[var(--success)]"
                : "bg-[var(--text-muted)]"
            }`}
          />
          <span>{probeEnabled ? "Probe on" : "Probe off"}</span>
        </button>

        <span className="h-4 w-px bg-[var(--border-subtle)]" aria-hidden="true" />

        <label
          htmlFor={`probe-interval-${providerId}`}
          className="inline-flex items-center gap-1 text-xs text-[var(--text-muted)]"
        >
          <Clock className="h-3.5 w-3.5" />
          <span className="sr-only">{providerName} probe interval in seconds</span>
        </label>

        <div className="relative flex items-center">
          <input
            id={`probe-interval-${providerId}`}
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            disabled={!probeEnabled}
            aria-label={`${providerName} probe interval in seconds`}
            value={intervalText}
            onChange={(e) => handleIntervalInputChange(e.target.value)}
            onBlur={() => {
              if (!intervalText || Number(intervalText) < MIN_PROBE_SEC) {
                setIntervalText(String(lastCommittedRef.current));
                setValidationMsg(null);
              }
            }}
            className="w-14 rounded border border-[var(--border-subtle)] bg-[var(--bg-surface)] py-0.5 pr-5 pl-1.5 text-right font-mono text-xs text-[var(--text-primary)] tabular-nums focus:border-[var(--accent)] focus:outline-none disabled:opacity-45"
          />
          <span className="pointer-events-none absolute right-2 font-mono text-xs text-[var(--text-muted)]">
            s
          </span>
        </div>

        {validationMsg ? (
          <span className="font-mono text-xs text-[var(--warning)]">{validationMsg}</span>
        ) : saveState === "saving" ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-[var(--text-muted)]" aria-label="Saving" />
        ) : saveState === "saved" ? (
          <span
            className="inline-flex items-center gap-0.5 font-mono text-xs text-[var(--success)]"
            aria-live="polite"
          >
            <Check className="h-3.5 w-3.5" />
            Saved
          </span>
        ) : lastProbeAt ? (
          <span className="hidden font-mono text-xs text-[var(--text-muted)] sm:inline">
            · {formatTimeAgo(lastProbeAt, nowMs)}
          </span>
        ) : null}
      </div>

      {onManualProbe && (
        <Button
          variant="secondary"
          size="sm"
          disabled={probing}
          onClick={onManualProbe}
          title="Run immediate quota probe now"
        >
          <RefreshCw className={`h-3.5 w-3.5 ${probing ? "animate-spin" : ""}`} />
          {probing ? "Probing…" : "Probe now"}
        </Button>
      )}
    </div>
  );

  if (compact) {
    return controls;
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-2.5">
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <span className="font-medium text-[var(--text-primary)]">{providerName} Probe</span>
        <span className="text-[11px] text-[var(--text-muted)]">
          Auto-saves interval · 30m exhaustion cooldown
        </span>
      </div>
      {controls}
    </div>
  );
}
