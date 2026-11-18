import { useCallback, useEffect, useState } from "react";
import {
  Check,
  ChevronRight,
  Copy,
  FlaskConical,
  Hammer,
  Package,
  Scissors,
  ShieldCheck,
  Sparkles,
  Terminal,
  Trash2,
} from "lucide-react";
import { api } from "../api";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  Card,
  INPUT_CLASS,
  SegmentedControl,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  TokenSaverCategories,
  TokenSaverConfig,
  TokenSaverStats,
  TokenSaverTestResponse,
} from "../types";
import { formatCompactNumber, formatNumber, formatTimeAgo } from "../utils/format";

const DEFAULT_CFG: TokenSaverConfig = {
  rtk_enabled: true,
  rtk_intensity: "standard",
  rtk_max_lines: 250,
  rtk_deduplicate: true,
  rtk_categories: {
    git: true,
    build: true,
    test: true,
    package: true,
    docker: true,
    system: true,
  },
  style_mode: "off",
  style_level: "full",
  custom_prompt: "",
};

const DEFAULT_STATS: TokenSaverStats = {
  total_saved_tokens: 0,
  total_original_tokens: 0,
  compressed_requests: 0,
  style_applied_requests: 0,
  avg_saved_pct: 0,
  by_category: [],
  recent: [],
};

const CATEGORY_CARDS: Array<{
  key: keyof TokenSaverCategories;
  label: string;
  desc: string;
  icon: typeof Terminal;
}> = [
  { key: "git", label: "Git diffs & logs", desc: "Caps huge hunks while keeping diff headers", icon: Scissors },
  { key: "build", label: "Build & compiler", desc: "Collapses Compiling/Downloading lines, keeps errors", icon: Hammer },
  { key: "test", label: "Test runners", desc: "Summarizes passing suites when no failures occur", icon: FlaskConical },
  { key: "package", label: "Package managers", desc: "Strips pip/npm resolution chatter", icon: Package },
  { key: "docker", label: "Docker & K8s", desc: "Collapses repeated image layer pull progress", icon: Terminal },
  { key: "system", label: "System & shell", desc: "Deduplicates and trims long directory listings", icon: Terminal },
];

const STYLE_MODES: Array<{
  id: TokenSaverConfig["style_mode"];
  label: string;
  desc: string;
}> = [
  { id: "off", label: "Off", desc: "Default verbose model formatting" },
  { id: "caveman", label: "Caveman", desc: "Terse engineering facts, zero filler" },
  { id: "ponytail", label: "Ponytail", desc: "Prefer minimal stdlib/native reuse" },
  { id: "adhd", label: "ADHD", desc: "Action-first numbered steps, <=5 bullets" },
  { id: "custom", label: "Custom", desc: "Your own persistent house directive" },
];

const SAMPLE_GIT_DIFF = `diff --git a/src/router.rs b/src/router.rs
index 123456..789abc 100644
--- a/src/router.rs
+++ b/src/router.rs
@@ -1,140 +1,140 @@
` +
  Array.from({ length: 120 }, (_, i) => `+    let route_${i} = dispatch_node(${i}); // routing trace`).join("\n") +
  `\n@@ -200,4 +200,5 @@\n+    return Ok(response);`;

const SAMPLE_CARGO_BUILD =
  Array.from({ length: 55 }, (_, i) => `   Compiling dependency-crate-${i} v0.${i}.1`).join("\n") +
  `\nwarning: unused variable \`retry_after\`\n    Finished \`release\` profile [optimized] target(s) in 14.2s`;

const SAMPLE_TEST_SUITE =
  Array.from({ length: 50 }, (_, i) => `test integration::case_${i}_passes ... ok`).join("\n") +
  `\ntest result: ok. 50 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`;

