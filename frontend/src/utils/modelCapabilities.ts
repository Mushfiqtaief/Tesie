import type { OpenAiModelEntry } from "../types";

export interface ModelCapabilities {
  hasVision: boolean;
  hasThinking: boolean;
  hasAudio: boolean;
  hasTools: boolean;
}

export function getModelCapabilities(
  fullSpec: string,
  entry?: OpenAiModelEntry,
): ModelCapabilities {
  const slashIdx = fullSpec.indexOf("/");
  const modelSlug = (slashIdx >= 0 ? fullSpec.slice(slashIdx + 1) : fullSpec).toLowerCase();
  const entryMods = entry?.modalities ?? [];

  // Vision detection: explicit attachment flag, "image" modality, or family regex
  const hasVision = Boolean(
    entry?.attachment === true ||
    entryMods.includes("image") ||
    /gemini|claude-3|claude-sonnet|claude-opus|gpt-4o|gpt-4-turbo|gpt-4.5|o1|o3|-vl|vision|internvl|qwen-vl|qwen2\.5-vl|qwen3-vl|pixtral|llava|glm-4v|deepseek-v4\.1|glm-5\.3|mimo-v2\.[56]|mimo-v2-omni|longcat-2\.5|hy4-flash|inkling-2|super-vl|kimi-k2|qwen3|minimax-m3|step-5|crouton|exo-free|pulse-alpha|muse-spark/i.test(
      modelSlug,
    )
  );

  // Thinking / Reasoning detection: explicit reasoning flag, thinking levels, or family regex
  const hasThinking = Boolean(
    entry?.reasoning === true ||
    (entry?.supported_levels && entry.supported_levels.length > 0) ||
    /thinking|reasoning|reasoner|r1|qwq|o1|o3|beam-free|big-pickle|skywork|deepseek|flash-free|glm-5/i.test(
      modelSlug,
    )
  );

  // Audio detection: explicit "audio" modality or family regex
  const hasAudio = Boolean(
    entryMods.includes("audio") ||
    /gemini|gpt-4o-audio|whisper|muse|mimo/i.test(modelSlug)
  );

  // Tools detection
  const hasTools = entry?.tool_call !== false;

  return { hasVision, hasThinking, hasAudio, hasTools };
}

/**
 * Returns distinct styling classes for model cards:
 * - Half blue on the left for Vision
 * - Half orange/amber on the right for Thinking
 * - Dual split styling if both are supported
 */
export function getModelCardClasses(caps: ModelCapabilities): string {
  if (caps.hasVision && caps.hasThinking) {
    return [
      "border-t border-b border-[var(--border-subtle)] border-l-2 border-l-sky-500 border-r-2 border-r-amber-500",
      "bg-[linear-gradient(90deg,rgba(14,165,233,0.09)_0%,rgba(14,165,233,0.02)_28%,var(--bg-surface)_50%,rgba(245,158,11,0.02)_72%,rgba(245,158,11,0.09)_100%)]",
      "hover:border-t-[var(--border-strong)] hover:border-b-[var(--border-strong)] hover:border-l-sky-400 hover:border-r-amber-400",
      "hover:bg-[linear-gradient(90deg,rgba(14,165,233,0.16)_0%,rgba(14,165,233,0.04)_28%,var(--bg-hover)_50%,rgba(245,158,11,0.04)_72%,rgba(245,158,11,0.16)_100%)]",
      "shadow-[0_2px_8px_-2px_rgba(0,0,0,0.3)] hover:shadow-[0_4px_16px_-2px_rgba(14,165,233,0.12),0_4px_16px_-2px_rgba(245,158,11,0.12)]",
    ].join(" ");
  }

  if (caps.hasVision) {
    return [
      "border-t border-b border-r border-[var(--border-subtle)] border-l-2 border-l-sky-500",
      "bg-[linear-gradient(90deg,rgba(14,165,233,0.09)_0%,rgba(14,165,233,0.02)_40%,var(--bg-surface)_100%)]",
      "hover:border-[var(--border-strong)] hover:border-l-sky-400",
      "hover:bg-[linear-gradient(90deg,rgba(14,165,233,0.16)_0%,rgba(14,165,233,0.04)_40%,var(--bg-hover)_100%)]",
      "shadow-[0_2px_8px_-2px_rgba(0,0,0,0.3)] hover:shadow-[0_4px_16px_-2px_rgba(14,165,233,0.14)]",
    ].join(" ");
  }

  if (caps.hasThinking) {
    return [
      "border-t border-b border-l border-[var(--border-subtle)] border-r-2 border-r-amber-500",
      "bg-[linear-gradient(270deg,rgba(245,158,11,0.09)_0%,rgba(245,158,11,0.02)_40%,var(--bg-surface)_100%)]",
      "hover:border-[var(--border-strong)] hover:border-r-amber-400",
      "hover:bg-[linear-gradient(270deg,rgba(245,158,11,0.16)_0%,rgba(245,158,11,0.04)_40%,var(--bg-hover)_100%)]",
      "shadow-[0_2px_8px_-2px_rgba(0,0,0,0.3)] hover:shadow-[0_4px_16px_-2px_rgba(245,158,11,0.14)]",
    ].join(" ");
  }

  return "border border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)] hover:bg-[var(--bg-hover)]/45";
}

/**
 * Returns distinct mini chip styling for account model pills
 */
