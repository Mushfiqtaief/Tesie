import { useEffect, type ButtonHTMLAttributes, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { formatCompactNumber, formatCountdown } from "../utils/format";

let activeBodyScrollLocks = 0;
let previousBodyOverflow = "";
let previousHtmlOverflow = "";

function useBodyScrollLockAndEscape(open: boolean, onClose: () => void) {
  useEffect(() => {
    if (!open || typeof document === "undefined") return;

    if (activeBodyScrollLocks === 0) {
      previousBodyOverflow = document.body.style.overflow;
      previousHtmlOverflow = document.documentElement.style.overflow;
      document.body.style.overflow = "hidden";
      document.documentElement.style.overflow = "hidden";
    }
    activeBodyScrollLocks += 1;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };

    const handleWheel = (e: WheelEvent) => {
      const target = e.target instanceof Element ? e.target : null;
      const scrollBox = target?.closest<HTMLElement>('[data-tesie-scroll-allow="true"]');
      if (!scrollBox) {
        e.preventDefault();
        return;
      }
      const { scrollTop, scrollHeight, clientHeight } = scrollBox;
      const canScrollUp = scrollTop > 0;
      const canScrollDown = scrollTop + clientHeight < scrollHeight - 1;
      if (
        scrollHeight <= clientHeight ||
        (e.deltaY < 0 && !canScrollUp) ||
        (e.deltaY > 0 && !canScrollDown)
      ) {
        e.preventDefault();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("wheel", handleWheel, { passive: false });

    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("wheel", handleWheel);
      activeBodyScrollLocks = Math.max(0, activeBodyScrollLocks - 1);
      if (activeBodyScrollLocks === 0) {
        document.body.style.overflow = previousBodyOverflow;
        document.documentElement.style.overflow = previousHtmlOverflow;
      }
    };
  }, [open, onClose]);
}

export function Card({
  title,
  subtitle,
  actions,
  children,
  className = "",
  bodyClassName = "",
  noPadding = false,
}: {
  title?: string;
  subtitle?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  noPadding?: boolean;
}) {
  return (
    <section
      className={`rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] ${className}`}
    >
      {(title || actions) && (
        <header className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] px-5 py-4">
          <div className="min-w-0">
            {title && (
              <h2 className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">
                {title}
              </h2>
            )}
            {subtitle && (
              <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">{subtitle}</p>
            )}
          </div>
          {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
        </header>
      )}
      <div className={`${noPadding ? "" : "p-5 sm:p-6"} ${bodyClassName}`.trim()}>{children}</div>
    </section>
  );
}

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
type ButtonSize = "sm" | "md";

export function Button({
  variant = "secondary",
  size = "sm",
  className = "",
  children,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
}) {
  const base =
    "inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-[transform,background-color,border-color,color,opacity] duration-150 [transition-timing-function:var(--ease-out)] active:scale-[0.97] disabled:opacity-45";
  const sizes: Record<ButtonSize, string> = {
    sm: "px-3 py-1.5 text-[13px]",
    md: "px-4 py-2 text-sm",
  };
  const variants: Record<ButtonVariant, string> = {
    primary:
      "bg-[var(--accent)] text-[var(--accent-fg)] hover:bg-[var(--accent-hover)] border border-transparent",
    secondary:
      "border border-[var(--border-strong)] bg-[var(--bg-elevated)] text-[var(--text-primary)] hover:bg-[var(--bg-hover)]",
    ghost:
      "border border-transparent text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]",
    danger:
      "border border-[color-mix(in_srgb,var(--danger)_30%,transparent)] bg-[var(--danger-soft)] text-[var(--danger)] hover:brightness-110",
  };

  return (
    <button
      type={props.type ?? "button"}
      className={`${base} ${sizes[size]} ${variants[variant]} ${className}`}
      {...props}
    >
      {children}
    </button>
  );
}

export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
  size = "md",
  className = "",
}: {
  options: Array<{ value: T; label: string; icon?: ReactNode }>;
  value: T;
  onChange: (value: T) => void;
  size?: "sm" | "md";
  className?: string;
}) {
  const pad = size === "sm" ? "px-3 py-1.5 text-[13px]" : "px-3.5 py-2 text-sm";
  return (
    <div
      role="tablist"
      className={`inline-flex items-center gap-0.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5 ${className}`}
    >
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(opt.value)}
            className={`inline-flex items-center gap-1.5 ${pad} rounded-md font-medium whitespace-nowrap transition-[background-color,border-color,color,transform] duration-150 [transition-timing-function:var(--ease-out)] active:scale-[0.97] ${
              active
                ? "bg-[var(--bg-surface)] text-[var(--text-primary)] border border-[var(--border-subtle)] shadow-2xs"
                : "text-[var(--text-muted)] hover:text-[var(--text-primary)] border border-transparent"
            }`}
          >
            {opt.icon}
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

export function StatusPill({
  tone,
  dot = false,
  pulse = false,
  children,
}: {
  tone: "success" | "warning" | "danger" | "accent" | "neutral";
  dot?: boolean;
  pulse?: boolean;
  children: ReactNode;
}) {
  const styles: Record<string, string> = {
    success:
      "bg-[var(--success-soft)] text-[var(--success)] border-[color-mix(in_srgb,var(--success)_28%,transparent)]",
    warning:
      "bg-[var(--warning-soft)] text-[var(--warning)] border-[color-mix(in_srgb,var(--warning)_28%,transparent)]",
    danger:
      "bg-[var(--danger-soft)] text-[var(--danger)] border-[color-mix(in_srgb,var(--danger)_28%,transparent)]",
    accent:
      "bg-[var(--accent-soft)] text-[var(--accent)] border-[color-mix(in_srgb,var(--accent)_28%,transparent)]",
    neutral:
      "bg-[var(--bg-elevated)] text-[var(--text-secondary)] border-[var(--border-subtle)]",
  };
  const dotColors: Record<string, string> = {
    success: "bg-[var(--success)]",
    warning: "bg-[var(--warning)]",
    danger: "bg-[var(--danger)]",
    accent: "bg-[var(--accent)]",
    neutral: "bg-[var(--text-muted)]",
  };

  return (
    <span
      className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-full border px-2.5 py-0.5 text-xs font-medium tabular-nums ${styles[tone]}`}
    >
      {dot && (
        <span
          className={`h-1.5 w-1.5 rounded-full ${dotColors[tone]} ${pulse ? "animate-pulse" : ""}`}
        />
      )}
      {children}
    </span>
  );
}

export function QuotaStatusBadge({
  status,
  cooldownUntil,
  nowMs,
}: {
  status: "active" | "cooldown" | null | undefined;
  cooldownUntil: number | null | undefined;
  nowMs: number;
}) {
  const cooling = status === "cooldown" && !!cooldownUntil && cooldownUntil > nowMs;
  if (cooling) {
    return (
      <StatusPill tone="warning" dot pulse>
        Cooldown ({formatCountdown(cooldownUntil, nowMs)})
      </StatusPill>
    );
  }
  return (
    <StatusPill tone="success" dot>
      Active
    </StatusPill>
  );
}

export function MeterBar({
  label,
  used,
  limit,
  tone = "accent",
}: {
  label: string;
  used: number;
  limit: number | null | undefined;
  tone?: "accent" | "success" | "cache" | "warning";
}) {
  const hasCap = limit !== null && limit !== undefined && limit > 0;
  const pct = hasCap ? Math.min(100, Math.round((used / limit) * 100)) : 0;

  let fillVar = "var(--accent)";
  if (pct >= 90) fillVar = "var(--danger)";
  else if (pct >= 75) fillVar = "var(--warning)";
  else if (tone === "success") fillVar = "var(--success)";
  else if (tone === "cache") fillVar = "var(--token-cache)";
  else if (tone === "warning") fillVar = "var(--warning)";

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-[11px] tabular-nums">
        <span className="text-[var(--text-muted)]">{label}</span>
        <span className="font-mono font-medium text-[var(--text-secondary)]">
          {formatCompactNumber(used)}
          {hasCap ? ` / ${formatCompactNumber(limit)}` : " / unlimited"}
        </span>
      </div>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--bg-elevated)] border border-[var(--border-subtle)]">
        <div
          className="h-full rounded-full transition-[width] duration-300"
          style={{
            backgroundColor: fillVar,
            width: hasCap ? `${pct}%` : used > 0 ? "18%" : "0%",
            opacity: hasCap ? 1 : 0.45,
          }}
        />
      </div>
    </div>
  );
}

export function Modal({
  open,
  title,
  subtitle,
  onClose,
  children,
  maxWidth = "max-w-lg",
}: {
  open: boolean;
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
  maxWidth?: string;
}) {
  useBodyScrollLockAndEscape(open, onClose);
  if (!open || typeof document === "undefined") return null;

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/55 p-4 backdrop-blur-[2px]"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        className={`animate-modal-in flex max-h-[calc(100vh-2rem)] w-full flex-col ${maxWidth} rounded-xl border border-[var(--border-strong)] bg-[var(--bg-surface)] shadow-[var(--shadow-float)]`}
      >
        <div className="flex shrink-0 items-start justify-between gap-3 border-b border-[var(--border-subtle)] px-5 py-3.5">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">{title}</h3>
            {subtitle && (
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">{subtitle}</p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close modal"
            className="rounded-md p-1 text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div data-tesie-scroll-allow="true" className="min-h-0 overflow-y-auto overscroll-contain p-5">
          {children}
        </div>
      </div>
    </div>,
    document.body,
  );
}

export function SlideOverDrawer({
  open,
  title,
  subtitle,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  subtitle?: string;
  onClose: () => void;
  children: ReactNode;
}) {
  useBodyScrollLockAndEscape(open, onClose);
  if (!open || typeof document === "undefined") return null;

  return createPortal(
    <div
      className="fixed inset-0 z-[100] flex justify-end bg-black/45 backdrop-blur-[1px]"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <aside
        role="dialog"
        aria-modal="true"
        className="animate-drawer-in flex h-full w-full max-w-xl flex-col border-l border-[var(--border-strong)] bg-[var(--bg-surface)] shadow-[var(--shadow-float)]"
      >
        <header className="flex shrink-0 items-start justify-between gap-3 border-b border-[var(--border-subtle)] px-5 py-4">
          <div className="min-w-0">
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">{title}</h3>
            {subtitle && (
              <p className="mt-0.5 truncate font-mono text-xs text-[var(--text-muted)]">
                {subtitle}
              </p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close drawer"
            className="rounded-md p-1 text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        <div data-tesie-scroll-allow="true" className="flex-1 overflow-y-auto overscroll-contain p-5">
          {children}
        </div>
      </aside>
    </div>,
    document.body,
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon?: ReactNode;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-[var(--border-strong)] bg-[var(--bg-elevated)] px-6 py-10 text-center">
      {icon && <div className="mb-2.5 text-[var(--text-muted)]">{icon}</div>}
      <p className="text-sm font-medium text-[var(--text-primary)]">{title}</p>
      <p className="mt-1 max-w-md text-xs text-[var(--text-muted)]">{description}</p>
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

export const INPUT_CLASS =
  "w-full rounded-lg border border-[var(--border-strong)] bg-[var(--bg-elevated)] px-3.5 py-2 text-[13px] text-[var(--text-primary)] placeholder:text-[var(--text-muted)] focus:border-[var(--accent)] focus:outline-none";