export function TokenSaverView({
  nowMs,
  onNotify,
}: {
  nowMs: number;
  onNotify: (msg: string, isErr?: boolean) => void;
}) {
  const [config, setConfig] = useState<TokenSaverConfig>(DEFAULT_CFG);
  const [stats, setStats] = useState<TokenSaverStats>(DEFAULT_STATS);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [maxLinesInput, setMaxLinesInput] = useState("250");
  const [customDraft, setCustomDraft] = useState("");
  const [copiedBypass, setCopiedBypass] = useState(false);

  // Compression test bench
  const [benchText, setBenchText] = useState(SAMPLE_CARGO_BUILD);
  const [benchCmd, setBenchCmd] = useState("cargo build");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<TokenSaverTestResponse | null>(null);

  const loadState = useCallback(async () => {
    try {
      const res = await api.getTokenSaver();
      setConfig(res.config);
      setStats(res.stats);
      setMaxLinesInput(String(res.config.rtk_max_lines || 250));
      setCustomDraft(res.config.custom_prompt || "");
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to load Token Saver settings", true);
    } finally {
      setLoading(false);
    }
  }, [onNotify]);

  useEffect(() => {
    void loadState();
  }, [loadState]);

  async function patchConfig(
    patch: Parameters<typeof api.updateTokenSaver>[0],
    toastMsg?: string,
  ) {
    setSaving(true);
    try {
      const res = await api.updateTokenSaver(patch);
      setConfig(res.config);
      setStats(res.stats);
      setMaxLinesInput(String(res.config.rtk_max_lines || 250));
      if (toastMsg) onNotify(toastMsg);
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to save Token Saver settings", true);
    } finally {
      setSaving(false);
    }
  }

  async function handleRunTest(e?: React.FormEvent) {
    e?.preventDefault();
    if (!benchText.trim()) return;
    setTesting(true);
    try {
      const res = await api.testTokenSaver({
        text: benchText,
        tool_name: "shell",
        command: benchCmd.trim() || undefined,
        config,
      });
      setTestResult(res);
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Compression test failed", true);
    } finally {
      setTesting(false);
    }
  }

  async function handleClearStats() {
    if (!window.confirm("Clear stored Token Saver savings statistics?")) return;
    try {
      await api.clearTokenSaverStats();
      await loadState();
      onNotify("Cleared Token Saver analytics");
    } catch (err) {
      onNotify(err instanceof Error ? err.message : "Failed to clear stats", true);
    }
  }

  if (loading) {
    return <TableSkeleton rows={6} />;
  }

  const catStatMap = new Map(stats.by_category.map((c) => [c.category, c]));

  return (
    <div className="space-y-5">
      {/* 4-Card KPI Strip */}
      <div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2 lg:grid-cols-4">
        <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-5 py-4">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Input tokens saved</span>
            <Scissors className="h-4 w-4 text-[var(--success)]" />
          </div>
          <div className="mt-2 font-mono text-2xl font-semibold text-[var(--success)] tabular-nums">
            {formatCompactNumber(stats.total_saved_tokens)}
          </div>
          <div className="mt-1 text-xs text-[var(--text-muted)] tabular-nums">
            From {formatCompactNumber(stats.total_original_tokens)} raw tool tokens
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-5 py-4">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Avg compression ratio</span>
            <Sparkles className="h-4 w-4 text-[var(--accent)]" />
          </div>
          <div className="mt-2 font-mono text-2xl font-semibold text-[var(--text-primary)] tabular-nums">
            {stats.avg_saved_pct > 0 ? `${stats.avg_saved_pct}%` : "—"}
          </div>
          <div className="mt-1 text-xs text-[var(--text-muted)]">
            Deterministic lossless-error trimming
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-5 py-4">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Optimized requests</span>
            {(stats.compressed_requests > 0 || stats.style_applied_requests > 0) && (
              <button
                type="button"
                onClick={() => void handleClearStats()}
                className="inline-flex items-center gap-1 text-xs font-medium text-[var(--text-muted)] hover:text-[var(--danger)]"
                title="Reset Token Saver statistics"
              >
                <Trash2 className="h-3.5 w-3.5" />
                Reset
              </button>
            )}
          </div>
          <div className="mt-2 font-mono text-2xl font-semibold text-[var(--text-primary)] tabular-nums">
            {formatNumber(stats.compressed_requests)}
          </div>
          <div className="mt-1 text-xs text-[var(--text-muted)] tabular-nums">
            {formatNumber(stats.style_applied_requests)} style-guided turns
          </div>
        </div>

        <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-5 py-4">
          <div className="flex items-center justify-between text-xs font-medium text-[var(--text-muted)]">
            <span>Per-request bypass</span>
            <ShieldCheck className="h-4 w-4 text-[var(--token-cache)]" />
          </div>
          <div className="mt-2 flex items-center justify-between gap-2">
            <code className="whitespace-nowrap rounded-md bg-[var(--bg-elevated)] px-2 py-1 font-mono text-[11px] text-[var(--text-primary)]">
              X-Tesie-Token-Saver: off
            </code>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard.writeText("X-Tesie-Token-Saver: off");
                setCopiedBypass(true);
                setTimeout(() => setCopiedBypass(false), 1500);
              }}
              className="shrink-0 rounded-md p-1.5 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
              title="Copy bypass header"
            >
              {copiedBypass ? (
                <Check className="h-4 w-4 text-[var(--success)]" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
            </button>
          </div>
          <div className="mt-1.5 text-xs text-[var(--text-muted)]">
            Send header to skip compression for raw output
          </div>
        </div>
      </div>

      {/* RTK Tool-Output Compression Settings */}
      <Card
        title="RTK Input Compression"
        subtitle="Compresses oversized shell, git, build, test, and package tool outputs before sending upstream"
        actions={
          <div className="flex flex-wrap items-center gap-2.5">
            <Button
              variant={config.rtk_enabled ? "primary" : "secondary"}
              size="sm"
              disabled={saving}
              onClick={() =>
                void patchConfig(
                  { rtk_enabled: !config.rtk_enabled },
                  `RTK compression ${!config.rtk_enabled ? "enabled" : "disabled"}`,
                )
              }
            >
              {config.rtk_enabled ? "RTK Enabled" : "RTK Disabled"}
            </Button>
          </div>
        }
      >
        <div className="space-y-5">
          {/* Top Tuning Controls */}
          <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/55 p-4">
            <div className="flex flex-wrap items-center gap-3">
              <span className="text-[13px] font-medium text-[var(--text-secondary)]">
                Intensity
              </span>
              <SegmentedControl<TokenSaverConfig["rtk_intensity"]>
                size="sm"
                value={config.rtk_intensity}
                onChange={(val) =>
                  void patchConfig({ rtk_intensity: val }, `RTK intensity set to ${val}`)
                }
                options={[
                  { value: "minimal", label: "Minimal" },
                  { value: "standard", label: "Standard" },
                  { value: "aggressive", label: "Aggressive" },
                ]}
              />
            </div>

            <div className="flex flex-wrap items-center gap-4">
              <label className="flex items-center gap-2 text-[13px] text-[var(--text-secondary)]">
                <span>Max lines</span>
                <input
                  type="number"
                  min={40}
                  max={2000}
                  value={maxLinesInput}
                  onChange={(e) => setMaxLinesInput(e.target.value)}
                  onBlur={() => {
                    const n = Math.max(40, Math.min(2000, Number(maxLinesInput) || 250));
                    setMaxLinesInput(String(n));
                    if (n !== config.rtk_max_lines) {
                      void patchConfig({ rtk_max_lines: n }, `Max lines set to ${n}`);
                    }
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      const n = Math.max(40, Math.min(2000, Number(maxLinesInput) || 250));
                      setMaxLinesInput(String(n));
                      if (n !== config.rtk_max_lines) {
                        void patchConfig({ rtk_max_lines: n }, `Max lines set to ${n}`);
                      }
                    }
                  }}
                  className="w-20 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface)] px-2.5 py-1 text-right font-mono text-[13px] text-[var(--text-primary)] tabular-nums focus:border-[var(--accent)] focus:outline-none"
                />
              </label>

              <label className="inline-flex cursor-pointer items-center gap-2 text-[13px] text-[var(--text-secondary)]">
                <input
                  type="checkbox"
                  checked={config.rtk_deduplicate}
                  onChange={(e) =>
                    void patchConfig(
                      { rtk_deduplicate: e.target.checked },
                      `Line deduplication ${e.target.checked ? "enabled" : "disabled"}`,
                    )
                  }
                  className="h-4 w-4 rounded accent-[var(--accent)]"
                />
                <span>Deduplicate repeated lines</span>
              </label>
            </div>
          </div>

          {/* 6 Separate Filter Category Cards */}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {CATEGORY_CARDS.map((cat) => {
              const Icon = cat.icon;
              const enabled = config.rtk_categories[cat.key];
              const stat = catStatMap.get(cat.key);
              return (
                <button
                  key={cat.key}
                  type="button"
                  disabled={saving}
                  onClick={() =>
                    void patchConfig(
                      { rtk_categories: { [cat.key]: !enabled } },
                      `${cat.label} filter ${!enabled ? "enabled" : "disabled"}`,
                    )
                  }
                  className={`flex flex-col justify-between gap-2.5 rounded-xl border p-4 text-left transition-[border-color,background-color] ${
                    enabled
                      ? "border-[color-mix(in_srgb,var(--accent)_45%,var(--border-subtle))] bg-[var(--accent-soft)]/25 hover:border-[var(--accent)]"
                      : "border-[var(--border-subtle)] bg-[var(--bg-surface)] opacity-75 hover:border-[var(--border-strong)]"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="inline-flex items-center gap-2 text-sm font-semibold text-[var(--text-primary)]">
                      <Icon className="h-4 w-4 text-[var(--accent)]" />
                      {cat.label}
                    </span>
                    <StatusPill tone={enabled ? "success" : "neutral"} dot>
                      {enabled ? "Active" : "Off"}
                    </StatusPill>
                  </div>
                  <p className="text-xs text-[var(--text-muted)]">{cat.desc}</p>
                  {stat && (
                    <div className="font-mono text-xs text-[var(--success)] tabular-nums">
                      {formatCompactNumber(stat.saved_tokens)} tokens saved ({stat.requests} reqs)
                    </div>
                  )}
                </button>
              );
            })}
          </div>

          <div className="flex items-start gap-2.5 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 px-4 py-3 text-xs text-[var(--text-secondary)]">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-[var(--success)]" />
            <span>
              <strong>Safety guards always active:</strong> File reads/edits/writes/greps (
              <code className="font-mono">read</code>, <code className="font-mono">edit</code>,{" "}
              <code className="font-mono">grep</code>), valid JSON documents, and error/stacktrace
              outputs are never compressed so agent accuracy stays intact.
            </span>
          </div>
        </div>
      </Card>

      {/* Output Style Savers */}
      <Card
        title="Output Style Saver"
        subtitle="Optionally append a compact response-style instruction to trim verbose assistant filler while keeping code exact"
        actions={
          config.style_mode !== "off" &&
          config.style_mode !== "custom" && (
            <SegmentedControl<TokenSaverConfig["style_level"]>
              size="sm"
              value={config.style_level}
              onChange={(lvl) =>
                void patchConfig({ style_level: lvl }, `Output style level set to ${lvl}`)
              }
              options={
                config.style_mode === "adhd"
                  ? [
                      { value: "lite", label: "Lite" },
                      { value: "full", label: "Full" },
                    ]
                  : [
                      { value: "lite", label: "Lite" },
                      { value: "full", label: "Full" },
                      { value: "ultra", label: "Ultra" },
                    ]
              }
            />
          )
        }
      >
        <div className="space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-5">
            {STYLE_MODES.map((m) => {
              const active = config.style_mode === m.id;
              return (
                <button
                  key={m.id}
                  type="button"
                  disabled={saving}
                  onClick={() =>
                    void patchConfig(
                      { style_mode: m.id },
                      m.id === "off"
                        ? "Output style saver disabled"
                        : `Output style set to ${m.label}`,
                    )
                  }
                  className={`flex flex-col justify-between gap-2 rounded-xl border p-4 text-left transition-[border-color,background-color] ${
                    active
                      ? "border-[var(--accent)] bg-[var(--accent-soft)]/35"
                      : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]"
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-sm font-semibold text-[var(--text-primary)]">
                      {m.label}
                    </span>
                    {active && <span className="h-2 w-2 rounded-full bg-[var(--accent)]" />}
                  </div>
                  <p className="text-xs text-[var(--text-muted)]">{m.desc}</p>
                </button>
              );
            })}
          </div>

          {config.style_mode === "custom" && (
            <div className="flex flex-col gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/50 p-4 sm:flex-row sm:items-end">
              <div className="flex-1">
                <label className="mb-1.5 block text-xs font-medium text-[var(--text-secondary)]">
                  Custom Token Saver directive
                </label>
                <textarea
                  rows={2}
                  placeholder="e.g. Answer in terse technical bullet points. Skip greetings and recaps."
                  value={customDraft}
                  onChange={(e) => setCustomDraft(e.target.value)}
                  className={INPUT_CLASS}
                />
              </div>
              <Button
                variant="primary"
                size="md"
                disabled={saving}
                onClick={() =>
                  void patchConfig(
                    { custom_prompt: customDraft },
                    "Saved custom Token Saver prompt",
                  )
                }
              >
                Save directive
              </Button>
            </div>
          )}
        </div>
      </Card>

      {/* Interactive RTK Compression Test Bench */}
      <Card
        title="Compression Test Bench"
        subtitle="Preview exact RTK token savings on sample tool output before sending traffic"
        actions={
          <div className="flex flex-wrap items-center gap-1.5">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setBenchCmd("git diff");
                setBenchText(SAMPLE_GIT_DIFF);
                setTestResult(null);
              }}
            >
              Git Diff sample
            </Button>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setBenchCmd("cargo build");
                setBenchText(SAMPLE_CARGO_BUILD);
                setTestResult(null);
              }}
            >
              Cargo Build sample
            </Button>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setBenchCmd("cargo test");
                setBenchText(SAMPLE_TEST_SUITE);
                setTestResult(null);
              }}
            >
              Test Suite sample
            </Button>
          </div>
        }
      >
        <form onSubmit={handleRunTest} className="space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-12">
            <div className="sm:col-span-4">
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Simulated command
              </label>
              <input
                type="text"
                placeholder="e.g. cargo build, git diff, pytest"
                value={benchCmd}
                onChange={(e) => setBenchCmd(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
            </div>
            <div className="flex items-end sm:col-span-8 sm:justify-end">
              <Button type="submit" variant="primary" size="md" disabled={testing || !benchText.trim()}>
                <Scissors className="h-4 w-4" />
                {testing ? "Testing…" : "Run compression test"}
              </Button>
            </div>
          </div>

          <textarea
            rows={6}
            value={benchText}
            onChange={(e) => setBenchText(e.target.value)}
            placeholder="Paste raw terminal or tool output here..."
            className={`${INPUT_CLASS} font-mono text-xs`}
          />

          {testResult && (
            <div className="space-y-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 p-4">
              <div className="flex flex-wrap items-center justify-between gap-2 text-sm tabular-nums">
                <div className="flex flex-wrap items-center gap-2">
                  <StatusPill tone={testResult.result.saved_tokens > 0 ? "success" : "neutral"} dot>
                    {testResult.result.saved_tokens > 0
                      ? `-${testResult.result.saved_pct}% (${formatNumber(testResult.result.saved_tokens)} tokens saved)`
                      : "Preserved unmodified"}
                  </StatusPill>
                  <StatusPill tone="accent">
                    category: {testResult.result.detected_category}
                  </StatusPill>
                  <span className="font-mono text-xs text-[var(--text-muted)]">
                    {formatNumber(testResult.result.original_tokens)} &rarr;{" "}
                    {formatNumber(testResult.result.compressed_tokens)} tok
                  </span>
                </div>
                {testResult.result.techniques_used.length > 0 && (
                  <span className="font-mono text-xs text-[var(--text-muted)]">
                    {testResult.result.techniques_used.join(" + ")}
                  </span>
                )}
              </div>

              {testResult.style_prompt_preview && (
                <details className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 text-xs">
                  <summary className="inline-flex cursor-pointer items-center gap-1 font-medium text-[var(--text-secondary)]">
                    <ChevronRight className="h-3.5 w-3.5" />
                    Active style directive preview
                  </summary>
                  <p className="mt-2 font-mono text-xs text-[var(--text-muted)]">
                    {testResult.style_prompt_preview}
                  </p>
                </details>
              )}

              <pre className="max-h-56 overflow-auto rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3 font-mono text-xs text-[var(--text-secondary)]">
                {testResult.result.text}
              </pre>
            </div>
          )}
        </form>
      </Card>

      {/* Recent Compression Activity */}
      {stats.recent.length > 0 && (
        <Card noPadding title="Recent savings log">
          <div className="overflow-x-auto">
            <table className="w-full text-left text-[13px]">
              <thead>
                <tr className="border-b border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 text-xs font-medium text-[var(--text-muted)]">
                  <th className="py-2.5 pl-5 pr-3">Model</th>
                  <th className="py-2.5 px-3">Category</th>
                  <th className="py-2.5 px-3">Style</th>
                  <th className="py-2.5 px-3 text-right">Original &rarr; Compressed</th>
                  <th className="py-2.5 pl-3 pr-5 text-right">Saved</th>
                  <th className="py-2.5 pr-5 text-right">When</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border-subtle)] tabular-nums">
                {stats.recent.map((ev) => (
                  <tr key={ev.id} className="hover:bg-[var(--bg-hover)]/45">
                    <td className="py-2.5 pl-5 pr-3 font-mono text-xs text-[var(--text-primary)]">
                      {ev.model}
                    </td>
                    <td className="py-2.5 px-3 font-mono text-xs text-[var(--text-secondary)]">
                      {ev.category}
                    </td>
                    <td className="py-2.5 px-3 text-xs text-[var(--text-muted)]">
                      {ev.style_mode}
                    </td>
                    <td className="py-2.5 px-3 text-right font-mono text-xs text-[var(--text-secondary)]">
                      {formatNumber(ev.original_tokens)} &rarr; {formatNumber(ev.compressed_tokens)}
                    </td>
                    <td className="py-2.5 pl-3 pr-5 text-right font-mono text-xs font-semibold text-[var(--success)]">
                      +{formatNumber(ev.saved_tokens)}
                    </td>
                    <td className="py-2.5 pr-5 text-right text-xs text-[var(--text-muted)]">
                      {formatTimeAgo(ev.created_at, nowMs)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  );
}
