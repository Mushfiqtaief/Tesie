import { memo, useEffect, useMemo, useState } from "react";
import {
  ArrowRight,
  Check,
  CheckCircle2,
  Code2,
  Copy,
  Cpu,
  FileCode2,
  Globe,
  KeyRound,
  Layers,
  Network,
  Plus,
  Radio,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  ShieldAlert,
  Sliders,
  Sparkles,
  Terminal,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { api } from "../api";
import { ProviderIcon } from "../components/ProviderIcon";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  INPUT_CLASS,
  Modal,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  AntigravityMitmConfig,
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

type ToolTab = "opencode" | "claude" | "codex" | "ides" | "mitm";

interface InterceptedModelDef {
  id: string;
  name: string;
  family: "Gemini" | "Claude" | "GPT";
}

const ANTIGRAVITY_INTERCEPTED_MODELS: InterceptedModelDef[] = [
  { id: "gemini-3.8-flash", name: "Gemini 3.8 Flash", family: "Gemini" },
  { id: "gemini-4-argon", name: "Gemini 4 Argon", family: "Gemini" },
  { id: "gemini-4-hydrogen", name: "Gemini 4 Hydrogen", family: "Gemini" },
  { id: "gemini-4-neon", name: "Gemini 4 Neon", family: "Gemini" },
  { id: "gemini-4-carbon", name: "Gemini 4 Carbon", family: "Gemini" },
  { id: "claude-opus-5-5", name: "Claude Opus 5.5", family: "Claude" },
  { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", family: "Claude" },
  { id: "claude-sonnet-4-6", name: "Claude Sonnet 4.6", family: "Claude" },
  { id: "claude-opus-4-6-thinking", name: "Claude Opus 4.6", family: "Claude" },
  { id: "gpt-oss-120b-medium", name: "GPT-OSS 120B", family: "GPT" },
  { id: "gemini-3-flash", name: "Gemini 3 Flash", family: "Gemini" },
  { id: "gemini-3.5-flash-lite", name: "Gemini 3.5 Flash Lite", family: "Gemini" },
  { id: "gemini-3.1-flash-lite", name: "Gemini 3.1 Flash Lite", family: "Gemini" },
  { id: "gemini-2.5-flash", name: "Gemini 2.5 Flash", family: "Gemini" },
  { id: "gemini-2.5-flash-lite", name: "Gemini 2.5 Flash Lite", family: "Gemini" },
];

export const CliToolsView = memo(function CliToolsView({
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

  // Antigravity MITM Proxy state
  const [mitmMode, setMitmMode] = useState<"cloudcode" | "hosts" | "proxy" | "vscode" | "endpoints" | "rollback">("cloudcode");
  const [pingingMitm, setPingingMitm] = useState(false);
  const [mitmPingResult, setMitmPingResult] = useState<{
    ok: boolean;
    count: number;
    sampleModels: string[];
    latencyMs?: number;
    error?: string;
  } | null>(null);

  // Antigravity MITM Model Selection & Remapping state
  const [mitmConfig, setMitmConfig] = useState<AntigravityMitmConfig>({
    enabled: true,
    default_target_model: null,
    model_aliases: {},
  });
  const [loadingMitmConfig, setLoadingMitmConfig] = useState(false);
  const [savingMitmConfig, setSavingMitmConfig] = useState(false);
  const [mitmSaveSuccess, setMitmSaveSuccess] = useState(false);
  const [mitmFilterQuery, setMitmFilterQuery] = useState("");
  const [customAliasSource, setCustomAliasSource] = useState("");
  const [customAliasTarget, setCustomAliasTarget] = useState("");

  useEffect(() => {
    let cancelled = false;
    async function loadMitmConfig() {
      setLoadingMitmConfig(true);
      try {
        const res = await api.getMitmConfig();
        if (!cancelled && res?.config) {
          setMitmConfig({
            enabled: res.config.enabled ?? true,
            default_target_model: res.config.default_target_model ?? null,
            model_aliases: res.config.model_aliases ?? {},
          });
        }
      } catch (err) {
        console.warn("Failed to load Antigravity MITM config", err);
      } finally {
        if (!cancelled) setLoadingMitmConfig(false);
      }
    }
    loadMitmConfig();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleSaveMitmConfig = async () => {
    setSavingMitmConfig(true);
    setMitmSaveSuccess(false);
    try {
      const res = await api.updateMitmConfig({
        enabled: mitmConfig.enabled,
        default_target_model: mitmConfig.default_target_model || null,
        model_aliases: mitmConfig.model_aliases,
      });
      if (res?.config) {
        setMitmConfig({
          enabled: res.config.enabled ?? true,
          default_target_model: res.config.default_target_model ?? null,
          model_aliases: res.config.model_aliases ?? {},
        });
      }
      setMitmSaveSuccess(true);
      setTimeout(() => setMitmSaveSuccess(false), 3000);
    } catch (err) {
      console.error("Failed to save Antigravity MITM config", err);
    } finally {
      setSavingMitmConfig(false);
    }
  };

  const handleSetModelAlias = (sourceId: string, targetId: string) => {
    setMitmConfig((prev) => {
      const nextAliases = { ...prev.model_aliases };
      if (!targetId || targetId === "passthrough") {
        delete nextAliases[sourceId];
      } else {
        nextAliases[sourceId] = targetId;
      }
      return { ...prev, model_aliases: nextAliases };
    });
  };

  const handleAddCustomAlias = () => {
    const src = customAliasSource.trim();
    const dst = customAliasTarget.trim();
    if (!src || !dst) return;
    setMitmConfig((prev) => ({
      ...prev,
      model_aliases: {
        ...prev.model_aliases,
        [src]: dst,
      },
    }));
    setCustomAliasSource("");
    setCustomAliasTarget("");
  };

  const handleRemoveCustomAlias = (key: string) => {
    setMitmConfig((prev) => {
      const nextAliases = { ...prev.model_aliases };
      delete nextAliases[key];
      return { ...prev, model_aliases: nextAliases };
    });
  };

  const handleResetMitmAliases = () => {
    setMitmConfig((prev) => ({
      ...prev,
      default_target_model: null,
      model_aliases: {},
    }));
  };

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

  const standardModelIds = useMemo(
    () => new Set(ANTIGRAVITY_INTERCEPTED_MODELS.map((m) => m.id)),
    [],
  );

  const filteredInterceptedModels = useMemo(() => {
    if (!mitmFilterQuery.trim()) return ANTIGRAVITY_INTERCEPTED_MODELS;
    const q = mitmFilterQuery.toLowerCase();
    return ANTIGRAVITY_INTERCEPTED_MODELS.filter(
      (m) =>
        m.name.toLowerCase().includes(q) ||
        m.id.toLowerCase().includes(q) ||
        m.family.toLowerCase().includes(q),
    );
  }, [mitmFilterQuery]);

  const customAliasesList = useMemo(() => {
    return Object.entries(mitmConfig.model_aliases).filter(
      ([key]) => !standardModelIds.has(key),
    );
  }, [mitmConfig.model_aliases, standardModelIds]);

  const targetModelOptions = useMemo(() => {
    const combosGroup = comboNames.map((name) => ({
      value: `combo/${name}`,
      label: `Combo: ${name}`,
    }));

    const ovhGroup = models
      .filter((m) => m.owned_by === "ovh")
      .map((m) => ({
        value: `ovh/${m.id}`,
        label: `OVH: ${m.id}`,
      }));

    const ocGroup = models
      .filter((m) => m.owned_by === "opencode" || m.owned_by === "opencode-free")
      .map((m) => ({
        value: `oc/${m.id}`,
        label: `OpenCode: ${m.id}`,
      }));

    const agGroup = models
      .filter((m) => m.owned_by === "antigravity")
      .map((m) => ({
        value: `antigravity/${m.id}`,
        label: `Antigravity: ${m.id}`,
      }));

    return { combosGroup, ovhGroup, ocGroup, agGroup };
  }, [comboNames, models]);

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

  async function pingAntigravityGateway() {
    setPingingMitm(true);
    const start = Date.now();
    try {
      const res = await fetch(`${origin}/v1internal:fetchAvailableModels`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(effectiveKey ? { Authorization: `Bearer ${effectiveKey}` } : {}),
        },
        body: JSON.stringify({}),
      });
      const lat = Date.now() - start;
      if (!res.ok) {
        throw new Error(`HTTP ${res.status} ${res.statusText}`);
      }
      const data = await res.json();
      const modelKeys = Object.keys(data.models || {});
      setMitmPingResult({
        ok: true,
        count: modelKeys.length,
        sampleModels: modelKeys.slice(0, 6),
        latencyMs: lat,
      });
    } catch (err: any) {
      setMitmPingResult({
        ok: false,
        count: 0,
        sampleModels: [],
        error: err?.message || String(err),
      });
    } finally {
      setPingingMitm(false);
    }
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
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
        {/* OpenCode Card */}
        <button
          type="button"
          onClick={() => setActiveTab("opencode")}
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
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
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
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
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
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
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
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

        {/* Antigravity MITM Proxy Card */}
        <button
          type="button"
          onClick={() => setActiveTab("mitm")}
          className={`flex flex-col justify-between rounded-xl border p-3.5 text-left transition-colors ${
            activeTab === "mitm"
              ? "border-[var(--accent)] bg-[var(--bg-surface)] shadow-md ring-1 ring-[var(--accent)]/30"
              : "border-[var(--border-subtle)] bg-[var(--bg-surface)]/70 hover:border-[var(--border-strong)] hover:bg-[var(--bg-surface)]"
          }`}
        >
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2.5">
              <ProviderIcon src="/providers/antigravity.png" name="Antigravity MITM" color="#4285F4" size={26} />
              <span className="text-xs font-semibold text-[var(--text-primary)]">Antigravity MITM</span>
            </div>
            <StatusPill tone="success" dot>Proxy Wire</StatusPill>
          </div>
          <div className="mt-3 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
            <span className="truncate">Google Cloud Code PA</span>
            <span className="font-mono text-[10px] text-[#4285F4]">Transparent</span>
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

        {/* ========================================================================= */}
        {/* TAB 5: ANTIGRAVITY MITM PROXY (GOOGLE CLOUD CODE & IDE INTERCEPTION) */}
        {/* ========================================================================= */}
        {activeTab === "mitm" && (
          <div className="space-y-6">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--border-subtle)] pb-4">
              <div className="flex items-center gap-3">
                <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-blue-500/15 text-blue-400 border border-blue-500/25">
                  <Network className="h-5 w-5" />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <h2 className="text-sm font-bold text-[var(--text-primary)]">Antigravity MITM Proxy</h2>
                    <span className="rounded-full bg-blue-500/10 px-2 py-0.5 text-[10px] font-semibold text-blue-400 border border-blue-500/20">
                      Transparent Interception
                    </span>
                  </div>
                  <p className="text-[11px] text-[var(--text-muted)] mt-0.5">
                    Capture and route Google Cloud Code PA traffic without modifying closed-source IDE binaries or extensions.
                  </p>
                </div>
              </div>

              {/* Ping Test Button */}
              <div className="flex items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={pingAntigravityGateway}
                  disabled={pingingMitm}
                  className="gap-1.5"
                >
                  <Radio className={`h-3.5 w-3.5 text-blue-400 ${pingingMitm ? "animate-pulse" : ""}`} />
                  <span>{pingingMitm ? "Probing..." : "Test Antigravity Gateway"}</span>
                </Button>
              </div>
            </div>

            {/* Ping Result Banner */}
            {mitmPingResult && (
              <div
                className={`flex flex-wrap items-center justify-between gap-2 rounded-xl border p-3 text-xs ${
                  mitmPingResult.ok
                    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                    : "border-rose-500/30 bg-rose-500/10 text-rose-300"
                }`}
              >
                <div className="flex items-center gap-2">
                  <span className={`h-2 w-2 rounded-full ${mitmPingResult.ok ? "bg-emerald-400 animate-pulse" : "bg-rose-400"}`} />
                  <span className="font-medium">
                    {mitmPingResult.ok
                      ? `Gateway Verified (${mitmPingResult.latencyMs}ms) — ${mitmPingResult.count} Antigravity models discovered`
                      : `Gateway Probe Error: ${mitmPingResult.error}`}
                  </span>
                </div>
                {mitmPingResult.sampleModels.length > 0 && (
                  <div className="flex flex-wrap items-center gap-1 font-mono text-[10px]">
                    {mitmPingResult.sampleModels.map((m) => (
                      <span key={m} className="rounded bg-black/30 px-1.5 py-0.5 border border-emerald-500/20">
                        {m}
                      </span>
                    ))}
                    {mitmPingResult.count > mitmPingResult.sampleModels.length && (
                      <span className="opacity-70">+{mitmPingResult.count - mitmPingResult.sampleModels.length} more</span>
                    )}
                  </div>
                )}
              </div>
            )}

            {/* Mode Selector Pill Buttons */}
            <div className="flex flex-wrap gap-2 rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-2 shadow-sm">
              <button
                type="button"
                onClick={() => setMitmMode("cloudcode")}
                className={`flex items-center gap-2 rounded-xl px-3.5 py-2 text-xs font-semibold transition-all ${
                  mitmMode === "cloudcode"
                    ? "bg-[var(--accent)] text-white shadow-md ring-1 ring-[var(--accent)]/50"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-surface)]"
                }`}
              >
                <Zap className={`h-3.5 w-3.5 ${mitmMode === "cloudcode" ? "text-amber-300" : "text-amber-400"}`} />
                <span>1. CLOUD_CODE_URL</span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[9px] font-bold ${
                    mitmMode === "cloudcode"
                      ? "bg-white/20 text-white"
                      : "bg-emerald-500/10 text-emerald-400 border border-emerald-500/20"
                  }`}
                >
                  Recommended · Zero Root
                </span>
              </button>

              <button
                type="button"
                onClick={() => setMitmMode("hosts")}
                className={`flex items-center gap-2 rounded-xl px-3.5 py-2 text-xs font-semibold transition-all ${
                  mitmMode === "hosts"
                    ? "bg-[var(--accent)] text-white shadow-md ring-1 ring-[var(--accent)]/50"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-surface)]"
                }`}
              >
                <Network className="h-3.5 w-3.5 text-sky-400" />
                <span>2. Hosts / DNS Loopback</span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[9px] font-bold ${
                    mitmMode === "hosts"
                      ? "bg-white/20 text-white"
                      : "bg-sky-500/10 text-sky-400 border border-sky-500/20"
                  }`}
                >
                  9Router Architecture
                </span>
              </button>

              <button
                type="button"
                onClick={() => setMitmMode("proxy")}
                className={`flex items-center gap-2 rounded-xl px-3.5 py-2 text-xs font-semibold transition-all ${
                  mitmMode === "proxy"
                    ? "bg-[var(--accent)] text-white shadow-md ring-1 ring-[var(--accent)]/50"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-surface)]"
                }`}
              >
                <Globe className="h-3.5 w-3.5 text-blue-400" />
                <span>3. HTTP_PROXY</span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[9px] font-bold ${
                    mitmMode === "proxy"
                      ? "bg-white/20 text-white"
                      : "bg-blue-500/10 text-blue-400 border border-blue-500/20"
                  }`}
                >
                  Terminal &amp; CLI
                </span>
              </button>

              <button
                type="button"
                onClick={() => setMitmMode("vscode")}
                className={`flex items-center gap-2 rounded-xl px-3.5 py-2 text-xs font-semibold transition-all ${
                  mitmMode === "vscode"
                    ? "bg-[var(--accent)] text-white shadow-md ring-1 ring-[var(--accent)]/50"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-surface)]"
                }`}
              >
                <FileCode2 className="h-3.5 w-3.5 text-purple-400" />
                <span>4. VS Code Settings</span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[9px] font-bold ${
                    mitmMode === "vscode"
                      ? "bg-white/20 text-white"
                      : "bg-purple-500/10 text-purple-400 border border-purple-500/20"
                  }`}
                >
                  settings.json
                </span>
              </button>

              <button
                type="button"
                onClick={() => setMitmMode("endpoints")}
                className={`flex items-center gap-2 rounded-xl px-3.5 py-2 text-xs font-semibold transition-all ${
                  mitmMode === "endpoints"
                    ? "bg-[var(--accent)] text-white shadow-md ring-1 ring-[var(--accent)]/50"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-surface)]"
                }`}
              >
                <Code2 className="h-3.5 w-3.5 text-cyan-400" />
                <span>5. Direct Endpoints</span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[9px] font-bold ${
                    mitmMode === "endpoints"
                      ? "bg-white/20 text-white"
                      : "bg-neutral-500/10 text-neutral-400 border border-neutral-500/20"
                  }`}
                >
                  REST &amp; SSE Wire
                </span>
              </button>

              <button
                type="button"
                onClick={() => setMitmMode("rollback")}
                className={`flex items-center gap-2 rounded-xl px-3.5 py-2 text-xs font-semibold transition-all ${
                  mitmMode === "rollback"
                    ? "bg-rose-600 text-white shadow-md ring-1 ring-rose-400/50"
                    : "text-rose-400 hover:text-rose-300 hover:bg-rose-500/10 border border-rose-500/25"
                }`}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                <span>6. Rollback &amp; Emergency Reset</span>
                <span
                  className={`rounded px-1.5 py-0.5 text-[9px] font-bold ${
                    mitmMode === "rollback" ? "bg-white/25 text-white" : "bg-rose-500/20 text-rose-300"
                  }`}
                >
                  Fix Port 443 &amp; Restore Google
                </span>
              </button>
            </div>

            {/* MODE 1: CLOUD_CODE_URL */}
            {mitmMode === "cloudcode" && (
              <div className="space-y-4">
                {/* Setup Box */}
                <div className="rounded-2xl border border-emerald-500/30 bg-emerald-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-emerald-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <Zap className="h-4 w-4 text-emerald-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Setup: Native CLOUD_CODE_URL Override
                      </span>
                    </div>
                    <span className="rounded-full bg-emerald-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-emerald-400 border border-emerald-500/20">
                      Zero Root Required &middot; No Port 443 Conflicts &middot; Official Google Hook
                    </span>
                  </div>

                  <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                    Google Antigravity and Cloud Code's underlying language server (<code className="text-emerald-400 font-mono">language_server.exe</code>) natively checks the <code className="text-amber-400 font-mono">CLOUD_CODE_URL</code> environment variable. Setting this points internal IDE completions and streaming generation straight to Tesie on port <strong>20129</strong> without touching system files or certificates.
                  </p>

                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                    {/* PowerShell Terminal Session */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-[var(--text-primary)]">Windows PowerShell (Launch IDE from Terminal):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-cloudcode-pwsh", `$env:CLOUD_CODE_URL="${origin}"\n& "$env:LOCALAPPDATA\\Programs\\antigravity\\Antigravity.exe" .`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-cloudcode-pwsh" ? "Copied" : "Copy snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-emerald-400 overflow-x-auto leading-relaxed">
{`$env:CLOUD_CODE_URL="${origin}"
& "$env:LOCALAPPDATA\\Programs\\antigravity\\Antigravity.exe" .`}
                      </pre>
                      <p className="text-[10px] text-[var(--text-muted)]">
                        Launches Antigravity IDE with the hook (or use <code className="text-emerald-400">agy</code> for Antigravity CLI).
                      </p>
                    </div>

                    {/* Permanent Windows User Env */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-[var(--text-primary)]">Windows (System-Wide User Variable — Recommended):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-cloudcode-perm", `[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", "${origin}", "User")`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-cloudcode-perm" ? "Copied" : "Copy command"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-cyan-300 overflow-x-auto leading-relaxed">
{`[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", "${origin}", "User")`}
                      </pre>
                      <p className="text-[10px] text-[var(--text-muted)]">
                        Set once, then open Antigravity normally from your Start Menu or Taskbar!
                      </p>
                    </div>
                  </div>

                  {/* macOS / Linux */}
                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                    <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                      <span className="font-semibold text-[var(--text-primary)]">macOS &amp; Linux (Bash / Zsh):</span>
                      <button
                        type="button"
                        onClick={() => copyText("mitm-cloudcode-bash", `export CLOUD_CODE_URL="${origin}"\nantigravity .`)}
                        className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                      >
                        <span>{copiedId === "mitm-cloudcode-bash" ? "Copied" : "Copy snippet"}</span>
                        <Copy className="h-3 w-3" />
                      </button>
                    </div>
                    <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-[var(--text-primary)] overflow-x-auto leading-relaxed">
{`export CLOUD_CODE_URL="${origin}"
antigravity .`}
                    </pre>
                  </div>
                </div>

                {/* Rollback & Revert Card for CLOUD_CODE_URL */}
                <div className="rounded-2xl border border-amber-500/30 bg-amber-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-amber-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <RotateCcw className="h-4 w-4 text-amber-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Rollback Tutorial: Reverting CLOUD_CODE_URL (Restore Direct Google)
                      </span>
                    </div>
                    <span className="rounded-full bg-amber-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-amber-400 border border-amber-500/20">
                      1-Click Revert &middot; Restores Direct Google API
                    </span>
                  </div>

                  <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                    Whenever you want to disconnect Tesie and let Antigravity / Cloud Code connect directly back to Google's production servers, simply unset <code className="text-amber-400 font-mono">CLOUD_CODE_URL</code>:
                  </p>

                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                    {/* Windows PowerShell Revert */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-amber-300">Windows PowerShell (Clear Both Session &amp; User):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-revert-cloudcode-pwsh", `Remove-Item env:CLOUD_CODE_URL -ErrorAction SilentlyContinue\n[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", $null, "User")`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-revert-cloudcode-pwsh" ? "Copied" : "Copy revert snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-amber-300 overflow-x-auto leading-relaxed">
{`Remove-Item env:CLOUD_CODE_URL -ErrorAction SilentlyContinue
[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", $null, "User")`}
                      </pre>
                    </div>

                    {/* macOS / Linux Revert */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-amber-300">macOS &amp; Linux (Bash / Zsh):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-revert-cloudcode-bash", `unset CLOUD_CODE_URL`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-revert-cloudcode-bash" ? "Copied" : "Copy revert snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-amber-300 overflow-x-auto leading-relaxed">
{`unset CLOUD_CODE_URL`}
                      </pre>
                      <p className="text-[10px] text-[var(--text-muted)]">
                        If you added this line to <code className="text-amber-400">~/.zshrc</code> or <code className="text-amber-400">~/.bashrc</code>, delete that line and run <code className="text-amber-400">source ~/.zshrc</code>.
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* MODE 2: HOSTS / DNS LOOPBACK (9ROUTER STYLE) */}
            {mitmMode === "hosts" && (
              <div className="space-y-4">
                {/* 9Router Architecture Analysis Box */}
                <div className="rounded-2xl border border-sky-500/30 bg-sky-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-sky-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <Network className="h-4 w-4 text-sky-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        9Router "Hosts / DNS Loopback" Architecture &amp; Diagnosis
                      </span>
                    </div>
                    <span className="rounded-full bg-sky-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-sky-400 border border-sky-500/20">
                      Requires Administrator &middot; Port 443 Required &middot; Custom Root CA
                    </span>
                  </div>

                  <div className="space-y-2 text-xs text-[var(--text-secondary)] leading-relaxed">
                    <p>
                      In <code className="text-sky-400 font-mono">9+1Router</code>, MITM interception works by taking over port 443 with a 3-part privileged mechanism:
                    </p>
                    <ol className="list-decimal pl-5 space-y-1 text-[11px] text-[var(--text-muted)]">
                      <li>
                        <strong>Hosts File Redirect:</strong> Adds <code className="text-sky-300 font-mono">127.0.0.1 daily-cloudcode-pa.googleapis.com</code> and <code className="text-sky-300 font-mono">127.0.0.1 cloudcode-pa.googleapis.com</code> to <code className="text-sky-300 font-mono">C:\Windows\System32\drivers\etc\hosts</code>.
                      </li>
                      <li>
                        <strong>Port 443 HTTPS Server:</strong> A Node.js TLS server (<code className="text-sky-300 font-mono">server.js</code>) binds directly to port <strong>443</strong> with dynamic SNI certificate generation.
                      </li>
                      <li>
                        <strong>Root CA Installation:</strong> Installs a self-signed <code className="text-sky-300 font-mono">9Router MITM Root CA</code> into the Windows Trusted Root Certification Authorities store via <code className="text-sky-300 font-mono">certutil -addstore Root rootCA.crt</code> and sets <code className="text-sky-300 font-mono">NODE_EXTRA_CA_CERTS</code>.
                      </li>
                    </ol>
                  </div>

                  {/* Why 502/connectex happens Alert */}
                  <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 p-3.5 space-y-2 text-xs">
                    <div className="flex items-center gap-2 font-bold text-amber-300">
                      <ShieldAlert className="h-4 w-4" />
                      <span>Why "connectex: dial tcp 127.0.0.1:443 refused" Occurs:</span>
                    </div>
                    <p className="text-[11px] text-amber-200/90 leading-relaxed">
                      If 9Router's server is stopped, killed, or fails to bind to port 443 (e.g. port conflict with IIS, VMware, or unprivileged process), the <code className="font-mono">hosts</code> file continues redirecting all Antigravity traffic to <code className="font-mono">127.0.0.1:443</code> where <em>nothing is listening</em>. The Windows kernel actively rejects the TCP handshake with code <code className="font-mono">10061 (WSAECONNREFUSED)</code>.
                    </p>
                    <p className="text-[11px] text-emerald-300 leading-relaxed">
                      💡 <strong>Why Tesie uses CLOUD_CODE_URL instead:</strong> Google Cloud Code natively provides the <code className="font-mono text-emerald-400">CLOUD_CODE_URL</code> hook. By directing traffic straight to Tesie on port <strong>20129</strong>, Tesie eliminates the need to edit hosts files, eliminates port 443 conflicts, and requires zero root/admin certificates.
                    </p>
                  </div>

                  {/* Manual Hosts Setup Snippet if user wants to use 9Router style */}
                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 pt-2">
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-[var(--text-primary)]">Check Windows Hosts File:</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-hosts-check", `Get-Content "$env:windir\\System32\\drivers\\etc\\hosts" | Select-String 'cloudcode-pa'`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-hosts-check" ? "Copied" : "Copy command"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-sky-300 overflow-x-auto leading-relaxed">
{`Get-Content "$env:windir\\System32\\drivers\\etc\\hosts" | Select-String 'cloudcode-pa'`}
                      </pre>
                      <p className="text-[10px] text-[var(--text-muted)]">
                        If this returns an entry, Antigravity is trapped redirecting to 127.0.0.1:443.
                      </p>
                    </div>

                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-emerald-400">Remove Hosts Loopback (Instant Fix):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-hosts-clean", `$h = "$env:windir\\System32\\drivers\\etc\\hosts"\n(Get-Content $h) | Where-Object { $_ -notmatch 'cloudcode-pa' } | Set-Content -Path $h -Force\nipconfig /flushdns`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-hosts-clean" ? "Copied" : "Copy clean snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-emerald-400 overflow-x-auto leading-relaxed">
{`$h = "$env:windir\\System32\\drivers\\etc\\hosts"
(Get-Content $h) | Where-Object { $_ -notmatch 'cloudcode-pa' } | Set-Content -Path $h -Force
ipconfig /flushdns`}
                      </pre>
                      <p className="text-[10px] text-[var(--text-muted)]">
                        Run in Administrator PowerShell to restore direct Google Cloud Code connections.
                      </p>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* MODE 3: HTTP_PROXY */}
            {mitmMode === "proxy" && (
              <div className="space-y-4">
                {/* Setup Box */}
                <div className="rounded-2xl border border-blue-500/30 bg-blue-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-blue-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <Globe className="h-4 w-4 text-blue-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Setup: Standard HTTP_PROXY &amp; HTTPS_PROXY
                      </span>
                    </div>
                    <span className="rounded-full bg-blue-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-blue-400 border border-blue-500/20">
                      Terminal Shells &middot; Child Processes &middot; CLI Wrappers
                    </span>
                  </div>

                  <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                    Standard Unix and Windows tools (including Node.js, curl, and python) respect the standard <code className="text-blue-400 font-mono">HTTP_PROXY</code> and <code className="text-blue-400 font-mono">HTTPS_PROXY</code> environment variables to route their HTTP traffic through the Tesie gateway.
                  </p>

                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                    {/* Windows PowerShell */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-[var(--text-primary)]">Windows PowerShell:</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-proxy-pwsh", `$env:HTTP_PROXY="${origin}"; $env:HTTPS_PROXY="${origin}"`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-proxy-pwsh" ? "Copied" : "Copy snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-blue-400 overflow-x-auto leading-relaxed">
{`$env:HTTP_PROXY="${origin}"
$env:HTTPS_PROXY="${origin}"`}
                      </pre>
                    </div>

                    {/* macOS / Linux */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-[var(--text-primary)]">macOS &amp; Linux (Bash / Zsh):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-proxy-bash", `export HTTP_PROXY="${origin}"\nexport HTTPS_PROXY="${origin}"`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-proxy-bash" ? "Copied" : "Copy snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-blue-400 overflow-x-auto leading-relaxed">
{`export HTTP_PROXY="${origin}"
export HTTPS_PROXY="${origin}"`}
                      </pre>
                    </div>
                  </div>
                </div>

                {/* Rollback Box */}
                <div className="rounded-2xl border border-amber-500/30 bg-amber-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-amber-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <RotateCcw className="h-4 w-4 text-amber-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Rollback Tutorial: Reverting HTTP_PROXY &amp; HTTPS_PROXY
                      </span>
                    </div>
                    <span className="rounded-full bg-amber-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-amber-400 border border-amber-500/20">
                      Instant Revert
                    </span>
                  </div>

                  <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                    Unset both variables so terminal CLIs stop routing through the local proxy:
                  </p>

                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                    {/* Windows Revert */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-amber-300">Windows PowerShell (Clear Session &amp; User):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-revert-proxy-pwsh", `Remove-Item env:HTTP_PROXY -ErrorAction SilentlyContinue\nRemove-Item env:HTTPS_PROXY -ErrorAction SilentlyContinue\n[Environment]::SetEnvironmentVariable("HTTP_PROXY", $null, "User")\n[Environment]::SetEnvironmentVariable("HTTPS_PROXY", $null, "User")`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-revert-proxy-pwsh" ? "Copied" : "Copy revert snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-amber-300 overflow-x-auto leading-relaxed">
{`Remove-Item env:HTTP_PROXY -ErrorAction SilentlyContinue
Remove-Item env:HTTPS_PROXY -ErrorAction SilentlyContinue
[Environment]::SetEnvironmentVariable("HTTP_PROXY", $null, "User")
[Environment]::SetEnvironmentVariable("HTTPS_PROXY", $null, "User")`}
                      </pre>
                    </div>

                    {/* macOS / Linux Revert */}
                    <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-2">
                      <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                        <span className="font-semibold text-amber-300">macOS &amp; Linux (Bash / Zsh):</span>
                        <button
                          type="button"
                          onClick={() => copyText("mitm-revert-proxy-bash", `unset HTTP_PROXY HTTPS_PROXY`)}
                          className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                        >
                          <span>{copiedId === "mitm-revert-proxy-bash" ? "Copied" : "Copy revert snippet"}</span>
                          <Copy className="h-3 w-3" />
                        </button>
                      </div>
                      <pre className="rounded-lg border border-[var(--border-subtle)] bg-black/40 p-2.5 font-mono text-xs text-amber-300 overflow-x-auto leading-relaxed">
{`unset HTTP_PROXY HTTPS_PROXY`}
                      </pre>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* MODE 3: VS CODE SETTINGS */}
            {mitmMode === "vscode" && (
              <div className="space-y-4">
                {/* Setup Box */}
                <div className="rounded-2xl border border-purple-500/30 bg-purple-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-purple-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <FileCode2 className="h-4 w-4 text-purple-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Setup: VS Code settings.json Proxy Routing
                      </span>
                    </div>
                    <span className="rounded-full bg-purple-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-purple-400 border border-purple-500/20">
                      .vscode/settings.json or Global Settings
                    </span>
                  </div>

                  <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                    Configure VS Code to automatically route all extension and telemetry network traffic through Tesie:
                  </p>

                  <div className="relative">
                    <pre className="rounded-xl border border-[var(--border-subtle)] bg-black/40 p-3.5 font-mono text-xs text-purple-300 overflow-x-auto leading-relaxed">
{`{
  "http.proxy": "${origin}",
  "http.proxySupport": "on",
  "http.proxyStrictSSL": false
}`}
                    </pre>
                    <button
                      type="button"
                      onClick={() => copyText("mitm-vscode-setup", `{\n  "http.proxy": "${origin}",\n  "http.proxySupport": "on",\n  "http.proxyStrictSSL": false\n}`)}
                      className="absolute top-2.5 right-2.5 rounded-lg bg-[var(--bg-surface)] px-2.5 py-1 text-[11px] text-[var(--accent)] border border-[var(--border-subtle)] hover:border-[var(--border-strong)] flex items-center gap-1 font-mono shadow-sm"
                    >
                      <Copy className="h-3 w-3" />
                      <span>{copiedId === "mitm-vscode-setup" ? "Copied" : "Copy JSON"}</span>
                    </button>
                  </div>
                </div>

                {/* Rollback Box */}
                <div className="rounded-2xl border border-amber-500/30 bg-amber-500/[0.03] p-5 space-y-4 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-amber-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <RotateCcw className="h-4 w-4 text-amber-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Rollback Tutorial: Reverting VS Code settings.json
                      </span>
                    </div>
                    <span className="rounded-full bg-amber-500/10 px-2.5 py-0.5 font-mono text-[10px] font-semibold text-amber-400 border border-amber-500/20">
                      Restore Vanilla VS Code
                    </span>
                  </div>

                  <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                    To revert back to direct internet connections inside VS Code, open <code className="text-amber-400 font-mono">settings.json</code> (press <kbd className="rounded bg-black/40 px-1 py-0.5 border border-white/10 font-mono text-[10px]">Ctrl+Shift+P</kbd> &rarr; <em>Preferences: Open User Settings (JSON)</em>) and either delete those 3 keys or replace them with defaults:
                  </p>

                  <div className="relative">
                    <pre className="rounded-xl border border-[var(--border-subtle)] bg-black/40 p-3.5 font-mono text-xs text-amber-300 overflow-x-auto leading-relaxed">
{`{
  "http.proxy": "",
  "http.proxySupport": "override",
  "http.proxyStrictSSL": true
}`}
                    </pre>
                    <button
                      type="button"
                      onClick={() => copyText("mitm-vscode-revert", `{\n  "http.proxy": "",\n  "http.proxySupport": "override",\n  "http.proxyStrictSSL": true\n}`)}
                      className="absolute top-2.5 right-2.5 rounded-lg bg-[var(--bg-surface)] px-2.5 py-1 text-[11px] text-amber-400 border border-amber-500/30 hover:border-amber-500 flex items-center gap-1 font-mono shadow-sm"
                    >
                      <Copy className="h-3 w-3" />
                      <span>{copiedId === "mitm-vscode-revert" ? "Copied" : "Copy Revert JSON"}</span>
                    </button>
                  </div>
                  <p className="text-[11px] text-[var(--text-muted)]">
                    Tip: Press <kbd className="rounded bg-black/40 px-1 py-0.5 border border-white/10 font-mono text-[10px]">Ctrl+Shift+P</kbd> &rarr; <em>Developer: Reload Window</em> to reload VS Code with clean network settings.
                  </p>
                </div>
              </div>
            )}

            {/* MODE 4: DIRECT ENDPOINTS */}
            {mitmMode === "endpoints" && (
              <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-5 space-y-4 shadow-sm">
                <div className="flex items-center justify-between border-b border-[var(--border-subtle)] pb-3">
                  <div className="flex items-center gap-2">
                    <Code2 className="h-4 w-4 text-cyan-400" />
                    <span className="text-xs font-bold text-[var(--text-primary)]">
                      Direct Native Antigravity Endpoints
                    </span>
                  </div>
                  <span className="font-mono text-[10px] text-blue-400 bg-blue-500/10 px-2 py-0.5 rounded border border-blue-500/20">
                    REST / SSE Wire
                  </span>
                </div>
                <p className="text-xs text-[var(--text-secondary)] leading-relaxed">
                  Tesie replicates Google Cloud Code PA's internal gRPC/HTTP endpoints with zero authentication hurdles, making them available for custom CLI scripts, plugins, or extensions:
                </p>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3 space-y-1">
                    <span className="text-[10px] uppercase font-bold text-[var(--text-muted)]">SSE Streaming Generation</span>
                    <button
                      type="button"
                      onClick={() => copyText("endpoint-stream", `${origin}/v1internal:streamGenerateContent?alt=sse`)}
                      className="w-full text-left font-mono text-xs text-emerald-400 hover:underline flex items-center justify-between"
                    >
                      <span className="truncate">POST /v1internal:streamGenerateContent?alt=sse</span>
                      <Copy className="h-3 w-3 shrink-0 ml-1 text-[var(--text-muted)]" />
                    </button>
                  </div>

                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3 space-y-1">
                    <span className="text-[10px] uppercase font-bold text-[var(--text-muted)]">Model Catalog Discovery</span>
                    <button
                      type="button"
                      onClick={() => copyText("endpoint-models", `${origin}/v1internal:fetchAvailableModels`)}
                      className="w-full text-left font-mono text-xs text-blue-400 hover:underline flex items-center justify-between"
                    >
                      <span className="truncate">POST /v1internal:fetchAvailableModels</span>
                      <Copy className="h-3 w-3 shrink-0 ml-1 text-[var(--text-muted)]" />
                    </button>
                  </div>
                </div>

                <div className="space-y-1.5 pt-2">
                  <div className="flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                    <span className="font-semibold text-[var(--text-primary)]">Quick Curl Test:</span>
                    <button
                      type="button"
                      onClick={() =>
                        copyText(
                          "endpoint-curl",
                          `curl -X POST "${origin}/v1internal:fetchAvailableModels" \\\n  -H "Content-Type: application/json" \\\n  -d '{}'`
                        )
                      }
                      className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                    >
                      <span>{copiedId === "endpoint-curl" ? "Copied" : "Copy curl test"}</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <pre className="rounded-xl border border-[var(--border-subtle)] bg-black/40 p-3 font-mono text-xs text-[var(--text-primary)] overflow-x-auto leading-relaxed">
{`curl -X POST "${origin}/v1internal:fetchAvailableModels" \\
  -H "Content-Type: application/json" \\
  -d '{}'`}
                  </pre>
                </div>
              </div>
            )}

            {/* MODE 5: FULL ROLLBACK & EMERGENCY RESET CENTER */}
            {mitmMode === "rollback" && (
              <div className="space-y-4">
                {/* Critical Diagnosis Card */}
                <div className="rounded-2xl border border-rose-500/40 bg-rose-500/[0.05] p-5 space-y-3.5 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2 border-b border-rose-500/20 pb-3">
                    <div className="flex items-center gap-2">
                      <ShieldAlert className="h-5 w-5 text-rose-400" />
                      <span className="text-sm font-bold text-rose-200">
                        Troubleshooting &amp; Emergency Restoration Center
                      </span>
                    </div>
                    <span className="rounded-full bg-rose-500/20 px-2.5 py-0.5 font-mono text-[10px] font-bold text-rose-300 border border-rose-500/30">
                      Fix: connectex dial tcp 127.0.0.1:443 refused
                    </span>
                  </div>

                  <div className="space-y-2 text-xs text-rose-200/90 leading-relaxed">
                    <p>
                      <strong>Why did this error happen?</strong> If you previously added <code className="text-rose-300 font-mono bg-black/40 px-1.5 py-0.5 rounded border border-rose-500/30">127.0.0.1 daily-cloudcode-pa.googleapis.com</code> to your system <code className="text-rose-300 font-mono">hosts</code> file, all HTTPS requests to Google Cloud Code PA are redirected to your local machine on port <strong>443</strong>.
                    </p>
                    <p>
                      Because Tesie runs as an unprivileged, safe local gateway on port <strong>20129</strong> (and does not bind to root port 443), Windows/macOS kernel immediately rejects the connection with:
                    </p>
                    <pre className="p-2.5 rounded-lg border border-rose-500/30 bg-black/50 font-mono text-[11px] text-rose-300 overflow-x-auto">
{`dial tcp 127.0.0.1:443: connectex: No connection could be made because the target machine actively refused it.`}
                    </pre>
                  </div>
                </div>

                {/* 1-Click Automated PowerShell Master Script */}
                <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-5 space-y-3.5 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <Terminal className="h-4 w-4 text-emerald-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        Windows: 1-Click Automated Rollback Script (PowerShell Admin)
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() =>
                        copyText(
                          "rollback-master-pwsh",
                          `# 1. Clean hosts file loopbacks\n$hosts = "$env:windir\\System32\\drivers\\etc\\hosts"\nif (Test-Path $hosts) {\n  $clean = (Get-Content $hosts) | Where-Object { $_ -notmatch 'cloudcode-pa\\.googleapis\\.com' }\n  Set-Content -Path $hosts -Value $clean -Force\n  Write-Host "[✓] Cleaned $hosts" -ForegroundColor Green\n}\n\n# 2. Clear process and user environment variables\nRemove-Item env:CLOUD_CODE_URL -ErrorAction SilentlyContinue\nRemove-Item env:HTTP_PROXY -ErrorAction SilentlyContinue\nRemove-Item env:HTTPS_PROXY -ErrorAction SilentlyContinue\n[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", $null, "User")\n[Environment]::SetEnvironmentVariable("HTTP_PROXY", $null, "User")\n[Environment]::SetEnvironmentVariable("HTTPS_PROXY", $null, "User")\nWrite-Host "[✓] Cleared CLOUD_CODE_URL and HTTP_PROXY environment variables" -ForegroundColor Green\n\n# 3. Flush DNS resolver cache\nClear-DnsClientCache\nipconfig /flushdns\nWrite-Host "[✓] Flushed Windows DNS Cache" -ForegroundColor Green\n\n# 4. Verify Google Cloud Code PA connection on port 443\nWrite-Host "Verifying direct connection to Google Cloud Code PA..." -ForegroundColor Cyan\nTest-NetConnection -ComputerName daily-cloudcode-pa.googleapis.com -Port 443`
                        )
                      }
                      className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                    >
                      <span>{copiedId === "rollback-master-pwsh" ? "Copied" : "Copy Full Script"}</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <p className="text-xs text-[var(--text-secondary)]">
                    Open PowerShell as <strong>Administrator</strong> and paste this script. It automatically cleans your hosts file, unsets all proxy environment variables, flushes Windows DNS, and verifies direct connection to Google:
                  </p>
                  <pre className="rounded-xl border border-[var(--border-subtle)] bg-black/40 p-3.5 font-mono text-xs text-emerald-400 overflow-x-auto leading-relaxed">
{`# 1. Clean hosts file loopbacks
$hosts = "$env:windir\\System32\\drivers\\etc\\hosts"
if (Test-Path $hosts) {
  $clean = (Get-Content $hosts) | Where-Object { $_ -notmatch 'cloudcode-pa\\.googleapis\\.com' }
  Set-Content -Path $hosts -Value $clean -Force
  Write-Host "[✓] Cleaned $hosts" -ForegroundColor Green
}

# 2. Clear process and user environment variables
Remove-Item env:CLOUD_CODE_URL -ErrorAction SilentlyContinue
Remove-Item env:HTTP_PROXY -ErrorAction SilentlyContinue
Remove-Item env:HTTPS_PROXY -ErrorAction SilentlyContinue
[Environment]::SetEnvironmentVariable("CLOUD_CODE_URL", $null, "User")
[Environment]::SetEnvironmentVariable("HTTP_PROXY", $null, "User")
[Environment]::SetEnvironmentVariable("HTTPS_PROXY", $null, "User")
Write-Host "[✓] Cleared CLOUD_CODE_URL and HTTP_PROXY environment variables" -ForegroundColor Green

# 3. Flush DNS resolver cache
Clear-DnsClientCache
ipconfig /flushdns
Write-Host "[✓] Flushed Windows DNS Cache" -ForegroundColor Green

# 4. Verify Google Cloud Code PA connection on port 443
Write-Host "Verifying direct connection to Google Cloud Code PA..." -ForegroundColor Cyan
Test-NetConnection -ComputerName daily-cloudcode-pa.googleapis.com -Port 443`}
                  </pre>
                </div>

                {/* macOS & Linux Master Rollback Script */}
                <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-5 space-y-3.5 shadow-sm">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <Terminal className="h-4 w-4 text-cyan-400" />
                      <span className="text-xs font-bold text-[var(--text-primary)]">
                        macOS &amp; Linux: 1-Click Automated Rollback Script (Terminal)
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() =>
                        copyText(
                          "rollback-master-bash",
                          `sudo sed -i.bak '/cloudcode-pa\\.googleapis\\.com/d' /etc/hosts\necho "[✓] Cleaned /etc/hosts"\nunset CLOUD_CODE_URL HTTP_PROXY HTTPS_PROXY\necho "[✓] Unset environment variables"\nif [[ "$OSTYPE" == "darwin"* ]]; then\n  sudo dscacheutil -flushcache; sudo killall -HUP mDNSResponder\nelse\n  sudo resolvectl flush-caches 2>/dev/null || sudo systemd-resolve --flush-caches 2>/dev/null || true\nfi\necho "[✓] Flushed DNS cache"\ncurl -Is https://daily-cloudcode-pa.googleapis.com | head -n 1`
                        )
                      }
                      className="text-[var(--accent)] hover:underline flex items-center gap-1 font-mono text-[10px]"
                    >
                      <span>{copiedId === "rollback-master-bash" ? "Copied" : "Copy Full Script"}</span>
                      <Copy className="h-3 w-3" />
                    </button>
                  </div>
                  <pre className="rounded-xl border border-[var(--border-subtle)] bg-black/40 p-3.5 font-mono text-xs text-cyan-300 overflow-x-auto leading-relaxed">
{`# 1. Clean /etc/hosts
sudo sed -i.bak '/cloudcode-pa\\.googleapis\\.com/d' /etc/hosts
echo "[✓] Cleaned /etc/hosts"

# 2. Unset environment variables
unset CLOUD_CODE_URL HTTP_PROXY HTTPS_PROXY
echo "[✓] Unset environment variables"

# 3. Flush DNS cache
if [[ "$OSTYPE" == "darwin"* ]]; then
  sudo dscacheutil -flushcache; sudo killall -HUP mDNSResponder
else
  sudo resolvectl flush-caches 2>/dev/null || sudo systemd-resolve --flush-caches 2>/dev/null || true
fi
echo "[✓] Flushed DNS cache"

# 4. Verify direct connection
curl -Is https://daily-cloudcode-pa.googleapis.com | head -n 1`}
                  </pre>
                </div>

                {/* Restoration Checklist */}
                <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-1.5">
                    <div className="flex items-center gap-2 text-xs font-bold text-emerald-400">
                      <CheckCircle2 className="h-4 w-4" />
                      <span>1. Hosts File Clean</span>
                    </div>
                    <p className="text-[11px] text-[var(--text-muted)]">
                      No 127.0.0.1 redirect for daily-cloudcode-pa.googleapis.com.
                    </p>
                  </div>

                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-1.5">
                    <div className="flex items-center gap-2 text-xs font-bold text-emerald-400">
                      <CheckCircle2 className="h-4 w-4" />
                      <span>2. Env Vars Cleared</span>
                    </div>
                    <p className="text-[11px] text-[var(--text-muted)]">
                      CLOUD_CODE_URL and HTTP_PROXY are unset.
                    </p>
                  </div>

                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-1.5">
                    <div className="flex items-center gap-2 text-xs font-bold text-emerald-400">
                      <CheckCircle2 className="h-4 w-4" />
                      <span>3. Settings Restored</span>
                    </div>
                    <p className="text-[11px] text-[var(--text-muted)]">
                      VS Code settings.json proxy configuration removed.
                    </p>
                  </div>

                  <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 space-y-1.5">
                    <div className="flex items-center gap-2 text-xs font-bold text-emerald-400">
                      <CheckCircle2 className="h-4 w-4" />
                      <span>4. IDE Restarted</span>
                    </div>
                    <p className="text-[11px] text-[var(--text-muted)]">
                      Antigravity window reloaded to connect directly to Google.
                    </p>
                  </div>
                </div>
              </div>
            )}

            {/* Model Selection & Interception Routing Studio */}
            <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-5 space-y-5 shadow-sm">
              {/* Header */}
              <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 border-b border-[var(--border-subtle)] pb-4">
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-purple-500/15 text-purple-400 border border-purple-500/25">
                      <Cpu className="h-4 w-4" />
                    </div>
                    <h3 className="text-sm font-bold text-[var(--text-primary)]">
                      Model Selection &amp; Interception Routing
                    </h3>
                    <span className="rounded-full bg-purple-500/10 px-2.5 py-0.5 font-mono text-[11px] font-semibold text-purple-400 border border-purple-500/20">
                      {Object.keys(mitmConfig.model_aliases).length + (mitmConfig.default_target_model ? 1 : 0)} Active Overrides
                    </span>
                  </div>
                  <p className="text-xs text-[var(--text-muted)]">
                    Choose whether intercepted IDE requests route directly to Antigravity, or swap them to DeepSeek, Qwen, Claude, or intelligent fallback Combos.
                  </p>
                </div>

                <div className="flex items-center gap-2 shrink-0">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={handleResetMitmAliases}
                    disabled={savingMitmConfig || (Object.keys(mitmConfig.model_aliases).length === 0 && !mitmConfig.default_target_model)}
                    className="gap-1.5 text-xs text-[var(--text-muted)] hover:text-rose-400"
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    <span>Reset All</span>
                  </Button>
                  <Button
                    variant="primary"
                    size="sm"
                    onClick={handleSaveMitmConfig}
                    disabled={savingMitmConfig || loadingMitmConfig}
                    className="gap-1.5 bg-purple-600 hover:bg-purple-500 text-white shadow-sm"
                  >
                    {savingMitmConfig || loadingMitmConfig ? (
                      <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                    ) : mitmSaveSuccess ? (
                      <Check className="h-3.5 w-3.5 text-white" />
                    ) : (
                      <Save className="h-3.5 w-3.5" />
                    )}
                    <span>{savingMitmConfig ? "Saving..." : loadingMitmConfig ? "Loading..." : mitmSaveSuccess ? "Saved!" : "Save Routing Rules"}</span>
                  </Button>
                </div>
              </div>

              {/* Save Success Banner */}
              {mitmSaveSuccess && (
                <div className="flex items-center gap-2 rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-3 text-xs text-emerald-300 font-medium">
                  <CheckCircle2 className="h-4 w-4 text-emerald-400" />
                  <span>MITM Model Routing rules successfully saved and active on the gateway!</span>
                </div>
              )}

              {/* Global Default Fallback Target Model */}
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-4 space-y-2.5">
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                  <div>
                    <span className="text-xs font-semibold text-[var(--text-primary)] flex items-center gap-1.5">
                      <Sliders className="h-3.5 w-3.5 text-[var(--accent)]" />
                      <span>Default Target for Unmapped Models</span>
                    </span>
                    <p className="text-[11px] text-[var(--text-muted)] mt-0.5">
                      If an IDE requests a model not explicitly mapped below, route it to:
                    </p>
                  </div>
                  <div className="w-full sm:w-80">
                    <select
                      value={mitmConfig.default_target_model || ""}
                      onChange={(e) => setMitmConfig((prev) => ({ ...prev, default_target_model: e.target.value || null }))}
                      className={INPUT_CLASS}
                    >
                      <option value="">Native Antigravity Pool (Auto Rotation across all 8+ Google Accounts)</option>
                      {targetModelOptions.combosGroup.length > 0 && (
                        <optgroup label="Combos (Load-Balanced & Fallback)">
                          {targetModelOptions.combosGroup.map((c) => (
                            <option key={c.value} value={c.value}>{c.label}</option>
                          ))}
                        </optgroup>
                      )}
                      {targetModelOptions.ovhGroup.length > 0 && (
                        <optgroup label="OVHcloud (DeepSeek & Qwen)">
                          {targetModelOptions.ovhGroup.map((c) => (
                            <option key={c.value} value={c.value}>{c.label}</option>
                          ))}
                        </optgroup>
                      )}
                      {targetModelOptions.ocGroup.length > 0 && (
                        <optgroup label="OpenCode Free Tier">
                          {targetModelOptions.ocGroup.map((c) => (
                            <option key={c.value} value={c.value}>{c.label}</option>
                          ))}
                        </optgroup>
                      )}
                      {targetModelOptions.agGroup.length > 0 && (
                        <optgroup label="Claude & Antigravity Native">
                          {targetModelOptions.agGroup.map((c) => (
                            <option key={c.value} value={c.value}>{c.label}</option>
                          ))}
                        </optgroup>
                      )}
                    </select>
                  </div>
                </div>
              </div>

              {/* Per-Model Overrides Grid */}
              <div className="space-y-3">
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-2">
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-bold text-[var(--text-primary)]">
                      Standard Intercepted Models ({filteredInterceptedModels.length})
                    </span>
                    <span className="text-[11px] text-[var(--text-muted)]">
                      Map incoming IDE models to any target model or Combo:
                    </span>
                  </div>
                  <div className="relative w-full sm:w-60">
                    <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-[var(--text-muted)]" />
                    <input
                      type="text"
                      placeholder="Filter models (e.g. claude, flash)..."
                      value={mitmFilterQuery}
                      onChange={(e) => setMitmFilterQuery(e.target.value)}
                      className={`${INPUT_CLASS} pl-8 py-1 text-xs`}
                    />
                  </div>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-2.5">
                  {filteredInterceptedModels.map((m) => {
                    const currentAlias = mitmConfig.model_aliases[m.id] || "";
                    const isOverridden = !!currentAlias;

                    return (
                      <div
                        key={m.id}
                        className={`rounded-xl border p-3 flex flex-col justify-between gap-2.5 transition-all ${
                          isOverridden
                            ? "border-purple-500/40 bg-purple-500/[0.04] shadow-sm"
                            : "border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 hover:border-[var(--border-strong)]"
                        }`}
                      >
                        <div className="flex items-start justify-between gap-2">
                          <div className="min-w-0">
                            <div className="flex items-center gap-2">
                              <span className="text-xs font-semibold text-[var(--text-primary)] truncate">
                                {m.name}
                              </span>
                              <span
                                className={`rounded px-1.5 py-0.2 font-mono text-[9px] font-bold uppercase border ${
                                  m.family === "Claude"
                                    ? "bg-amber-500/10 text-amber-400 border-amber-500/20"
                                    : m.family === "GPT"
                                    ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/20"
                                    : "bg-blue-500/10 text-blue-400 border-blue-500/20"
                                }`}
                              >
                                {m.family}
                              </span>
                            </div>
                            <span className="font-mono text-[10px] text-[var(--text-muted)]">
                              Incoming: <code className="text-[var(--text-secondary)]">{m.id}</code>
                            </span>
                          </div>

                          {isOverridden && (
                            <button
                              type="button"
                              onClick={() => handleSetModelAlias(m.id, "")}
                              className="text-[10px] text-[var(--text-muted)] hover:text-rose-400 font-medium"
                              title="Reset to default passthrough"
                            >
                              Reset
                            </button>
                          )}
                        </div>

                        <div className="flex items-center gap-2">
                          <ArrowRight className={`h-3.5 w-3.5 shrink-0 ${isOverridden ? "text-purple-400" : "text-[var(--text-muted)]"}`} />
                          <select
                            value={currentAlias}
                            onChange={(e) => handleSetModelAlias(m.id, e.target.value)}
                            className={`${INPUT_CLASS} py-1 text-xs ${
                              isOverridden ? "border-purple-500/50 text-purple-300 font-medium" : ""
                            }`}
                          >
                            <option value="">Default Passthrough ({m.name})</option>
                            {targetModelOptions.combosGroup.length > 0 && (
                              <optgroup label="Combos (Load-Balanced)">
                                {targetModelOptions.combosGroup.map((c) => (
                                  <option key={c.value} value={c.value}>{c.label}</option>
                                ))}
                              </optgroup>
                            )}
                            {targetModelOptions.ovhGroup.length > 0 && (
                              <optgroup label="OVHcloud (DeepSeek & Qwen)">
                                {targetModelOptions.ovhGroup.map((c) => (
                                  <option key={c.value} value={c.value}>{c.label}</option>
                                ))}
                              </optgroup>
                            )}
                            {targetModelOptions.ocGroup.length > 0 && (
                              <optgroup label="OpenCode Free">
                                {targetModelOptions.ocGroup.map((c) => (
                                  <option key={c.value} value={c.value}>{c.label}</option>
                                ))}
                              </optgroup>
                            )}
                            {targetModelOptions.agGroup.length > 0 && (
                              <optgroup label="Claude & Antigravity Native">
                                {targetModelOptions.agGroup.map((c) => (
                                  <option key={c.value} value={c.value}>{c.label}</option>
                                ))}
                              </optgroup>
                            )}
                          </select>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>

              {/* Custom Intercepted Model Aliases */}
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/20 p-4 space-y-3">
                <div>
                  <span className="text-xs font-bold text-[var(--text-primary)]">Custom Model Aliases</span>
                  <p className="text-[11px] text-[var(--text-muted)]">
                    Add mappings for custom model strings requested by older CLI versions or custom extensions:
                  </p>
                </div>

                {customAliasesList.length > 0 && (
                  <div className="space-y-1.5">
                    {customAliasesList.map(([src, dst]) => (
                      <div
                        key={src}
                        className="flex items-center justify-between gap-2 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 text-xs font-mono"
                      >
                        <div className="flex items-center gap-2 truncate">
                          <span className="text-[var(--text-primary)] font-semibold">{src}</span>
                          <ArrowRight className="h-3 w-3 text-purple-400" />
                          <span className="text-purple-300">{dst}</span>
                        </div>
                        <button
                          type="button"
                          onClick={() => handleRemoveCustomAlias(src)}
                          className="text-[var(--text-muted)] hover:text-rose-400"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}

                <div className="flex flex-col sm:flex-row items-center gap-2 pt-1">
                  <input
                    type="text"
                    placeholder="Incoming model (e.g. code-gecko)..."
                    value={customAliasSource}
                    onChange={(e) => setCustomAliasSource(e.target.value)}
                    className={`${INPUT_CLASS} py-1.5 text-xs font-mono`}
                  />
                  <select
                    value={customAliasTarget}
                    onChange={(e) => setCustomAliasTarget(e.target.value)}
                    className={`${INPUT_CLASS} py-1.5 text-xs`}
                  >
                    <option value="">Select Target Model / Combo...</option>
                    {targetModelOptions.combosGroup.map((c) => (
                      <option key={c.value} value={c.value}>{c.label}</option>
                    ))}
                    {targetModelOptions.ovhGroup.map((c) => (
                      <option key={c.value} value={c.value}>{c.label}</option>
                    ))}
                    {targetModelOptions.ocGroup.map((c) => (
                      <option key={c.value} value={c.value}>{c.label}</option>
                    ))}
                    {targetModelOptions.agGroup.map((c) => (
                      <option key={c.value} value={c.value}>{c.label}</option>
                    ))}
                  </select>
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={handleAddCustomAlias}
                    disabled={!customAliasSource.trim() || !customAliasTarget.trim()}
                    className="shrink-0 gap-1"
                  >
                    <Plus className="h-3.5 w-3.5" />
                    <span>Add Alias</span>
                  </Button>
                </div>
              </div>
            </div>

            {/* Antigravity MITM Core Features */}
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 space-y-1.5">
                <div className="flex items-center gap-2 text-xs font-bold text-[var(--text-primary)]">
                  <Code2 className="h-4 w-4 text-blue-400" />
                  <span>Tool Uncloaking</span>
                </div>
                <p className="text-[11px] leading-relaxed text-[var(--text-secondary)]">
                  Functions with <code className="text-blue-400">_ide</code> suffixes are seamlessly uncloaked and routed to active tools.
                </p>
              </div>

              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 space-y-1.5">
                <div className="flex items-center gap-2 text-xs font-bold text-[var(--text-primary)]">
                  <Sparkles className="h-4 w-4 text-amber-400" />
                  <span>IDE Version Normalization</span>
                </div>
                <p className="text-[11px] leading-relaxed text-[var(--text-secondary)]">
                  Enforces <code className="text-amber-400">ideVersion: 2.24.0</code> on generation so Google never soft-locks the client.
                </p>
              </div>

              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 space-y-1.5">
                <div className="flex items-center gap-2 text-xs font-bold text-[var(--text-primary)]">
                  <Globe className="h-4 w-4 text-emerald-400" />
                  <span>Proxy Isolation</span>
                </div>
                <p className="text-[11px] leading-relaxed text-[var(--text-secondary)]">
                  Every Google account is mapped to its dedicated sticky egress IP to prevent cross-account correlation.
                </p>
              </div>

              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 space-y-1.5">
                <div className="flex items-center gap-2 text-xs font-bold text-[var(--text-primary)]">
                  <Layers className="h-4 w-4 text-purple-400" />
                  <span>Account Rotation</span>
                </div>
                <p className="text-[11px] leading-relaxed text-[var(--text-secondary)]">
                  Automatic fallback across all 8+ Google accounts keeps your IDE coding uninterrupted during cooldowns.
                </p>
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
});
