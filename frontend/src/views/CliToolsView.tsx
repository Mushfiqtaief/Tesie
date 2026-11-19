import { useEffect, useMemo, useState } from "react";
import {
  Check,
  Code2,
  Copy,
  FileCode2,
  KeyRound,
  Plus,
  RotateCcw,
  Search,
  Sparkles,
  Terminal,
  X,
} from "lucide-react";
import { ProviderIcon } from "../components/ProviderIcon";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  INPUT_CLASS,
  Modal,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  ApiKeyView,
  ApplyClaudeCliPayload,
  ApplyCodexCliPayload,
  ApplyOpenCodeCliPayload,
  CliToolsStatusMap,
  ComboRecord,
  CreateApiKeyPayload,
  CreatedApiKeyResponse,
  OpenAiModelEntry,
} from "../types";

type ToolTab = "opencode" | "claude" | "codex" | "ides";

export function CliToolsView({
  cliStatus,
  models,
  combos,
  keys,
  loading,
  onApplyOpenCode,
  onResetOpenCode,
  onApplyClaude,
  onResetClaude,
  onApplyCodex,
  onResetCodex,
  onCreateKey,
}: {
  cliStatus: CliToolsStatusMap | null;
  models: OpenAiModelEntry[];
  combos: ComboRecord[];
  keys: ApiKeyView[];
  loading: boolean;
  onApplyOpenCode: (payload: ApplyOpenCodeCliPayload) => Promise<void>;
  onResetOpenCode: () => Promise<void>;
  onApplyClaude: (payload: ApplyClaudeCliPayload) => Promise<void>;
  onResetClaude: () => Promise<void>;
  onApplyCodex: (payload: ApplyCodexCliPayload) => Promise<void>;
  onResetCodex: () => Promise<void>;
  onCreateKey: (payload: CreateApiKeyPayload) => Promise<CreatedApiKeyResponse>;
}) {
  const origin =
    typeof window !== "undefined" ? window.location.origin : "http://127.0.0.1:20129";
  const defaultBaseV1 = `${origin}/v1`;

  const [activeTab, setActiveTab] = useState<ToolTab>("opencode");
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [previewModal, setPreviewModal] = useState<{
    title: string;
    path: string;
    content: string;
  } | null>(null);

  // Shared endpoint & key selection
  const [baseUrl, setBaseUrl] = useState(defaultBaseV1);
  const [selectedApiKey, setSelectedApiKey] = useState("");
  const [creatingKey, setCreatingKey] = useState(false);

  // OpenCode form state
  const [ocModels, setOcModels] = useState<string[]>([]);
  const [ocActiveModel, setOcActiveModel] = useState("");
  const [ocSubagentModel, setOcSubagentModel] = useState("");
  const [ocBusy, setOcBusy] = useState(false);
  const [ocFilter, setOcFilter] = useState("");

  // Claude Code form state
  const [clDefault, setClDefault] = useState("");
  const [clOpus, setClOpus] = useState("");
  const [clSonnet, setClSonnet] = useState("");
  const [clHaiku, setClHaiku] = useState("");
  const [clSubagent, setClSubagent] = useState("");
  const [clBusy, setClBusy] = useState(false);

  // Codex form state
  const [cxModel, setCxModel] = useState("");
  const [cxSubagent, setCxSubagent] = useState("");
  const [cxBusy, setCxBusy] = useState(false);

  // Build unified model options (Combos first, then provider models)
  const comboNames = useMemo(
    () => combos.filter((c) => c.enabled).map((c) => c.name),
    [combos],
  );

  const catalogSpecs = useMemo(() => {
    return Array.from(
      new Set(
        models
          .filter((m) => m.owned_by !== "combo")
          .map((m) => {
            if (m.id.includes("/")) return m.id;
            if (m.owned_by === "antigravity") return `ag/${m.id}`;
            if (m.owned_by === "ovh") return `ovh/${m.id}`;
            return `oc/${m.id}`;
          }),
      ),
    );
  }, [models]);

  const allSelectableModels = useMemo(
    () => [...comboNames, ...catalogSpecs],
    [comboNames, catalogSpecs],
  );

  const fallbackPrimary = comboNames[0] || catalogSpecs[0] || "oc/deepseek-v4.1-flash-free";

  // Hydrate API key default
  useEffect(() => {
    if (!selectedApiKey) {
      const withRaw = keys.find((k) => k.enabled && k.raw_key);
      if (withRaw?.raw_key) {
        setSelectedApiKey(withRaw.raw_key);
      }
    }
  }, [keys, selectedApiKey]);

  // Hydrate OpenCode form from disk status
  useEffect(() => {
    const oc = cliStatus?.opencode;
    if (!oc) return;
    if (oc.models.length > 0 && ocModels.length === 0) {
      setOcModels(oc.models);
    } else if (ocModels.length === 0 && allSelectableModels.length > 0) {
      const seed = allSelectableModels.slice(0, 4);
      setOcModels(seed);
      setOcActiveModel(seed[0]);
      setOcSubagentModel(seed[0]);
    }
    if (oc.active_model && !ocActiveModel) setOcActiveModel(oc.active_model);
    if (oc.subagent_model && !ocSubagentModel) setOcSubagentModel(oc.subagent_model);
  }, [cliStatus?.opencode, allSelectableModels.length]); // eslint-disable-line react-hooks/exhaustive-deps

  // Hydrate Claude Code form from disk status
  useEffect(() => {
    const cl = cliStatus?.claude;
    if (!cl) return;
    setClDefault((cur) => cur || cl.default_model || fallbackPrimary);
    setClOpus((cur) => cur || cl.opus_model || fallbackPrimary);
    setClSonnet((cur) => cur || cl.sonnet_model || fallbackPrimary);
    setClHaiku((cur) => cur || cl.haiku_model || fallbackPrimary);
    setClSubagent((cur) => cur || cl.subagent_model || fallbackPrimary);
  }, [cliStatus?.claude, fallbackPrimary]);

  // Hydrate Codex form
  useEffect(() => {
    const cx = cliStatus?.codex;
    setCxModel((cur) => cur || cx?.active_model || fallbackPrimary);
    setCxSubagent((cur) => cur || fallbackPrimary);
  }, [cliStatus?.codex, fallbackPrimary]);

  if (loading && !cliStatus) {
    return <TableSkeleton rows={5} />;
  }

  function copyText(id: string, val: string) {
    void navigator.clipboard.writeText(val);
    setCopiedId(id);
    setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 1500);
  }

  async function handleQuickMintKey() {
    setCreatingKey(true);
    try {
      const created = await onCreateKey({ name: "cli-auto-key" });
      setSelectedApiKey(created.raw_key);
    } finally {
      setCreatingKey(false);
    }
  }

  const effectiveKey = selectedApiKey.trim() || "sk-tesie-local";

  function toggleOcModel(m: string) {
    setOcModels((prev) => {
      if (prev.includes(m)) {
        const next = prev.filter((x) => x !== m);
        if (ocActiveModel === m) setOcActiveModel(next[0] || "");
        return next;
      }
      const next = [...prev, m];
      if (!ocActiveModel) setOcActiveModel(m);
      return next;
    });
  }

  function buildOpenCodePreview(): string {
    const list = ocModels.length > 0 ? ocModels : [fallbackPrimary];
    const active = ocActiveModel || list[0];
    const sub = ocSubagentModel || active;
    const modelsObj: Record<string, unknown> = {};
    for (const m of list) {
      modelsObj[m] = {
        name: m,
        modalities: { input: ["text", "image"], output: ["text"] },
      };
    }
    return JSON.stringify(
      {
        $schema: "https://opencode.ai/config.json",
        provider: {
          tesie: {
            npm: "@ai-sdk/openai-compatible",
            name: "Tesie",
            options: {
              baseURL: baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`,
              apiKey: effectiveKey,
            },
            models: modelsObj,
          },
        },
        model: `tesie/${active}`,
        agent: {
          explorer: {
            description: "Fast explorer subagent routed via Tesie",
            mode: "subagent",
            model: `tesie/${sub}`,
          },
        },
      },
      null,
      2,
    );
  }

  function buildClaudePreview(): string {
    const cleanOrigin = baseUrl.replace(/\/+$/, "").replace(/\/v1$/, "");
    return JSON.stringify(
      {
        hasCompletedOnboarding: true,
        env: {
          ANTHROPIC_BASE_URL: cleanOrigin,
          ANTHROPIC_AUTH_TOKEN: effectiveKey,
          ...(clDefault ? { ANTHROPIC_MODEL: clDefault } : {}),
          ...(clOpus ? { ANTHROPIC_DEFAULT_OPUS_MODEL: clOpus } : {}),
          ...(clSonnet ? { ANTHROPIC_DEFAULT_SONNET_MODEL: clSonnet } : {}),
          ...(clHaiku ? { ANTHROPIC_DEFAULT_HAIKU_MODEL: clHaiku } : {}),
          ...(clSubagent ? { CLAUDE_CODE_SUBAGENT_MODEL: clSubagent } : {}),
        },
      },
      null,
      2,
    );
  }

  function buildCodexPreview(): string {
    const v1 = baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`;
    const m = cxModel || fallbackPrimary;
    const sub = cxSubagent || m;
    return `model = "${m}"
model_provider = "tesie"

[model_providers.tesie]
name = "Tesie"
base_url = "${v1}"
wire_api = "responses"
http_headers = { Authorization = "Bearer ${effectiveKey}" }

[agents]
default_subagent_model = "${sub}"`;
  }

  const ocStatus = cliStatus?.opencode;
  const clStatus = cliStatus?.claude;
  const cxStatus = cliStatus?.codex;

  const filteredOcCatalog = allSelectableModels.filter(
    (m) => !ocFilter.trim() || m.toLowerCase().includes(ocFilter.trim().toLowerCase()),
  );

  return (
    <div className="space-y-5">
      {/* 1. Header Overview Bar */}
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-lg font-bold tracking-tight text-[var(--text-primary)]">
              CLI &amp; IDE Developer Hub
            </h1>
            <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--success-soft)] bg-[var(--success-soft)]/40 px-2.5 py-0.5 text-[11px] font-medium text-[var(--success)]">
              <span className="h-1.5 w-1.5 rounded-full bg-[var(--success)] animate-pulse" />
              Gateway Active
            </span>
          </div>
          <p className="mt-0.5 text-xs text-[var(--text-muted)]">
            One-click local configuration for OpenCode, Claude Code, OpenAI Codex, and IDE extensions routing through Tesie.
          </p>
        </div>

        {/* Quick Config Count */}
        <div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
          <span className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2.5 py-1 font-mono">
            {baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`}
          </span>
          <button
            type="button"
            onClick={() => copyText("header-url", baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`)}
            className="flex items-center gap-1 text-[var(--accent)] hover:underline"
            title="Copy API Base URL"
          >
            {copiedId === "header-url" ? (
              <Check className="h-3.5 w-3.5 text-[var(--success)]" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
            <span>{copiedId === "header-url" ? "Copied" : "Copy"}</span>
          </button>
        </div>
      </div>

      {/* 2. Top Interactive Tool Switcher Strip (Status Cards) */}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {/* OpenCode Card */}
        <button
          type="button"
          onClick={() => setActiveTab("opencode")}
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-all ${
            activeTab === "opencode"
              ? "border-[var(--accent)] bg-[var(--bg-surface)] shadow-md ring-1 ring-[var(--accent)]/30"
              : "border-[var(--border-subtle)] bg-[var(--bg-surface)]/70 hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface)]"
          }`}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5">
              <ProviderIcon src="/providers/opencode.png" name="OpenCode" color="#E56A4A" size={26} />
              <span className="text-xs font-semibold text-[var(--text-primary)]">OpenCode CLI</span>
            </div>
            {ocStatus?.configured ? (
              <StatusPill tone="success" dot>Connected</StatusPill>
            ) : (
              <StatusPill tone="neutral">Ready</StatusPill>
            )}
          </div>
          <div className="mt-3 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
            <span className="truncate font-mono">
              {ocStatus?.active_model ? `tesie/${ocStatus.active_model}` : "Not configured"}
            </span>
            <span className="font-mono text-[10px] opacity-75">~/.config/opencode</span>
          </div>
        </button>

        {/* Claude Code Card */}
        <button
          type="button"
          onClick={() => setActiveTab("claude")}
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-all ${
            activeTab === "claude"
              ? "border-[var(--accent)] bg-[var(--bg-surface)] shadow-md ring-1 ring-[var(--accent)]/30"
              : "border-[var(--border-subtle)] bg-[var(--bg-surface)]/70 hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface)]"
          }`}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5">
              <ProviderIcon src="/providers/claude.png" name="Claude Code" color="#D97757" size={26} />
              <span className="text-xs font-semibold text-[var(--text-primary)]">Claude Code</span>
            </div>
            {clStatus?.configured ? (
              <StatusPill tone="success" dot>Connected</StatusPill>
            ) : (
              <StatusPill tone="neutral">Ready</StatusPill>
            )}
          </div>
          <div className="mt-3 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
            <span className="truncate font-mono">
              {clStatus?.sonnet_model || clStatus?.default_model || "Native Anthropic"}
            </span>
            <span className="font-mono text-[10px] opacity-75">~/.claude/settings</span>
          </div>
        </button>

        {/* Codex Card */}
        <button
          type="button"
          onClick={() => setActiveTab("codex")}
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-all ${
            activeTab === "codex"
              ? "border-[var(--accent)] bg-[var(--bg-surface)] shadow-md ring-1 ring-[var(--accent)]/30"
              : "border-[var(--border-subtle)] bg-[var(--bg-surface)]/70 hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface)]"
          }`}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5">
              <ProviderIcon src="/providers/codex.png" name="Codex CLI" color="#10A37F" size={26} />
              <span className="text-xs font-semibold text-[var(--text-primary)]">OpenAI Codex</span>
            </div>
            {cxStatus?.configured ? (
              <StatusPill tone="success" dot>Connected</StatusPill>
            ) : (
              <StatusPill tone="neutral">Ready</StatusPill>
            )}
          </div>
          <div className="mt-3 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
            <span className="truncate font-mono">
              {cxStatus?.active_model || "TOML config"}
            </span>
            <span className="font-mono text-[10px] opacity-75">~/.codex/config.toml</span>
          </div>
        </button>

        {/* IDE Extensions Card */}
        <button
          type="button"
          onClick={() => setActiveTab("ides")}
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-all ${
            activeTab === "ides"
              ? "border-[var(--accent)] bg-[var(--bg-surface)] shadow-md ring-1 ring-[var(--accent)]/30"
              : "border-[var(--border-subtle)] bg-[var(--bg-surface)]/70 hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface)]"
          }`}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5">
              <div className="flex h-[26px] w-[26px] items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
                <Code2 className="h-4 w-4" />
              </div>
              <span className="text-xs font-semibold text-[var(--text-primary)]">IDE Extensions</span>
            </div>
            <span className="text-[10px] font-medium text-[var(--text-muted)]">OpenAI Wire</span>
          </div>
          <div className="mt-3 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
            <span className="truncate">Cursor · Cline · Roo</span>
            <span className="font-mono text-[10px] text-[var(--accent)]">Universal</span>
          </div>
        </button>
      </div>

      {/* 3. Gateway Configuration Toolbar */}
      <div className="grid grid-cols-1 gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3 md:grid-cols-12">
        <div className="md:col-span-6">
          <label className="mb-1 block text-[11px] font-medium text-[var(--text-muted)]">
            Gateway Endpoint URL (Base V1)
          </label>
          <div className="flex items-center gap-2">
            <input
              type="text"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              className={`${INPUT_CLASS} py-1.5 font-mono text-xs`}
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => copyText("bar-base-url", baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`)}
              title="Copy endpoint"
            >
              {copiedId === "bar-base-url" ? <Check className="h-3.5 w-3.5 text-[var(--success)]" /> : <Copy className="h-3.5 w-3.5" />}
              {copiedId === "bar-base-url" ? "Copied" : "Copy"}
            </Button>
          </div>
        </div>

        <div className="md:col-span-6">
          <label className="mb-1 block text-[11px] font-medium text-[var(--text-muted)]">
            Client Authentication Key
          </label>
          <div className="flex items-center gap-2">
            <select
              value={selectedApiKey}
              onChange={(e) => setSelectedApiKey(e.target.value)}
              className={`${INPUT_CLASS} flex-1 py-1.5 font-mono text-xs`}
            >
              <option value="">
                {keys.some((k) => k.enabled)
                  ? "Select key (or leave empty for sk-tesie-local)..."
                  : "sk-tesie-local (open local loopback)"}
              </option>
              {keys
                .filter((k) => k.enabled && k.raw_key)
                .map((k) => (
                  <option key={k.id} value={k.raw_key || ""}>
                    {k.name} ({k.key_prefix}...)
                  </option>
                ))}
            </select>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void handleQuickMintKey()}
              disabled={creatingKey}
              title="Generate a dedicated API key for CLI tools"
            >
              <KeyRound className="h-3.5 w-3.5" />
              {creatingKey ? "Minting..." : "Mint Key"}
            </Button>
          </div>
        </div>
      </div>

      {/* 4. Active Tool Workspace */}
      <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-5 sm:p-6 shadow-xs">
        {/* ========================================================================= */}
        {/* TAB 1: OPENCODE CLI */}
        {/* ========================================================================= */}
        {activeTab === "opencode" && (
          <div className="space-y-6">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] pb-4">
              <div className="flex items-center gap-3">
                <ProviderIcon src="/providers/opencode.png" name="OpenCode" color="#E56A4A" size={32} />
                <div>
                  <h2 className="text-sm font-bold text-[var(--text-primary)]">OpenCode CLI Configuration</h2>
                  <p className="font-mono text-[11px] text-[var(--text-muted)]">
                    Target: {ocStatus?.config_path || "~/.config/opencode/opencode.json"}
                  </p>
                </div>
              </div>

              {/* Terminal Snippet Box */}
              <div className="flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-3 py-1.5 font-mono text-xs">
                <span className="text-[var(--text-muted)]">$</span>
                <span className="font-semibold text-[var(--text-primary)]">opencode</span>
                <button
                  type="button"
                  onClick={() => copyText("run-opencode", "opencode")}
                  className="ml-1 text-[var(--text-muted)] hover:text-[var(--accent)]"
                  title="Copy terminal command"
                >
                  {copiedId === "run-opencode" ? <Check className="h-3.5 w-3.5 text-[var(--success)]" /> : <Copy className="h-3.5 w-3.5" />}
                </button>
              </div>
            </div>

            {/* Model Config Grid */}
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <div>
                <label className="mb-1.5 block text-xs font-semibold text-[var(--text-secondary)]">
                  Primary Interactive Model (`model`)
                </label>
                <select
                  value={ocActiveModel}
                  onChange={(e) => setOcActiveModel(e.target.value)}
                  className={`${INPUT_CLASS} font-mono text-xs`}
                >
                  {(ocModels.length > 0 ? ocModels : allSelectableModels).map((m) => (
                    <option key={m} value={m}>
                      tesie/{m} {comboNames.includes(m) ? "★ [Combo]" : ""}
                    </option>
                  ))}
                </select>
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  Main model loaded when invoking `opencode`.
                </p>
              </div>

              <div>
                <label className="mb-1.5 block text-xs font-semibold text-[var(--text-secondary)]">
                  Explorer Subagent Model (`agent.explorer.model`)
                </label>
                <select
                  value={ocSubagentModel}
                  onChange={(e) => setOcSubagentModel(e.target.value)}
                  className={`${INPUT_CLASS} font-mono text-xs`}
                >
                  {(ocModels.length > 0 ? ocModels : allSelectableModels).map((m) => (
                    <option key={m} value={m}>
                      tesie/{m} {comboNames.includes(m) ? "★ [Combo]" : ""}
                    </option>
                  ))}
                </select>
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  Subagent model used for background codebase exploration and reviews.
                </p>
              </div>
            </div>

            {/* Registered Models Chip Selector */}
            <div className="space-y-2.5">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-semibold text-[var(--text-secondary)]">
                    Registered Models &amp; Combos ({ocModels.length} selected)
                  </span>
                  <span className="text-[11px] text-[var(--text-muted)]">
                    Visible inside OpenCode model switcher (`/model`)
                  </span>
                </div>

                <div className="flex items-center gap-2">
                  <div className="relative w-44">
                    <Search className="pointer-events-none absolute top-2 left-2.5 h-3.5 w-3.5 text-[var(--text-muted)]" />
                    <input
                      type="text"
                      placeholder="Filter catalog..."
                      value={ocFilter}
                      onChange={(e) => setOcFilter(e.target.value)}
                      className={`${INPUT_CLASS} py-1 pl-8 text-xs`}
                    />
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      const top = allSelectableModels.slice(0, 6);
                      setOcModels(top);
                      if (!ocActiveModel && top[0]) setOcActiveModel(top[0]);
                    }}
                  >
                    <Sparkles className="h-3 w-3 text-[var(--accent)]" />
                    Top 6
                  </Button>
                </div>
              </div>

              {/* Tag Cloud */}
              <div className="flex max-h-48 flex-wrap gap-1.5 overflow-y-auto rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 p-3">
                {filteredOcCatalog.map((m) => {
                  const checked = ocModels.includes(m);
                  const isCombo = comboNames.includes(m);
                  return (
                    <button
                      key={m}
                      type="button"
                      onClick={() => toggleOcModel(m)}
                      className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 font-mono text-xs transition-colors ${
                        checked
                          ? "border-[var(--accent)] bg-[var(--accent-soft)] font-medium text-[var(--accent)]"
                          : "border-[var(--border-subtle)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]"
                      }`}
                    >
                      {checked ? <Check className="h-3 w-3" /> : <Plus className="h-3 w-3" />}
                      {isCombo ? `★ [combo] ${m}` : m}
                    </button>
                  );
                })}
              </div>
            </div>

            {/* Action Bar & Preview */}
            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-subtle)] pt-4">
              <div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
                <FileCode2 className="h-4 w-4" />
                <span>Writes clean schema-compliant configuration directly to disk.</span>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() =>
                    setPreviewModal({
                      title: "OpenCode JSON Preview",
                      path: ocStatus?.config_path || "~/.config/opencode/opencode.json",
                      content: buildOpenCodePreview(),
                    })
                  }
                >
                  <FileCode2 className="h-3.5 w-3.5" />
                  Preview JSON
                </Button>

                {ocStatus?.configured && (
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={ocBusy}
                    onClick={async () => {
                      setOcBusy(true);
                      try {
                        await onResetOpenCode();
                      } finally {
                        setOcBusy(false);
                      }
                    }}
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    Reset
                  </Button>
                )}

                <Button
                  variant="primary"
                  size="sm"
                  disabled={ocBusy || ocModels.length === 0}
                  onClick={async () => {
                    setOcBusy(true);
                    try {
                      await onApplyOpenCode({
                        base_url: baseUrl,
                        api_key: selectedApiKey || undefined,
                        models: ocModels,
                        active_model: ocActiveModel || ocModels[0],
                        subagent_model: ocSubagentModel || ocActiveModel || ocModels[0],
                      });
                    } finally {
                      setOcBusy(false);
                    }
                  }}
                >
                  <Terminal className="h-3.5 w-3.5" />
                  {ocBusy ? "Applying..." : "Apply to OpenCode"}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* ========================================================================= */}
        {/* TAB 2: CLAUDE CODE CLI */}
        {/* ========================================================================= */}
        {activeTab === "claude" && (
          <div className="space-y-6">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] pb-4">
              <div className="flex items-center gap-3">
                <ProviderIcon src="/providers/claude.png" name="Claude Code" color="#D97757" size={32} />
                <div>
                  <h2 className="text-sm font-bold text-[var(--text-primary)]">Claude Code CLI Configuration</h2>
                  <p className="font-mono text-[11px] text-[var(--text-muted)]">
                    Target: {clStatus?.config_path || "~/.claude/settings.json"}
                  </p>
                </div>
              </div>

              {/* Terminal Snippet Box */}
              <div className="flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-3 py-1.5 font-mono text-xs">
                <span className="text-[var(--text-muted)]">$</span>
                <span className="font-semibold text-[var(--text-primary)]">claude</span>
                <button
                  type="button"
                  onClick={() => copyText("run-claude", "claude")}
                  className="ml-1 text-[var(--text-muted)] hover:text-[var(--accent)]"
                  title="Copy terminal command"
                >
                  {copiedId === "run-claude" ? <Check className="h-3.5 w-3.5 text-[var(--success)]" /> : <Copy className="h-3.5 w-3.5" />}
                </button>
              </div>
            </div>

            <datalist id="cli-all-models-list">
              {allSelectableModels.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>

            {/* Model Inputs Grid */}
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {[
                {
                  label: "Primary Model (ANTHROPIC_MODEL)",
                  hint: "Default model used for all standard coding turns",
                  val: clDefault,
                  set: setClDefault,
                },
                {
                  label: "Sonnet Tier (DEFAULT_SONNET_MODEL)",
                  hint: "Model invoked during sonnet level queries",
                  val: clSonnet,
                  set: setClSonnet,
                },
                {
                  label: "Subagent Model (SUBAGENT_MODEL)",
                  hint: "Model invoked for background researcher subagents",
                  val: clSubagent,
                  set: setClSubagent,
                },
                {
                  label: "Opus Tier (DEFAULT_OPUS_MODEL)",
                  hint: "Heavy reasoning model for architect queries",
                  val: clOpus,
                  set: setClOpus,
                },
                {
                  label: "Haiku Tier (DEFAULT_HAIKU_MODEL)",
                  hint: "Fast light model for summaries and linting",
                  val: clHaiku,
                  set: setClHaiku,
                },
              ].map((slot) => (
                <div key={slot.label} className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-3.5">
                  <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                    {slot.label}
                  </label>
                  <input
                    list="cli-all-models-list"
                    value={slot.val}
                    onChange={(e) => slot.set(e.target.value)}
                    placeholder={fallbackPrimary}
                    className={`${INPUT_CLASS} font-mono text-xs`}
                  />
                  <p className="mt-1.5 text-[11px] text-[var(--text-muted)]">
                    {slot.hint}
                  </p>
                </div>
              ))}
            </div>

            {/* Action Bar & Preview */}
            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-subtle)] pt-4">
              <div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
                <FileCode2 className="h-4 w-4" />
                <span>Injects ANTHROPIC_BASE_URL and ANTHROPIC_AUTH_TOKEN transparently into settings.json.</span>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() =>
                    setPreviewModal({
                      title: "Claude Code settings.json",
                      path: clStatus?.config_path || "~/.claude/settings.json",
                      content: buildClaudePreview(),
                    })
                  }
                >
                  <FileCode2 className="h-3.5 w-3.5" />
                  Preview JSON
                </Button>

                {clStatus?.configured && (
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={clBusy}
                    onClick={async () => {
                      setClBusy(true);
                      try {
                        await onResetClaude();
                      } finally {
                        setClBusy(false);
                      }
                    }}
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    Reset
                  </Button>
                )}

                <Button
                  variant="primary"
                  size="sm"
                  disabled={clBusy}
                  onClick={async () => {
                    setClBusy(true);
                    try {
                      await onApplyClaude({
                        base_url: baseUrl,
                        api_key: selectedApiKey || undefined,
                        default_model: clDefault,
                        opus_model: clOpus,
                        sonnet_model: clSonnet,
                        haiku_model: clHaiku,
                        subagent_model: clSubagent,
                      });
                    } finally {
                      setClBusy(false);
                    }
                  }}
                >
                  <Terminal className="h-3.5 w-3.5" />
                  {clBusy ? "Applying..." : "Apply to Claude Code"}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* ========================================================================= */}
        {/* TAB 3: OPENAI CODEX CLI */}
        {/* ========================================================================= */}
        {activeTab === "codex" && (
          <div className="space-y-6">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] pb-4">
              <div className="flex items-center gap-3">
                <ProviderIcon src="/providers/codex.png" name="Codex CLI" color="#10A37F" size={32} />
                <div>
                  <h2 className="text-sm font-bold text-[var(--text-primary)]">OpenAI Codex CLI Configuration</h2>
                  <p className="font-mono text-[11px] text-[var(--text-muted)]">
                    Target: {cxStatus?.config_path || "~/.codex/config.toml"}
                  </p>
                </div>
              </div>

              {/* Terminal Snippet Box */}
              <div className="flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-3 py-1.5 font-mono text-xs">
                <span className="text-[var(--text-muted)]">$</span>
                <span className="font-semibold text-[var(--text-primary)]">codex</span>
                <button
                  type="button"
                  onClick={() => copyText("run-codex", "codex")}
                  className="ml-1 text-[var(--text-muted)] hover:text-[var(--accent)]"
                  title="Copy terminal command"
                >
                  {copiedId === "run-codex" ? <Check className="h-3.5 w-3.5 text-[var(--success)]" /> : <Copy className="h-3.5 w-3.5" />}
                </button>
              </div>
            </div>

            {/* Model Config Grid */}
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <div>
                <label className="mb-1.5 block text-xs font-semibold text-[var(--text-secondary)]">
                  Primary Codex Model (`model`)
                </label>
                <select
                  value={cxModel}
                  onChange={(e) => setCxModel(e.target.value)}
                  className={`${INPUT_CLASS} font-mono text-xs`}
                >
                  {allSelectableModels.map((m) => (
                    <option key={m} value={m}>
                      {m} {comboNames.includes(m) ? "★ [Combo]" : ""}
                    </option>
                  ))}
                </select>
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  Primary model invoked for coding instructions and code generation.
                </p>
              </div>

              <div>
                <label className="mb-1.5 block text-xs font-semibold text-[var(--text-secondary)]">
                  Default Subagent Model (`default_subagent_model`)
                </label>
                <select
                  value={cxSubagent}
                  onChange={(e) => setCxSubagent(e.target.value)}
                  className={`${INPUT_CLASS} font-mono text-xs`}
                >
                  {allSelectableModels.map((m) => (
                    <option key={m} value={m}>
                      {m} {comboNames.includes(m) ? "★ [Combo]" : ""}
                    </option>
                  ))}
                </select>
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  Subagent model used for secondary code analysis and test execution.
                </p>
              </div>
            </div>

            {/* Action Bar & Preview */}
            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-subtle)] pt-4">
              <div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
                <FileCode2 className="h-4 w-4" />
                <span>Writes clean TOML configuration directly to ~/.codex/config.toml.</span>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() =>
                    setPreviewModal({
                      title: "Codex config.toml",
                      path: cxStatus?.config_path || "~/.codex/config.toml",
                      content: buildCodexPreview(),
                    })
                  }
                >
                  <FileCode2 className="h-3.5 w-3.5" />
                  Preview TOML
                </Button>

                {cxStatus?.configured && (
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={cxBusy}
                    onClick={async () => {
                      setCxBusy(true);
                      try {
                        await onResetCodex();
                      } finally {
                        setCxBusy(false);
                      }
                    }}
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    Reset
                  </Button>
                )}

                <Button
                  variant="primary"
                  size="sm"
                  disabled={cxBusy || !cxModel}
                  onClick={async () => {
                    setCxBusy(true);
                    try {
                      await onApplyCodex({
                        base_url: baseUrl,
                        api_key: selectedApiKey || undefined,
                        model: cxModel,
                        subagent_model: cxSubagent || cxModel,
                      });
                    } finally {
                      setCxBusy(false);
                    }
                  }}
                >
                  <Terminal className="h-3.5 w-3.5" />
                  {cxBusy ? "Applying..." : "Apply to Codex"}
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* ========================================================================= */}
        {/* TAB 4: IDE EXTENSIONS (CURSOR, CLINE, ROO CODE) */}
        {/* ========================================================================= */}
        {activeTab === "ides" && (
          <div className="space-y-6">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] pb-4">
              <div className="flex items-center gap-3">
                <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
                  <Code2 className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="text-sm font-bold text-[var(--text-primary)]">Universal IDE Extensions Setup</h2>
                  <p className="text-[11px] text-[var(--text-muted)]">
                    Use any OpenAI-compatible IDE extension with Tesie (Cursor, Cline, Roo Code, Windsurf, Continue).
                  </p>
                </div>
              </div>
            </div>

            <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
              {/* Cursor Card */}
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-bold text-[var(--text-primary)]">Cursor AI Editor</span>
                  <span className="rounded bg-[var(--accent-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--accent)]">
                    OpenAI API
                  </span>
                </div>
                <p className="text-xs text-[var(--text-secondary)]">
                  Go to <strong>Cursor Settings &gt; Models &gt; OpenAI API Key</strong>:
                </p>
                <div className="space-y-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3 text-xs font-mono">
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Base URL:</span>
                    <button
                      type="button"
                      onClick={() => copyText("cursor-base", baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`)}
                      className="text-[var(--accent)] hover:underline flex items-center gap-1"
                    >
                      <span>{baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`}</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">API Key:</span>
                    <button
                      type="button"
                      onClick={() => copyText("cursor-key", effectiveKey)}
                      className="text-[var(--accent)] hover:underline flex items-center gap-1"
                    >
                      <span>{effectiveKey.slice(0, 14)}...</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Model ID:</span>
                    <span className="text-[var(--text-primary)]">{fallbackPrimary}</span>
                  </div>
                </div>
              </div>

              {/* Cline & Roo Code Card */}
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-bold text-[var(--text-primary)]">Cline &amp; Roo Code</span>
                  <span className="rounded bg-[var(--accent-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--accent)]">
                    VS Code Extension
                  </span>
                </div>
                <p className="text-xs text-[var(--text-secondary)]">
                  Set API Provider to <strong>OpenAI-Compatible</strong> in extension settings:
                </p>
                <div className="space-y-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3 text-xs font-mono">
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Base URL:</span>
                    <button
                      type="button"
                      onClick={() => copyText("cline-base", baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`)}
                      className="text-[var(--accent)] hover:underline flex items-center gap-1"
                    >
                      <span>{baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`}</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">API Key:</span>
                    <button
                      type="button"
                      onClick={() => copyText("cline-key", effectiveKey)}
                      className="text-[var(--accent)] hover:underline flex items-center gap-1"
                    >
                      <span>{effectiveKey.slice(0, 14)}...</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-[var(--text-muted)]">Model ID:</span>
                    <span className="text-[var(--text-primary)]">{fallbackPrimary}</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Config Preview Modal */}
      <Modal
        open={!!previewModal}
        title={previewModal?.title ?? "Configuration preview"}
        onClose={() => setPreviewModal(null)}
        maxWidth="max-w-2xl"
      >
        {previewModal && (
          <div className="space-y-3">
            <div className="flex items-center justify-between gap-2">
              <span className="font-mono text-xs text-[var(--text-muted)]">
                {previewModal.path}
              </span>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => copyText("modal-preview", previewModal.content)}
              >
                {copiedId === "modal-preview" ? (
                  <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                ) : (
                  <Copy className="h-3.5 w-3.5" />
                )}
                {copiedId === "modal-preview" ? "Copied" : "Copy config"}
              </Button>
            </div>
            <pre className="max-h-96 overflow-auto rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 font-mono text-xs leading-relaxed text-[var(--text-secondary)]">
              {previewModal.content}
            </pre>
            <div className="flex justify-end">
              <Button variant="secondary" size="md" onClick={() => setPreviewModal(null)}>
                <X className="h-3.5 w-3.5" />
                Close
              </Button>
            </div>
          </div>
        )}
      </Modal>
    </div>
  );
}
