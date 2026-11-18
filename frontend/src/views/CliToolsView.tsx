import { useEffect, useState } from "react";
import {
  Check,
  ChevronDown,
  Copy,
  FileCode2,
  KeyRound,
  Plus,
  RotateCcw,
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

  const [expandedTool, setExpandedTool] = useState<string>("opencode");
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
  const comboNames = combos.filter((c) => c.enabled).map((c) => c.name);
  const catalogSpecs = Array.from(
    new Set(
      models
        .filter((m) => m.owned_by !== "combo")
        .map((m) => {
          if (m.id.includes("/")) return m.id;
          if (m.owned_by === "antigravity") return `ag/${m.id}`;
          return `oc/${m.id}`;
        }),
    ),
  );
  const allSelectableModels = [...comboNames, ...catalogSpecs];
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
    <div className="space-y-4">
      {/* Compact Shared Endpoint & API Key Bar */}
      <div className="grid grid-cols-1 gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3 md:grid-cols-12">
        <div className="md:col-span-6">
          <label className="mb-1 block text-[11px] font-medium text-[var(--text-muted)]">
            Gateway endpoint URL
          </label>
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            className={`${INPUT_CLASS} py-1 font-mono`}
          />
        </div>

        <div className="md:col-span-6">
          <label className="mb-1 block text-[11px] font-medium text-[var(--text-muted)]">
            Client API key
          </label>
          <div className="flex gap-2">
            <select
              value={selectedApiKey}
              onChange={(e) => setSelectedApiKey(e.target.value)}
              className={`${INPUT_CLASS} flex-1 py-1 font-mono`}
            >
              <option value="">
                {keys.some((k) => k.enabled)
                  ? "Select or generate a key..."
                  : "sk-tesie-local (open local access)"}
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
              title="Create a dedicated API key for CLI tools"
            >
              <KeyRound className="h-3.5 w-3.5" />
              {creatingKey ? "..." : "New key"}
            </Button>
          </div>
        </div>
      </div>

      {/* Auto-Configurable CLI Assistants */}
      <div className="overflow-hidden rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] divide-y divide-[var(--border-subtle)]">
        {/* 1. OPENCODE CLI */}
        <div>
          <button
            type="button"
            onClick={() => setExpandedTool((cur) => (cur === "opencode" ? "" : "opencode"))}
            className="flex w-full items-center justify-between gap-3 px-5 py-3.5 text-left transition-colors hover:bg-[var(--bg-hover)]/45"
          >
            <div className="flex min-w-0 items-center gap-3">
              <ProviderIcon
                src="/providers/opencode.png"
                name="OpenCode"
                color="#E56A4A"
                size={34}
              />
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold text-[var(--text-primary)]">
                    OpenCode CLI
                  </span>
                  {ocStatus?.configured ? (
                    <StatusPill tone="success" dot>
                      Connected
                    </StatusPill>
                  ) : ocStatus?.installed ? (
                    <StatusPill tone="warning">Not configured</StatusPill>
                  ) : (
                    <StatusPill tone="neutral">Ready</StatusPill>
                  )}
                  {ocStatus?.active_model && (
                    <span className="font-mono text-[11px] text-[var(--text-muted)]">
                      tesie/{ocStatus.active_model}
                    </span>
                  )}
                </div>
                <p className="mt-0.5 truncate font-mono text-[11px] text-[var(--text-muted)]">
                  {ocStatus?.config_path || "~/.config/opencode/opencode.json"}
                </p>
              </div>
            </div>

            <ChevronDown
              className={`h-4 w-4 shrink-0 text-[var(--text-muted)] transition-transform ${
                expandedTool === "opencode" ? "rotate-180" : ""
              }`}
            />
          </button>

          {expandedTool === "opencode" && (
            <div className="space-y-4 border-t border-[var(--border-subtle)] bg-[var(--bg-canvas)]/35 p-5">
              {/* Model Selection Chips */}
              <div className="space-y-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <label className="text-xs font-medium text-[var(--text-secondary)]">
                    Models & combos registered in OpenCode ({ocModels.length} selected)
                  </label>
                  <div className="flex items-center gap-2">
                    <input
                      type="text"
                      placeholder="Filter models..."
                      value={ocFilter}
                      onChange={(e) => setOcFilter(e.target.value)}
                      className={`${INPUT_CLASS} w-44 py-1`}
                    />
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
                      Select top 6
                    </Button>
                  </div>
                </div>

                <div className="flex max-h-40 flex-wrap gap-1.5 overflow-y-auto rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3">
                  {filteredOcCatalog.map((m) => {
                    const checked = ocModels.includes(m);
                    const isCombo = comboNames.includes(m);
                    return (
                      <button
                        key={m}
                        type="button"
                        onClick={() => toggleOcModel(m)}
                        className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 font-mono text-[11px] transition-colors ${
                          checked
                            ? "border-[var(--accent)] bg-[var(--accent-soft)] font-medium text-[var(--accent)]"
                            : "border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
                        }`}
                      >
                        {checked ? <Check className="h-3 w-3" /> : <Plus className="h-3 w-3" />}
                        {isCombo ? `[combo] ${m}` : m}
                      </button>
                    );
                  })}
                </div>
              </div>

              {/* Active Model & Subagent Model */}
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Default active model (`model` in opencode.json)
                  </label>
                  <select
                    value={ocActiveModel}
                    onChange={(e) => setOcActiveModel(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  >
                    {(ocModels.length > 0 ? ocModels : allSelectableModels).map((m) => (
                      <option key={m} value={m}>
                        tesie/{m}
                      </option>
                    ))}
                  </select>
                </div>

                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Explorer subagent model (`agent.explorer.model`)
                  </label>
                  <select
                    value={ocSubagentModel}
                    onChange={(e) => setOcSubagentModel(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  >
                    {(ocModels.length > 0 ? ocModels : allSelectableModels).map((m) => (
                      <option key={m} value={m}>
                        tesie/{m}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-subtle)] pt-3">
                <span className="font-mono text-[11px] text-[var(--text-muted)]">
                  {ocStatus?.config_path || "~/.config/opencode/opencode.json"}
                </span>

                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() =>
                      setPreviewModal({
                        title: "OpenCode manual configuration",
                        path: "~/.config/opencode/opencode.json",
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
        </div>

        {/* 2. CLAUDE CODE CLI */}
        <div>
          <button
            type="button"
            onClick={() => setExpandedTool((cur) => (cur === "claude" ? "" : "claude"))}
            className="flex w-full items-center justify-between gap-3 px-5 py-3.5 text-left transition-colors hover:bg-[var(--bg-hover)]/45"
          >
            <div className="flex min-w-0 items-center gap-3">
              <ProviderIcon
                src="/providers/claude.png"
                name="Claude Code"
                color="#D97757"
                size={34}
              />
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold text-[var(--text-primary)]">
                    Claude Code CLI
                  </span>
                  {clStatus?.configured ? (
                    <StatusPill tone="success" dot>
                      Connected
                    </StatusPill>
                  ) : clStatus?.installed ? (
                    <StatusPill tone="warning">Not configured</StatusPill>
                  ) : (
                    <StatusPill tone="neutral">Ready</StatusPill>
                  )}
                  {clStatus?.sonnet_model && (
                    <span className="font-mono text-[11px] text-[var(--text-muted)]">
                      {clStatus.sonnet_model}
                    </span>
                  )}
                </div>
                <p className="mt-0.5 truncate font-mono text-[11px] text-[var(--text-muted)]">
                  {clStatus?.config_path || "~/.claude/settings.json"}
                </p>
              </div>
            </div>

            <ChevronDown
              className={`h-4 w-4 shrink-0 text-[var(--text-muted)] transition-transform ${
                expandedTool === "claude" ? "rotate-180" : ""
              }`}
            />
          </button>

          {expandedTool === "claude" && (
            <div className="space-y-4 border-t border-[var(--border-subtle)] bg-[var(--bg-canvas)]/35 p-5">
              <datalist id="cli-all-models-list">
                {allSelectableModels.map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>

              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {[
                  { label: "Default model (ANTHROPIC_MODEL)", val: clDefault, set: setClDefault },
                  {
                    label: "Opus tier (DEFAULT_OPUS_MODEL)",
                    val: clOpus,
                    set: setClOpus,
                  },
                  {
                    label: "Sonnet tier (DEFAULT_SONNET_MODEL)",
                    val: clSonnet,
                    set: setClSonnet,
                  },
                  {
                    label: "Haiku tier (DEFAULT_HAIKU_MODEL)",
                    val: clHaiku,
                    set: setClHaiku,
                  },
                  {
                    label: "Subagent model (SUBAGENT_MODEL)",
                    val: clSubagent,
                    set: setClSubagent,
                  },
                ].map((slot) => (
                  <div key={slot.label}>
                    <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                      {slot.label}
                    </label>
                    <input
                      list="cli-all-models-list"
                      value={slot.val}
                      onChange={(e) => slot.set(e.target.value)}
                      placeholder={fallbackPrimary}
                      className={`${INPUT_CLASS} font-mono`}
                    />
                  </div>
                ))}
              </div>

              <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-subtle)] pt-3">
                <span className="font-mono text-[11px] text-[var(--text-muted)]">
                  {clStatus?.config_path || "~/.claude/settings.json"}
                </span>

                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() =>
                      setPreviewModal({
                        title: "Claude Code settings.json",
                        path: "~/.claude/settings.json",
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
        </div>

        {/* 3. OPENAI CODEX CLI */}
        <div>
          <button
            type="button"
            onClick={() => setExpandedTool((cur) => (cur === "codex" ? "" : "codex"))}
            className="flex w-full items-center justify-between gap-3 px-5 py-3.5 text-left transition-colors hover:bg-[var(--bg-hover)]/45"
          >
            <div className="flex min-w-0 items-center gap-3">
              <ProviderIcon
                src="/providers/codex.png"
                name="Codex CLI"
                color="#10A37F"
                size={34}
              />
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold text-[var(--text-primary)]">
                    OpenAI Codex CLI
                  </span>
                  {cxStatus?.configured ? (
                    <StatusPill tone="success" dot>
                      Connected
                    </StatusPill>
                  ) : cxStatus?.installed ? (
                    <StatusPill tone="warning">Not configured</StatusPill>
                  ) : (
                    <StatusPill tone="neutral">Ready</StatusPill>
                  )}
                </div>
                <p className="mt-0.5 truncate font-mono text-[11px] text-[var(--text-muted)]">
                  {cxStatus?.config_path || "~/.codex/config.toml"}
                </p>
              </div>
            </div>

            <ChevronDown
              className={`h-4 w-4 shrink-0 text-[var(--text-muted)] transition-transform ${
                expandedTool === "codex" ? "rotate-180" : ""
              }`}
            />
          </button>

          {expandedTool === "codex" && (
            <div className="space-y-4 border-t border-[var(--border-subtle)] bg-[var(--bg-canvas)]/35 p-5">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Primary Codex model
                  </label>
                  <select
                    value={cxModel}
                    onChange={(e) => setCxModel(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  >
                    {allSelectableModels.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Default subagent model
                  </label>
                  <select
                    value={cxSubagent}
                    onChange={(e) => setCxSubagent(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  >
                    {allSelectableModels.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border-subtle)] pt-3">
                <span className="font-mono text-[11px] text-[var(--text-muted)]">
                  {cxStatus?.config_path || "~/.codex/config.toml"}
                </span>

                <div className="flex flex-wrap items-center gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() =>
                      setPreviewModal({
                        title: "Codex config.toml",
                        path: "~/.codex/config.toml",
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
        </div>
      </div>

      {/* Compact OpenAI-Compatible IDE Quick Bar */}
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3 text-xs">
        <div className="flex flex-wrap items-center gap-2 text-[var(--text-muted)]">
          <span className="font-medium text-[var(--text-primary)]">IDE Extensions</span>
          <span>&middot;</span>
          <span>Cursor, Cline &amp; Roo Code (OpenAI-Compatible)</span>
        </div>
        <div className="flex items-center gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2.5 py-1 font-mono text-[11px] text-[var(--text-secondary)]">
          <span>{baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`}</span>
          <button
            type="button"
            onClick={() =>
              copyText("ide-base-url", baseUrl.endsWith("/v1") ? baseUrl : `${baseUrl}/v1`)
            }
            className="font-sans font-medium text-[var(--accent)] hover:underline"
          >
            {copiedId === "ide-base-url" ? "Copied" : "Copy URL"}
          </button>
        </div>
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
