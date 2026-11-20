import { useEffect, useState } from "react";

/**
 * Visibility-aware clock hook so countdown/relative-time components only re-render
 * themselves on the cadence they actually need, instead of waking up the entire app.
 */
export function useLiveClock(intervalMs: number, enabled = true): number {
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (!enabled || intervalMs <= 0) return;

    let timerId: number | null = null;

    const start = () => {
      if (timerId !== null || document.hidden) return;
      timerId = window.setInterval(() => {
        setNowMs(Date.now());
      }, intervalMs);
    };

    const stop = () => {
      if (timerId !== null) {
        window.clearInterval(timerId);
        timerId = null;
      }
    };

    const handleVisibility = () => {
      if (document.hidden) {
        stop();
      } else {
        setNowMs(Date.now());
        start();
      }
    };

    if (!document.hidden) {
      start();
    }
    document.addEventListener("visibilitychange", handleVisibility);

    return () => {
      stop();
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, [intervalMs, enabled]);

  return nowMs;
}
