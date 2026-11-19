import { Eye, Brain, Volume2, Layers } from "lucide-react";
import type { ModelCapabilities, ModelLimits } from "../utils/modelCapabilities";
import { formatTokenLimit } from "../utils/modelCapabilities";

export function ModelLimitBadge({
  limits,
  compact = false,
}: {
  limits: ModelLimits | null | undefined;
  compact?: boolean;
}) {
  if (!limits || !limits.badgeLabel) {
    return null;
  }

  if (compact) {
    return (
      <span
        title={limits.tooltip}
        className="inline-flex items-center gap-1 font-mono text-[10px] text-[var(--text-secondary)] bg-[var(--bg-elevated)]/90 px-1.5 py-0.5 border border-[var(--border-subtle)] rounded cursor-help shadow-2xs"
      >
        <Layers className="h-2.5 w-2.5 text-[var(--text-muted)] shrink-0" />
        <span>{limits.badgeLabel}</span>
      </span>
    );
  }

  return (
    <span
      title={limits.tooltip}
      className="inline-flex items-center gap-1.5 rounded-md bg-[var(--bg-elevated)]/90 px-2 py-0.5 font-mono text-[11px] font-medium text-[var(--text-secondary)] border border-[var(--border-subtle)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] transition-colors cursor-help shadow-2xs"
    >
      <Layers className="h-3 w-3 text-[var(--text-muted)] shrink-0" />
      <span>
        {limits.context != null && (
          <span>
            <span className="text-[var(--text-muted)] text-[10px] font-sans font-normal uppercase tracking-wider">ctx</span>{" "}
            <strong className="text-[var(--text-primary)] font-semibold">
              {formatTokenLimit(limits.context)}
            </strong>
          </span>
        )}
        {limits.context != null && limits.output != null && (
          <span className="mx-1 text-[var(--text-muted)]/60 font-light">·</span>
        )}
        {limits.output != null && (
          <span>
            <span className="text-[var(--text-muted)] text-[10px] font-sans font-normal uppercase tracking-wider">out</span>{" "}
            <strong className="text-[var(--text-primary)] font-semibold">
              {formatTokenLimit(limits.output)}
            </strong>
          </span>
        )}
      </span>
    </span>
  );
}

/**
 * Top-right corner icon-only capability stamps with glowing accents & tooltips
 */
export function ModelCapabilityIcons({
  caps,
  className = "",
}: {
  caps: ModelCapabilities;
  className?: string;
}) {
  if (!caps.hasVision && !caps.hasThinking && !caps.hasAudio) {
    return null;
  }

  return (
    <div className={`flex items-center gap-1 shrink-0 ${className}`}>
      {caps.hasVision && (
        <span
          title="Vision: Supports image & screenshot analysis"
          className="inline-flex items-center justify-center h-5 w-5 rounded bg-sky-500/10 border border-sky-500/25 text-sky-400 hover:bg-sky-500/20 hover:border-sky-400 transition-colors shadow-2xs cursor-help"
        >
          <Eye className="h-3 w-3" />
        </span>
      )}
      {caps.hasThinking && (
        <span
          title="Thinking: Chain-of-thought / extended reasoning"
          className="inline-flex items-center justify-center h-5 w-5 rounded bg-amber-500/10 border border-amber-500/25 text-amber-400 hover:bg-amber-500/20 hover:border-amber-400 transition-colors shadow-2xs cursor-help"
        >
          <Brain className="h-3 w-3" />
        </span>
      )}
      {caps.hasAudio && (
        <span
          title="Audio: Supports voice & audio processing"
          className="inline-flex items-center justify-center h-5 w-5 rounded bg-emerald-500/10 border border-emerald-500/25 text-emerald-400 hover:bg-emerald-500/20 hover:border-emerald-400 transition-colors shadow-2xs cursor-help"
        >
          <Volume2 className="h-3 w-3" />
        </span>
      )}
    </div>
  );
}

export function ModelCapabilityBadges({
  caps,
  compact = false,
}: {
  caps: ModelCapabilities;
  compact?: boolean;
}) {
  if (!caps.hasVision && !caps.hasThinking && !caps.hasAudio) {
    return null;
  }

  if (compact) {
    return (
      <div className="flex items-center gap-1 shrink-0">
        {caps.hasVision && (
          <span title="Vision: Supports image & screenshot analysis">
            <Eye className="h-3 w-3 text-sky-400" />
          </span>
        )}
        {caps.hasThinking && (
          <span title="Thinking: Chain-of-thought / extended reasoning">
            <Brain className="h-3 w-3 text-amber-400" />
          </span>
        )}
        {caps.hasAudio && (
          <span title="Audio: Supports voice & audio input">
            <Volume2 className="h-3 w-3 text-emerald-400" />
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-wrap items-center gap-1 shrink-0">
      {caps.hasVision && (
        <span
          className="inline-flex items-center gap-1 rounded bg-sky-500/15 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-sky-400 border border-sky-500/30 shadow-xs backdrop-blur-xs"
          title="Vision: Supports image, photo, & screenshot analysis"
        >
          <Eye className="h-2.5 w-2.5" />
          <span>Vision</span>
        </span>
      )}
      {caps.hasThinking && (
        <span
          className="inline-flex items-center gap-1 rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-amber-400 border border-amber-500/30 shadow-xs backdrop-blur-xs"
          title="Thinking: Chain-of-thought / extended reasoning"
        >
          <Brain className="h-2.5 w-2.5" />
          <span>Thinking</span>
        </span>
      )}
      {caps.hasAudio && (
        <span
          className="inline-flex items-center gap-0.5 rounded bg-emerald-500/15 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-emerald-400 border border-emerald-500/30 shadow-xs backdrop-blur-xs"
          title="Audio: Supports voice & audio input"
        >
          <Volume2 className="h-2.5 w-2.5" />
          <span>Audio</span>
        </span>
      )}
    </div>
  );
}
