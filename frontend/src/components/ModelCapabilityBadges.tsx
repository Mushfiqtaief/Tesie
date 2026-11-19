import { Eye, Brain, Volume2 } from "lucide-react";
import type { ModelCapabilities } from "../utils/modelCapabilities";

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
