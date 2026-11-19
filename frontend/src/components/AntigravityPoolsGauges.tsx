import type { AntigravityAccountView, AntigravityPoolWindow } from "../types";
import { formatCountdown } from "../utils/format";

export function splitAgMainPools(acct: AntigravityAccountView): {
  geminiWeekly?: AntigravityPoolWindow;
  gemini5h?: AntigravityPoolWindow;
  claudeWeekly?: AntigravityPoolWindow;
  claude5h?: AntigravityPoolWindow;
} {
  const unlockedPools = (acct.quota?.pools ?? []).filter((p) => p.unlocked !== false);
  const pick = (groupNeedles: string[], isWeekly: boolean) =>
    unlockedPools.find((p) => {
      const gn = `${p.group_name} ${p.key}`.toLowerCase();
      const win = `${p.window_label} ${p.key}`.toLowerCase();
      const groupMatch = groupNeedles.some((n) => gn.includes(n));
      const winMatch = isWeekly
        ? win.includes("week")
        : win.includes("5h") || win.includes("session");
      return groupMatch && winMatch;
    });

  return {
    geminiWeekly: pick(["gemini"], true),
    gemini5h: pick(["gemini"], false),
    claudeWeekly: pick(["claude", "gpt"], true),
    claude5h: pick(["claude", "gpt"], false),
  };
}

interface AntigravityPoolsGaugesProps {
  acct: AntigravityAccountView;
  isBanned: boolean;
  nowMs: number;
}

export function AntigravityPoolsGauges({
  acct,
  isBanned,
  nowMs,
}: AntigravityPoolsGaugesProps) {
  const { geminiWeekly, gemini5h, claudeWeekly, claude5h } = splitAgMainPools(acct);
  const hasGemini = Boolean(geminiWeekly || gemini5h);
  const hasClaude = Boolean(claudeWeekly || claude5h);

  if (!hasGemini && !hasClaude) return null;

  const renderGauge = (
    pool: AntigravityPoolWindow | undefined,
    label: string,
    dotColor: string,
    barColor: string,
  ) => {
    if (!pool) return null;
    const rawPct = Math.round(pool.remaining_percentage);
    const isEnabled = acct.enabled;
    const pct = isBanned || !isEnabled ? 0 : Math.max(0, Math.min(100, rawPct));
    const hasReset = pool.reset_at && pool.reset_at > nowMs;
    const isDepleted = pct <= 0;

    let activeBarColor = barColor;
    if (isBanned) {
      activeBarColor = "bg-rose-500/40";
    } else if (!isEnabled) {
      activeBarColor = "bg-zinc-600";
    } else if (pct <= 20) {
      activeBarColor = "bg-rose-500";
    } else if (pct <= 50) {
      activeBarColor = "bg-amber-400";
    }

    const resetCountdown = hasReset ? formatCountdown(pool.reset_at, nowMs) : null;
    const tooltip = isBanned
      ? `${label}: Account Banned`
      : !isEnabled
        ? `${label}: Account Disabled`
        : hasReset
          ? `${label}: ${pct}% · Resets in ${resetCountdown}`
          : `${label}: ${pct}% remaining`;

    return (
      <div
        className="flex items-center justify-between gap-2.5 px-1 min-w-0"
        title={tooltip}
      >
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <span
            className={`h-2 w-2 rounded-full shrink-0 ${
              isBanned ? "bg-rose-400" : !isEnabled ? "bg-zinc-500" : dotColor
            }`}
          />
          <span className="text-xs font-medium text-[var(--text-secondary)] truncate">
            {label}
          </span>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <div className="w-16 h-1.5 bg-[var(--bg-base)] rounded-full overflow-hidden">
            <div
              className={`h-full rounded-full transition-all duration-300 ${activeBarColor}`}
              style={{ width: `${pct}%` }}
            />
          </div>
          <div className="flex items-center gap-1 font-mono text-xs font-semibold tabular-nums text-right min-w-[36px] justify-end">
            {isBanned ? (
              <span className="text-rose-400">0%</span>
            ) : !isEnabled ? (
              <span className="text-[var(--text-muted)]">0%</span>
            ) : isDepleted && hasReset ? (
              <span className="text-rose-400 flex items-center gap-1">
                <span>0%</span>
                <span className="text-[10px] text-amber-400 font-sans font-normal">
                  ({resetCountdown})
                </span>
              </span>
            ) : (
              <span
                className={
                  pct <= 20
                    ? "text-rose-400"
                    : pct <= 50
                      ? "text-amber-400"
                      : "text-[var(--text-primary)]"
                }
              >
                {pct}%
              </span>
            )}
          </div>
        </div>
      </div>
    );
  };

  const gridCols = hasGemini && hasClaude ? "grid-cols-1 sm:grid-cols-2" : "grid-cols-1";

  return (
    <div
      className={`grid ${gridCols} gap-2.5 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 p-2.5`}
    >
      {/* Gemini Pool Group */}
      {hasGemini && (
        <div className="flex flex-col gap-1.5 min-w-0">
          {geminiWeekly &&
            renderGauge(geminiWeekly, "Gemini Weekly Pool", "bg-blue-400", "bg-blue-400")}
          {gemini5h &&
            renderGauge(gemini5h, "Gemini 5h Limit", "bg-sky-400", "bg-sky-400")}
        </div>
      )}

      {/* Claude / GPT Pool Group */}
      {hasClaude && (
        <div className="flex flex-col gap-1.5 min-w-0">
          {claudeWeekly &&
            renderGauge(claudeWeekly, "Claude / GPT Pool", "bg-purple-400", "bg-purple-400")}
          {claude5h &&
            renderGauge(claude5h, "Claude / GPT 5h Limit", "bg-indigo-400", "bg-indigo-400")}
        </div>
      )}
    </div>
  );
}
