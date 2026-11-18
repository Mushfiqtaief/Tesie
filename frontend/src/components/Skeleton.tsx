export function SkeletonBlock({ className = "" }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={`skeleton-shimmer rounded-md border border-[var(--border-subtle)] ${className}`}
    />
  );
}

export function DashboardSkeleton() {
  return (
    <div className="space-y-5" aria-busy="true" aria-label="Loading dashboard telemetry">
      {/* 5 Metric Cards Skeleton */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-5">
        {Array.from({ length: 5 }, (_, i) => (
          <div
            key={i}
            className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-2.5"
          >
            <SkeletonBlock className="h-3.5 w-24" />
            <SkeletonBlock className="h-7 w-28" />
            <SkeletonBlock className="h-3 w-32" />
          </div>
        ))}
      </div>

      {/* Topology + Feed Skeleton */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,2fr)_minmax(300px,1fr)]">
        <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4">
          <SkeletonBlock className="mb-3 h-4 w-44" />
          <SkeletonBlock className="h-[340px] w-full rounded-lg" />
        </div>
        <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-3">
          <SkeletonBlock className="h-4 w-32" />
          {Array.from({ length: 7 }, (_, i) => (
            <SkeletonBlock key={i} className="h-9 w-full" />
          ))}
        </div>
      </div>

      {/* Area Chart Skeleton */}
      <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-3">
        <div className="flex items-center justify-between">
          <SkeletonBlock className="h-4 w-40" />
          <SkeletonBlock className="h-8 w-52 rounded-lg" />
        </div>
        <SkeletonBlock className="h-52 w-full rounded-lg" />
      </div>
    </div>
  );
}

export function QuotaPageSkeleton() {
  return (
    <div className="space-y-5" aria-busy="true" aria-label="Loading provider quotas">
      <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
        {Array.from({ length: 3 }, (_, i) => (
          <div
            key={i}
            className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-2"
          >
            <SkeletonBlock className="h-4 w-28" />
            <SkeletonBlock className="h-6 w-20" />
            <SkeletonBlock className="h-3 w-full" />
          </div>
        ))}
      </div>
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        {Array.from({ length: 4 }, (_, i) => (
          <div
            key={i}
            className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-3"
          >
            <div className="flex justify-between">
              <SkeletonBlock className="h-5 w-36" />
              <SkeletonBlock className="h-6 w-24 rounded-full" />
            </div>
            <SkeletonBlock className="h-3 w-full" />
            <SkeletonBlock className="h-3 w-full" />
          </div>
        ))}
      </div>
    </div>
  );
}

export function TableSkeleton({ rows = 8 }: { rows?: number }) {
  return (
    <div
      className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-2.5"
      aria-busy="true"
    >
      <SkeletonBlock className="h-9 w-full rounded-lg" />
      {Array.from({ length: rows }, (_, i) => (
        <SkeletonBlock key={i} className="h-10 w-full" />
      ))}
    </div>
  );
}