export function getModelChipClasses(caps: ModelCapabilities): string {
  if (caps.hasVision && caps.hasThinking) {
    return "border-l-2 border-l-sky-500 border-r-2 border-r-amber-500 border-t border-b border-[var(--border-subtle)] bg-[linear-gradient(90deg,rgba(14,165,233,0.08)_0%,transparent_35%,transparent_65%,rgba(245,158,11,0.08)_100%)]";
  }
  if (caps.hasVision) {
    return "border-l-2 border-l-sky-500 border-t border-b border-r border-[var(--border-subtle)] bg-[linear-gradient(90deg,rgba(14,165,233,0.08)_0%,transparent_45%)]";
  }
  if (caps.hasThinking) {
    return "border-r-2 border-r-amber-500 border-t border-b border-l border-[var(--border-subtle)] bg-[linear-gradient(270deg,rgba(245,158,11,0.08)_0%,transparent_45%)]";
  }
  return "border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/50";
}

export function formatTokenLimit(tokens?: number | null): string {
  if (tokens == null || tokens <= 0) return "";
  if (tokens >= 1_000_000) {
    const m = tokens / 1_000_000;
    return m % 1 === 0 ? `${m}M` : `${m.toFixed(1)}M`;
  }
  if (tokens === 131072) return "128k";
  if (tokens === 65536) return "64k";
  if (tokens === 32768) return "32k";
  if (tokens === 16384) return "16k";
  if (tokens === 8192) return "8k";
  if (tokens === 4096) return "4k";
  if (tokens >= 1_000) {
    const k = Math.round(tokens / 1_000);
    return `${k}k`;
  }
  return String(tokens);
}

export interface ModelLimits {
  context?: number;
  input?: number;
  output?: number;
  badgeLabel: string;
  tooltip: string;
}

export function getModelLimits(
  fullSpec: string,
  entry?: OpenAiModelEntry,
): ModelLimits | null {
  let context = entry?.limit?.context ?? entry?.context_length;
  let input = entry?.limit?.input;
  let output = entry?.limit?.output ?? entry?.max_output_tokens;

  if (context == null && output == null) {
    const slashIdx = fullSpec.indexOf("/");
    const slug = (slashIdx >= 0 ? fullSpec.slice(slashIdx + 1) : fullSpec).toLowerCase();
    if (slug.includes("gemini")) {
      context = 1048576;
      output = 65536;
    } else if (slug.includes("claude") || slug.includes("sonnet") || slug.includes("opus")) {
      context = 200000;
      output = 64000;
    } else if (slug.includes("gpt-5") || slug.includes("gpt-6")) {
      context = 1050000;
      output = 128000;
    } else if (slug.includes("gpt-4o")) {
      context = 128000;
      output = 16384;
    } else if (slug.includes("o1") || slug.includes("o3")) {
      context = 200000;
      output = 100000;
    } else if (slug.includes("1017") || slug.includes("deepseek-v4.1-flash-1017")) {
      context = 1000000;
      output = 384000;
    } else if (slug.includes("deepseek-v4.1-flash")) {
      context = 200000;
      output = 32000;
    } else if (slug.includes("glm-5.3") || slug.includes("mimo-v2.6") || slug.includes("mimo-v2.5")) {
      context = 200000;
      output = 32000;
    } else if (slug.includes("hy4-flash")) {
      context = 1048576;
      input = 983040;
      output = 65536;
    } else if (slug.includes("longcat-2.5")) {
      context = 1000000;
      output = 262144;
    } else if (slug.includes("inkling-2") || slug.includes("beam-free") || slug.includes("muse-spark")) {
      context = 1048576;
      output = 131072;
    } else if (slug.includes("kimi-k2")) {
      context = 262144;
      output = 262144;
    } else if (slug.includes("deepseek-v4-pro") || slug.includes("deepseek-v4")) {
      context = 64000;
      output = 8192;
    } else if (slug.includes("deepseek") || slug.includes("r1")) {
      context = 64000;
      output = 8192;
    } else if (slug.includes("llama-3_3-70b") || slug.includes("llama-3.3-70b")) {
      context = 131072;
      output = 131072;
    } else if (slug.includes("mistral-small")) {
      context = 131072;
      output = 131072;
    } else if (slug.includes("gpt-oss-120b")) {
      context = 131072;
      output = 131072;
    } else if (slug.includes("qwen3.6-27b") || slug.includes("qwen3.6") || slug.includes("qwen3.5-397b") || slug.includes("qwen3.5-9b")) {
      context = 262144;
      output = 65536;
    } else if (slug.includes("qwen2.5-vl") || slug.includes("qwen3-vl")) {
      context = 32768;
      output = 32768;
    } else if (slug.includes("qwen") || slug.includes("llama")) {
      context = 131072;
      output = 8192;
    }
  }

  if (context == null && output == null) {
    return null;
  }

  const parts: string[] = [];
  const tooltipParts: string[] = [];

  if (context != null) {
    parts.push(`ctx ${formatTokenLimit(context)}`);
    tooltipParts.push(`Context: ${context.toLocaleString()} tokens`);
  }
  if (input != null && input !== context) {
    tooltipParts.push(`Max Input: ${input.toLocaleString()} tokens`);
  }
  if (output != null) {
    parts.push(`out ${formatTokenLimit(output)}`);
    tooltipParts.push(`Max Output: ${output.toLocaleString()} tokens`);
  }

  return {
    context,
    input,
    output,
    badgeLabel: parts.join(" · "),
    tooltip: tooltipParts.join(" · "),
  };
}
