import { useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowRight,
  ArrowUp,
  Check,
  Copy,
  Eye,
  Brain,
  Layers,
  Pencil,
  Play,
  Plus,
  Search,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { ProviderIcon } from "../components/ProviderIcon";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  Card,
  EmptyState,
  INPUT_CLASS,
  Modal,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  ComboRecord,
  ComboStrategy,
  CreateComboPayload,
  OpenAiModelEntry,
  ProviderCatalogItem,
  UpdateComboPayload,
} from "../types";
import { getProviderLogo } from "../utils/format";
import {
  ModelCapabilityBadges,
  ModelCapabilityIcons,
  ModelLimitBadge,
} from "../components/ModelCapabilityBadges";
import {
  getModelCapabilities,
  getModelLimits,
  getModelChipClasses,
  formatTokenLimit,
} from "../utils/modelCapabilities";

const STRATEGY_META: Record<ComboStrategy, { label: string; desc: string }> = {
  fallback: {
    label: "Fallback",
    desc: "Try models in order; fail over automatically on 429 or error",
  },
  "round-robin": {
    label: "Round Robin",
    desc: "Rotate requests evenly across models with automatic failover",
  },
  random: {
    label: "Random",
    desc: "Distribute requests randomly across models with failover",
  },
};

const LEVEL_LABEL_MAP: Record<string, string> = {
  auto: "Auto",
  none: "None",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Xhigh",
  max: "Max",
};

interface CatalogModelMeta {
  prefixedId: string;
  displayName: string;
  supportedLevels: string[];
  defaultLevel: string | null;
  entry?: OpenAiModelEntry;
  providerLogo: string;
  providerColor?: string;
  providerName: string;
  providerId: string;
}

interface ChainDraftItem {
  uid: string;
  baseModel: string;
  level: string | null;
}

function resolveModelProviderInfo(
  prefixedId: string,
  entry: OpenAiModelEntry | undefined,
  providers: ProviderCatalogItem[],
): { providerId: string; providerName: string; providerLogo: string; providerColor: string } {
  const slashIdx = prefixedId.indexOf("/");
  const prefix = slashIdx >= 0 ? prefixedId.slice(0, slashIdx).toLowerCase() : "";
  const ownedBy = entry?.owned_by?.toLowerCase() || "";

  // 1. Check matching provider from configured providers catalog
  const matched = providers.find(
    (p) =>
      p.id.toLowerCase() === prefix ||
      p.prefix.toLowerCase() === prefix ||
      p.id.toLowerCase() === ownedBy ||
      p.prefix.toLowerCase() === ownedBy,
  );

  if (matched) {
    return {
      providerId: matched.id,
      providerName: matched.name,
      providerLogo: matched.favicon_url || matched.logo || getProviderLogo(matched.prefix || matched.id),
      providerColor: matched.color || "#0EA5E9",
    };
  }

  // 2. Specific heuristics
  if (prefix === "lf" || prefix === "cp-lf" || ownedBy.includes("logfare") || ownedBy.includes("logflare")) {
    return {
      providerId: "cp-lf",
      providerName: "Logfare",
      providerLogo: "/providers/logfare.png",
      providerColor: "#0EA5E9",
    };
  }
  if (prefix === "ovh" || ownedBy === "ovh") {
    return {
      providerId: "ovh",
      providerName: "OVHcloud",
      providerLogo: "/providers/ovh.png",
      providerColor: "#0053D6",
    };
  }
  if (prefix === "ag" || prefix === "antigravity" || ownedBy === "antigravity") {
    return {
      providerId: "antigravity",
      providerName: "Antigravity",
      providerLogo: "/providers/antigravity.png",
      providerColor: "#38BDF8",
    };
  }
  if (prefix === "oc" || prefix === "opencode" || ownedBy === "opencode") {
    return {
      providerId: "opencode",
      providerName: "OpenCode Free",
      providerLogo: "/providers/opencode.png",
      providerColor: "#E56A4A",
    };
  }

  return {
    providerId: prefix || "custom",
    providerName: prefix ? prefix.toUpperCase() : "Custom",
    providerLogo: getProviderLogo(prefixedId),
    providerColor: "#64748B",
  };
}

interface EffectiveComboLimits {
  label: string;
  tooltip: string;
  contextMin?: number;
  contextMax?: number;
  outputMin?: number;
  outputMax?: number;
  primaryContext?: number;
  primaryOutput?: number;
}

function computeComboLimits(
  modelSpecs: string[],
  catalogMap: Map<string, CatalogModelMeta>,
  strategy: ComboStrategy,
): EffectiveComboLimits | null {
  if (modelSpecs.length === 0) return null;

  const resolvedLimits = modelSpecs.map((spec) => {
    const parsed = parseComboModelSpec(spec);
    const meta = catalogMap.get(parsed.baseModel);
    return getModelLimits(parsed.baseModel, meta?.entry);
  });

  const validLimits = resolvedLimits.filter((l): l is NonNullable<typeof l> => l != null);
  if (validLimits.length === 0) return null;

  const contexts = validLimits.map((l) => l.context).filter((c): c is number => c != null && c > 0);
  const outputs = validLimits.map((l) => l.output).filter((o): o is number => o != null && o > 0);

  const primaryLimit = validLimits[0];
  const primaryCtx = primaryLimit?.context;
  const primaryOut = primaryLimit?.output;

  const minCtx = contexts.length > 0 ? Math.min(...contexts) : undefined;
  const maxCtx = contexts.length > 0 ? Math.max(...contexts) : undefined;
  const minOut = outputs.length > 0 ? Math.min(...outputs) : undefined;
  const maxOut = outputs.length > 0 ? Math.max(...outputs) : undefined;

  let ctxPart = "";
  if (minCtx != null && maxCtx != null) {
    if (minCtx === maxCtx) {
      ctxPart = `ctx ${formatTokenLimit(minCtx)}`;
    } else {
      ctxPart = `ctx ${formatTokenLimit(minCtx)} ~ ${formatTokenLimit(maxCtx)}`;
    }
  }

  let outPart = "";
  if (minOut != null && maxOut != null) {
    if (minOut === maxOut) {
      outPart = `out ${formatTokenLimit(minOut)}`;
    } else {
      outPart = `out ${formatTokenLimit(minOut)} ~ ${formatTokenLimit(maxOut)}`;
    }
  }

  const label = [ctxPart, outPart].filter(Boolean).join(" · ");
  let tooltip = "";
  if (strategy === "fallback" && primaryLimit) {
    tooltip = `Primary: ${formatTokenLimit(primaryCtx)} ctx / ${formatTokenLimit(primaryOut)} out · Fallback Floor: ${formatTokenLimit(minCtx)} ctx / ${formatTokenLimit(minOut)} out`;
  } else {
    tooltip = `Pipeline Range: ${formatTokenLimit(minCtx)} - ${formatTokenLimit(maxCtx)} ctx · ${formatTokenLimit(minOut)} - ${formatTokenLimit(maxOut)} out`;
  }

  return {
    label,
    tooltip,
    contextMin: minCtx,
    contextMax: maxCtx,
    outputMin: minOut,
    outputMax: maxOut,
    primaryContext: primaryCtx,
    primaryOutput: primaryOut,
  };
}

function parseComboModelSpec(raw: string): { baseModel: string; level: string | null } {
  const trimmed = raw.trim();
  const parenMatch = trimmed.match(/^(.*)\(([a-z0-9_-]+)\)$/i);
  if (parenMatch) {
    const levelCandidate = parenMatch[2].trim().toLowerCase();
    if (levelCandidate in LEVEL_LABEL_MAP) {
      return {
        baseModel: parenMatch[1].trim(),
        level: levelCandidate,
      };
    }
  }
  return { baseModel: trimmed, level: null };
}

function formatComboModelSpec(baseModel: string, level: string | null): string {
  const cleanBase = baseModel.trim();
  if (!level) return cleanBase;
  return `${cleanBase}(${level})`;
}

export function CombosView({
  combos,
  models,
  providers = [],
  loading,
  onCreateCombo,
  onUpdateCombo,
  onDeleteCombo,
  onTestModel,
}: {
  combos: ComboRecord[];
  models: OpenAiModelEntry[];
  providers?: ProviderCatalogItem[];
  loading: boolean;
  onCreateCombo: (payload: CreateComboPayload) => Promise<void>;
  onUpdateCombo: (id: string, payload: UpdateComboPayload) => Promise<void>;
  onDeleteCombo: (id: string, name: string) => Promise<void>;
  onTestModel: (modelId: string) => void;
}) {
  const [search, setSearch] = useState("");
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const uidCounterRef = useRef(0);
  function nextUid() {
    uidCounterRef.current += 1;
    return `chain-${uidCounterRef.current}`;
  }

  // Modal state (Create or Edit)
  const [modalOpen, setModalOpen] = useState(false);
  const [editing, setEditing] = useState<ComboRecord | null>(null);
  const [name, setName] = useState("");
  const [strategy, setStrategy] = useState<ComboStrategy>("fallback");
  const [chain, setChain] = useState<ChainDraftItem[]>([]);
  const [pickerSearch, setPickerSearch] = useState("");
  const [pickerCategory, setPickerCategory] = useState<string>("all");
  const [customModelInput, setCustomModelInput] = useState("");
  const [saving, setSaving] = useState(false);

  const catalogEntries = useMemo(() => {
    const map = new Map<string, CatalogModelMeta>();
    const list: CatalogModelMeta[] = [];
    for (const m of models) {
      if (m.owned_by === "combo") continue;
      const prefixedId = m.id.includes("/")
        ? m.id
        : m.owned_by === "antigravity"
          ? `ag/${m.id}`
          : m.owned_by === "ovh"
            ? `ovh/${m.id}`
            : `oc/${m.id}`;
      if (map.has(prefixedId)) continue;
      const provInfo = resolveModelProviderInfo(prefixedId, m, providers);
      const meta: CatalogModelMeta = {
        prefixedId,
        displayName: m.display_name || m.id,
        supportedLevels: Array.isArray(m.supported_levels) ? m.supported_levels : [],
        defaultLevel: m.default_level ?? null,
        entry: m,
        providerLogo: provInfo.providerLogo,
        providerColor: provInfo.providerColor,
        providerName: provInfo.providerName,
        providerId: provInfo.providerId,
      };
      map.set(prefixedId, meta);
      list.push(meta);
    }
    return { map, list };
  }, [models, providers]);

  const availableProviderFilters = useMemo(() => {
    const map = new Map<
      string,
      { id: string; name: string; count: number; logo: string; color?: string }
    >();
    for (const item of catalogEntries.list) {
      const pid = item.providerId;
      const cur = map.get(pid);
      if (cur) {
        cur.count++;
      } else {
        map.set(pid, {
          id: pid,
          name: item.providerName,
          count: 1,
          logo: item.providerLogo,
          color: item.providerColor,
        });
      }
    }
    return Array.from(map.values());
  }, [catalogEntries.list]);

  if (loading && combos.length === 0 && models.length === 0) {
    return <TableSkeleton rows={5} />;
  }

  const availableModels: string[] = catalogEntries.list.map((m) => m.prefixedId);

  function validateChainEntry(
    baseModel: string,
    level: string | null,
  ): string | null {
    if (catalogEntries.list.length === 0) return null;
    const meta = catalogEntries.map.get(baseModel);
    if (!meta) return "Model not in active catalog";
    if (meta.supportedLevels.length > 0 && level && !meta.supportedLevels.includes(level)) {
      return `Level "${level}" unsupported (${meta.supportedLevels.join(", ")})`;
    }
    if (meta.supportedLevels.length === 0 && level) {
      return `Model does not support thinking level "${level}"`;
    }
    return null;
  }

  function makeDraftItem(rawSpec: string): ChainDraftItem {
    const parsed = parseComboModelSpec(rawSpec);
    const meta = catalogEntries.map.get(parsed.baseModel);
    let level = parsed.level;
    if (!level && meta && meta.supportedLevels.length > 0) {
      level = meta.defaultLevel || meta.supportedLevels[0];
    }
    return {
      uid: nextUid(),
      baseModel: parsed.baseModel,
      level,
    };
  }

  function copyText(id: string, val: string) {
    void navigator.clipboard.writeText(val);
    setCopiedId(id);
    setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 1500);
  }

  function openCreate() {
    setEditing(null);
    setName("");
    setStrategy("fallback");
    setChain(availableModels.slice(0, 2).map((m) => makeDraftItem(m)));
    setPickerSearch("");
    setPickerCategory("all");
    setCustomModelInput("");
    setModalOpen(true);
  }

  function openEdit(c: ComboRecord) {
    setEditing(c);
    setName(c.name);
    setStrategy(c.strategy);
    setChain(
      c.models.map((raw) => {
        const parsed = parseComboModelSpec(raw);
        return {
          uid: nextUid(),
          baseModel: parsed.baseModel,
          level: parsed.level,
        };
      }),
    );
    setPickerSearch("");
    setPickerCategory("all");
    setCustomModelInput("");
    setModalOpen(true);
  }

  function addModelToChain(raw: string) {
    const clean = raw.trim();
    if (!clean) return;
    const parsed = parseComboModelSpec(clean);
    const meta = catalogEntries.map.get(parsed.baseModel);

    // Single-level models cannot have duplicate entries without differentiating levels
    if (meta && meta.supportedLevels.length === 0) {
      if (chain.some((item) => item.baseModel === parsed.baseModel)) return;
      setChain((prev) => [
        ...prev,
        { uid: nextUid(), baseModel: parsed.baseModel, level: parsed.level },
      ]);
      return;
    }

    // Multi-level models CAN be added multiple times; auto-pick next unused level if possible
    if (meta && meta.supportedLevels.length > 0) {
      const usedLevels = new Set(
        chain.filter((item) => item.baseModel === parsed.baseModel).map((i) => i.level),
      );
      let pickedLevel = parsed.level;
      if (!pickedLevel) {
        const def = meta.defaultLevel || meta.supportedLevels[0];
        if (!usedLevels.has(def)) {
          pickedLevel = def;
        } else {
          pickedLevel =
            meta.supportedLevels.find((lvl) => !usedLevels.has(lvl)) ?? def;
        }
      }
      setChain((prev) => [
        ...prev,
        { uid: nextUid(), baseModel: parsed.baseModel, level: pickedLevel },
      ]);
      return;
    }

    // Custom spec
    if (chain.some((item) => formatComboModelSpec(item.baseModel, item.level) === clean)) {
      return;
    }
    setChain((prev) => [
      ...prev,
      { uid: nextUid(), baseModel: parsed.baseModel, level: parsed.level },
    ]);
  }

  function updateChainItemLevel(uid: string, nextLevel: string | null) {
    setChain((prev) =>
      prev.map((item) => (item.uid === uid ? { ...item, level: nextLevel } : item)),
    );
  }

  function moveInChain(idx: number, dir: -1 | 1) {
    const target = idx + dir;
    if (target < 0 || target >= chain.length) return;
    const copy = [...chain];
    const [item] = copy.splice(idx, 1);
    copy.splice(target, 0, item);
    setChain(copy);
  }

  async function handleSave(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim() || chain.length === 0) return;
    const serializedModels = chain.map((item) =>
      formatComboModelSpec(item.baseModel, item.level),
    );
    setSaving(true);
    try {
      if (editing) {
        await onUpdateCombo(editing.id, {
          name: name.trim(),
          strategy,
          models: serializedModels,
        });
      } else {
        await onCreateCombo({
          name: name.trim(),
          strategy,
          models: serializedModels,
          enabled: true,
        });
      }
      setModalOpen(false);
    } finally {
      setSaving(false);
    }
  }

  async function handleCreateStarterCombo() {
    const defaults = availableModels.filter(
      (m) =>
        m.includes("glm") ||
        m.includes("deepseek") ||
        m.includes("mimo") ||
        m.includes("nemotron"),
    );
    const picked = defaults.length >= 2 ? defaults.slice(0, 3) : availableModels.slice(0, 3);
    if (picked.length === 0) return;
    setSaving(true);
    try {
      await onCreateCombo({
        name: "smart-free",
        strategy: "fallback",
        models: picked,
        enabled: true,
      });
    } finally {
      setSaving(false);
    }
  }

  const qLower = search.trim().toLowerCase();
  const filteredCombos = combos.filter(
    (c) =>
      !qLower ||
      c.name.toLowerCase().includes(qLower) ||
      c.models.some((m) => m.toLowerCase().includes(qLower)),
  );

  const pickerFiltered = catalogEntries.list.filter((meta) => {
    // Allow re-picking multi-level models (e.g. Gemini 3.8 Flash high + low),
    // hide only single-level models already present in chain.
    const alreadySingleAdded =
      meta.supportedLevels.length === 0 &&
      chain.some((item) => item.baseModel === meta.prefixedId);
    if (alreadySingleAdded) return false;

    if (pickerCategory === "vision") {
      if (!getModelCapabilities(meta.prefixedId).hasVision) return false;
    } else if (pickerCategory === "thinking") {
      if (!getModelCapabilities(meta.prefixedId).hasThinking) return false;
    } else if (pickerCategory !== "all") {
      const matchProvider =
        meta.providerId === pickerCategory ||
        meta.providerId.toLowerCase() === pickerCategory.toLowerCase() ||
        meta.prefixedId.toLowerCase().startsWith(`${pickerCategory.toLowerCase()}/`);
      if (!matchProvider) return false;
    }

    const pq = pickerSearch.trim().toLowerCase();
    if (!pq) return true;
    return (
      meta.prefixedId.toLowerCase().includes(pq) ||
      meta.displayName.toLowerCase().includes(pq)
    );
  });

  return (
    <div className="space-y-5">
      {/* Top Toolbar */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="relative w-full sm:w-80">
          <Search className="pointer-events-none absolute top-2 left-3 h-3.5 w-3.5 text-[var(--text-muted)]" />
          <input
            type="text"
            placeholder="Search combos or target models..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className={`${INPUT_CLASS} pl-8`}
          />
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {!combos.some((c) => c.name === "smart-free") && availableModels.length > 0 && (
            <Button
              variant="secondary"
              onClick={() => void handleCreateStarterCombo()}
              disabled={saving}
            >
              <Sparkles className="h-3.5 w-3.5 text-[var(--accent)]" />
              Add starter combo
            </Button>
          )}
          <Button variant="primary" onClick={openCreate}>
            <Plus className="h-3.5 w-3.5" />
            Create combo
          </Button>
        </div>
      </div>

      {/* Combo List */}
      {filteredCombos.length === 0 ? (
        <EmptyState
          icon={<Layers className="h-5 w-5" />}
          title={combos.length === 0 ? "No model combos yet" : "No matching combos"}
          description="Bundle multiple models into a single virtual model ID with automatic fallback or round-robin load balancing — usable directly in OpenCode, Claude Code, and any SDK."
          action={
            <div className="flex flex-wrap items-center justify-center gap-2">
              {availableModels.length > 0 && (
                <Button
                  variant="secondary"
                  onClick={() => void handleCreateStarterCombo()}
                  disabled={saving}
                >
                  <Sparkles className="h-3.5 w-3.5 text-[var(--accent)]" />
                  Create smart-free preset
                </Button>
              )}
              <Button variant="primary" onClick={openCreate}>
                <Plus className="h-3.5 w-3.5" />
                Create custom combo
              </Button>
            </div>
          }
        />
      ) : (
        <div className="space-y-3">
          {filteredCombos.map((combo) => {
            const comboLimits = computeComboLimits(combo.models, catalogEntries.map, combo.strategy);
            return (
              <Card key={combo.id} noPadding className="overflow-hidden">
                <div className="p-4 sm:p-5">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-3">
                      <ProviderIcon name={combo.name} isCombo size={34} />
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="font-mono text-sm font-semibold text-[var(--text-primary)]">
                            {combo.name}
                          </span>
                          <button
                            type="button"
                            onClick={() => copyText(combo.id, combo.name)}
                            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                            title="Copy combo model ID"
                          >
                            {copiedId === combo.id ? (
                              <Check className="h-3 w-3 text-[var(--success)]" />
                            ) : (
                              <Copy className="h-3 w-3" />
                            )}
                            {copiedId === combo.id ? "Copied" : "Copy"}
                          </button>
                          <StatusPill tone="neutral">
                            {STRATEGY_META[combo.strategy]?.label ?? combo.strategy}
                          </StatusPill>
                          <StatusPill tone={combo.enabled ? "success" : "neutral"} dot>
                            {combo.enabled ? "Enabled" : "Paused"}
                          </StatusPill>
                          {comboLimits && (
                            <span
                              className="inline-flex items-center gap-1 rounded-md border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-[11px] font-semibold text-[var(--accent)] cursor-help shadow-2xs"
                              title={comboLimits.tooltip}
                            >
                              <Layers className="h-3 w-3 text-[var(--text-muted)]" />
                              {comboLimits.label}
                            </span>
                          )}
                        </div>
                      </div>
                    </div>

                    <div className="flex flex-wrap items-center gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => onTestModel(combo.name)}
                      >
                        <Play className="h-3 w-3" />
                        Test
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => openEdit(combo)}>
                        <Pencil className="h-3 w-3" />
                        Edit
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          void onUpdateCombo(combo.id, { enabled: !combo.enabled })
                        }
                      >
                        {combo.enabled ? "Pause" : "Enable"}
                      </Button>
                      <button
                        type="button"
                        onClick={() => void onDeleteCombo(combo.id, combo.name)}
                        className="rounded-md p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--danger-soft)] hover:text-[var(--danger)]"
                        title={`Delete ${combo.name}`}
                        aria-label={`Delete ${combo.name}`}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </div>

                  {/* Ordered Model Chain */}
                  <div className="mt-3.5 flex flex-wrap items-center gap-2 border-t border-[var(--border-subtle)] pt-3.5">
                    {combo.models.map((m, idx) => {
                      const parsed = parseComboModelSpec(m);
                      const meta = catalogEntries.map.get(parsed.baseModel);
                      const warning = validateChainEntry(parsed.baseModel, parsed.level);
                      const friendlyName = meta?.displayName || parsed.baseModel;
                      const caps = getModelCapabilities(parsed.baseModel);
                      const limits = getModelLimits(parsed.baseModel, meta?.entry);
                      const chipStyle = getModelChipClasses(caps);
                      const provLogo = meta?.providerLogo || getProviderLogo(parsed.baseModel);
                      return (
                        <div key={`${m}-${idx}`} className="flex items-center gap-2">
                          <div
                            title={warning ? `${m} — ${warning}` : limits?.tooltip ? `${m} (${limits.tooltip})` : m}
                            className={`inline-flex items-center gap-2 rounded-lg border px-2.5 py-1.5 transition-colors ${
                              warning
                                ? "border-[color-mix(in_srgb,var(--warning)_45%,transparent)] bg-[var(--warning-soft)]"
                                : chipStyle
                            }`}
                          >
                            <span className="font-mono text-[10px] font-semibold text-[var(--text-muted)]">
                              {idx + 1}
                            </span>
                            <ProviderIcon src={provLogo} name={friendlyName} color={meta?.providerColor} size={18} />
                            <span className="text-xs font-medium text-[var(--text-primary)]">
                              {friendlyName}
                              {parsed.level && (
                                <span className="ml-1 font-mono text-[11px] font-semibold text-[var(--accent)]">
                                  ({parsed.level})
                                </span>
                              )}
                            </span>
                          <ModelCapabilityBadges caps={caps} compact />
                          <ModelLimitBadge limits={limits} compact />
                          {warning && (
                            <span
                              className="inline-flex items-center gap-0.5 font-mono text-[10px] font-medium text-[var(--warning)]"
                              title={warning}
                            >
                              <AlertTriangle className="h-3 w-3" />
                              Unsupported
                            </span>
                          )}
                        </div>
                        {idx < combo.models.length - 1 && (
                          <ArrowRight className="h-3.5 w-3.5 text-[var(--text-muted)]" />
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            </Card>
          );
        })}
        </div>
      )}

      {/* Create / Edit Combo Modal */}
      <Modal
        open={modalOpen}
        title={editing ? `Edit Combo: ${editing.name}` : "Create Model Combo"}
        onClose={() => setModalOpen(false)}
        maxWidth="max-w-3xl"
      >
        <form onSubmit={handleSave} className="space-y-4">
          {/* Section 1: Identification & Strategy */}
          <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 p-3.5 space-y-3">
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div>
                <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                  Combo Model ID
                </label>
                <input
                  type="text"
                  required
                  placeholder="e.g. smart-free, fast-coding"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className={`${INPUT_CLASS} font-mono text-sm`}
                />
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  Exposed to clients as{" "}
                  <code className="rounded bg-[var(--bg-surface)] px-1 py-0.2 font-mono text-[var(--accent)] font-semibold">
                    {name.trim() || "<id>"}
                  </code>
                </p>
              </div>

              <div>
                <label className="mb-1 block text-xs font-semibold text-[var(--text-secondary)]">
                  Routing Strategy
                </label>
                <select
                  value={strategy}
                  onChange={(e) => setStrategy(e.target.value as ComboStrategy)}
                  className={`${INPUT_CLASS} text-sm`}
                >
                  <option value="fallback">Fallback — Try models in sequence on 429/error</option>
                  <option value="round-robin">Round Robin — Rotate requests evenly</option>
                  <option value="random">Random — Distribute load across models</option>
                </select>
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  {strategy === "fallback"
                    ? "Tries model #1 first; automatically falls over to #2 on 429 quota or failure."
                    : strategy === "round-robin"
                    ? "Rotates requests evenly in round-robin fashion across all chain targets."
                    : "Randomly dispatches requests to balance load across configured models."}
                </p>
              </div>
            </div>
          </div>

          {/* Section 2: Active Pipeline */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-xs font-semibold text-[var(--text-secondary)] uppercase tracking-wider">
                  Target Models Pipeline
                </span>
                <span className="rounded-full bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-[11px] font-semibold text-[var(--accent)] border border-[var(--border-subtle)]">
                  {chain.length} {chain.length === 1 ? "model" : "models"}
                </span>
              </div>
              {chain.length > 1 && (
                <span className="text-[11px] text-[var(--text-muted)]">
                  {strategy === "fallback" ? "Dispatches in top-to-bottom sequence" : "Rotation list"}
                </span>
              )}
            </div>

            {/* Live Effective Limits for Combo Chain */}
            {(() => {
              const chainLimits = computeComboLimits(
                chain.map((item) => formatComboModelSpec(item.baseModel, item.level)),
                catalogEntries.map,
                strategy,
              );
              if (!chainLimits) return null;
              return (
                <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 px-3 py-2 text-xs">
                  <div className="flex items-center gap-2">
                    <Layers className="h-3.5 w-3.5 text-[var(--accent)]" />
                    <span className="text-[var(--text-secondary)] font-medium">Pipeline Limits:</span>
                    <span className="font-mono font-semibold text-[var(--accent)]">{chainLimits.label}</span>
                  </div>
                  <span className="font-mono text-[11px] text-[var(--text-muted)]">{chainLimits.tooltip}</span>
                </div>
              );
            })()}

            {chain.length === 0 ? (
              <div className="rounded-xl border border-dashed border-[var(--border-strong)] bg-[var(--bg-elevated)]/30 p-6 text-center space-y-1">
                <Layers className="mx-auto h-6 w-6 text-[var(--text-muted)]" />
                <p className="text-xs font-medium text-[var(--text-primary)]">Your combo chain is empty</p>
                <p className="text-[11px] text-[var(--text-muted)]">
                  Click any model from the catalog below to add it to your pipeline.
                </p>
              </div>
            ) : (
              <div className="space-y-1.5 max-h-48 overflow-y-auto pr-1">
                {chain.map((item, idx) => {
                  const meta = catalogEntries.map.get(item.baseModel);
                  const supportedLevels = meta?.supportedLevels ?? [];
                  const hasMultipleLevels = supportedLevels.length > 0;
                  const warning = validateChainEntry(item.baseModel, item.level);
                  const wireSpec = formatComboModelSpec(item.baseModel, item.level);
                  const caps = getModelCapabilities(item.baseModel);
                  const limits = getModelLimits(item.baseModel, meta?.entry);
                  const provLogo = meta?.providerLogo || getProviderLogo(item.baseModel);
                  const levelUnsupported =
                    !!item.level &&
                    ((hasMultipleLevels && !supportedLevels.includes(item.level)) ||
                      (!!meta && !hasMultipleLevels));

                  return (
                    <div
                      key={item.uid}
                      className={`flex flex-wrap items-center justify-between gap-2.5 rounded-xl border px-3 py-2 transition-colors ${
                        warning
                          ? "border-[color-mix(in_srgb,var(--warning)_45%,transparent)] bg-[var(--warning-soft)]"
                          : "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]"
                      }`}
                    >
                      <div className="flex min-w-0 flex-1 items-center gap-2.5">
                        <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-[var(--bg-elevated)] font-mono text-[10px] font-bold text-[var(--accent)] border border-[var(--border-subtle)]">
                          {idx + 1}
                        </span>
                        <ProviderIcon
                          src={provLogo}
                          name={meta?.displayName || item.baseModel}
                          color={meta?.providerColor}
                          size={22}
                        />
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="truncate text-xs font-semibold text-[var(--text-primary)]">
                              {meta?.displayName || item.baseModel}
                            </span>
                            {item.level && (
                              <span className="rounded bg-[var(--bg-elevated)] px-1.5 py-0.2 font-mono text-[10px] font-semibold text-[var(--accent)] border border-[var(--border-subtle)]">
                                {item.level}
                              </span>
                            )}
                            {warning && (
                              <span
                                className="inline-flex items-center gap-1 rounded bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[var(--warning)]"
                                title={warning}
                              >
                                <AlertTriangle className="h-3 w-3 shrink-0" />
                                {warning}
                              </span>
                            )}
                          </div>
                          <div className="flex items-center gap-2 pt-0.5">
                            <span className="truncate font-mono text-[10px] text-[var(--text-muted)]">
                              {wireSpec}
                            </span>
                            <ModelLimitBadge limits={limits} compact />
                          </div>
                        </div>
                      </div>

                      <div className="flex shrink-0 items-center gap-1.5">
                        <ModelCapabilityIcons caps={caps} />

                        {hasMultipleLevels && (
                          <select
                            aria-label={`Level for ${meta?.displayName || item.baseModel}`}
                            value={item.level ?? meta?.defaultLevel ?? supportedLevels[0]}
                            onChange={(e) => updateChainItemLevel(item.uid, e.target.value)}
                            className="rounded-md border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2 py-1 font-mono text-[11px] text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
                          >
                            {levelUnsupported && item.level && (
                              <option value={item.level}>
                                {item.level} (unsupported)
                              </option>
                            )}
                            {supportedLevels.map((lvl) => (
                              <option key={lvl} value={lvl}>
                                {LEVEL_LABEL_MAP[lvl] ?? lvl} ({lvl})
                              </option>
                            ))}
                          </select>
                        )}

                        <div className="flex items-center gap-0.5 ml-1 pl-1 border-l border-[var(--border-subtle)]">
                          <button
                            type="button"
                            disabled={idx === 0}
                            onClick={() => moveInChain(idx, -1)}
                            className="rounded p-1 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)] disabled:opacity-25"
                            title="Move up"
                          >
                            <ArrowUp className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            disabled={idx === chain.length - 1}
                            onClick={() => moveInChain(idx, 1)}
                            className="rounded p-1 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)] disabled:opacity-25"
                            title="Move down"
                          >
                            <ArrowDown className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={() =>
                              setChain((prev) => prev.filter((x) => x.uid !== item.uid))
                            }
                            className="rounded p-1 text-[var(--danger)] hover:bg-[var(--danger-soft)]"
                            title="Remove model"
                          >
                            <X className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* Section 3: Add From Catalog */}
          <div className="space-y-2.5 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 p-3.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-xs font-semibold text-[var(--text-secondary)] uppercase tracking-wider">
                Add Models From Catalog
              </span>
              <div className="relative w-48">
                <Search className="pointer-events-none absolute top-2 left-2.5 h-3.5 w-3.5 text-[var(--text-muted)]" />
                <input
                  type="text"
                  placeholder="Search catalog..."
                  value={pickerSearch}
                  onChange={(e) => setPickerSearch(e.target.value)}
                  className={`${INPUT_CLASS} pl-8 py-1 text-xs`}
                />
              </div>
            </div>

            {/* Dynamic Filter Pills */}
            <div className="flex flex-wrap items-center gap-1.5">
              <button
                type="button"
                onClick={() => setPickerCategory("all")}
                className={`rounded-lg px-2.5 py-1 text-[11px] font-medium transition-colors ${
                  pickerCategory === "all"
                    ? "bg-[var(--accent)] text-white shadow-2xs"
                    : "bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] border border-[var(--border-subtle)]"
                }`}
              >
                All Models ({catalogEntries.list.length})
              </button>
              <button
                type="button"
                onClick={() => setPickerCategory("vision")}
                className={`inline-flex items-center gap-1 rounded-lg px-2.5 py-1 text-[11px] font-medium transition-colors ${
                  pickerCategory === "vision"
                    ? "bg-sky-500 text-white shadow-2xs"
                    : "bg-[var(--bg-surface)] text-sky-400 hover:bg-sky-500/10 border border-sky-500/30"
                }`}
              >
                <Eye className="h-3 w-3" />
                Vision
              </button>
              <button
                type="button"
                onClick={() => setPickerCategory("thinking")}
                className={`inline-flex items-center gap-1 rounded-lg px-2.5 py-1 text-[11px] font-medium transition-colors ${
                  pickerCategory === "thinking"
                    ? "bg-amber-500 text-white shadow-2xs"
                    : "bg-[var(--bg-surface)] text-amber-400 hover:bg-amber-500/10 border border-amber-500/30"
                }`}
              >
                <Brain className="h-3 w-3" />
                Thinking
              </button>
              {availableProviderFilters.map((prov) => {
                const isActive = pickerCategory === prov.id;
                return (
                  <button
                    key={prov.id}
                    type="button"
                    onClick={() => setPickerCategory(prov.id)}
                    className={`inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[11px] font-medium transition-colors ${
                      isActive
                        ? "bg-[var(--accent)] text-white shadow-2xs"
                        : "bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] border border-[var(--border-subtle)]"
                    }`}
                  >
                    <ProviderIcon src={prov.logo} name={prov.name} color={prov.color} size={14} />
                    <span>{prov.name}</span>
                    <span
                      className={`font-mono text-[10px] ${
                        isActive ? "text-white/80" : "text-[var(--text-muted)]"
                      }`}
                    >
                      ({prov.count})
                    </span>
                  </button>
                );
              })}
            </div>

            {/* Catalog Grid */}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 max-h-52 overflow-y-auto pr-1">
              {pickerFiltered.length === 0 ? (
                <div className="col-span-2 py-4 text-center text-xs text-[var(--text-muted)]">
                  No matching catalog models found.
                </div>
              ) : (
                pickerFiltered.map((meta) => {
                  const caps = getModelCapabilities(meta.prefixedId);
                  const limits = getModelLimits(meta.prefixedId, meta.entry);
                  return (
                    <button
                      key={meta.prefixedId}
                      type="button"
                      onClick={() => addModelToChain(meta.prefixedId)}
                      className="group flex items-center justify-between gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-2.5 text-left transition-[border-color,background-color] hover:border-[var(--accent)] hover:bg-[var(--bg-hover)]/60 cursor-pointer shadow-2xs"
                      title={`Click to add ${meta.prefixedId}`}
                    >
                      <div className="flex min-w-0 items-center gap-2">
                        <ProviderIcon src={meta.providerLogo} name={meta.displayName} color={meta.providerColor} size={20} />
                        <div className="min-w-0">
                          <div className="truncate text-xs font-semibold text-[var(--text-primary)]">
                            {meta.displayName}
                          </div>
                          <div className="truncate font-mono text-[10px] text-[var(--text-muted)]">
                            {meta.prefixedId}
                          </div>
                        </div>
                      </div>

                      <div className="flex items-center gap-1.5 shrink-0">
                        <ModelCapabilityIcons caps={caps} />
                        <ModelLimitBadge limits={limits} compact />
                        <span className="rounded-md bg-[var(--bg-elevated)] p-1 text-[var(--accent)] group-hover:bg-[var(--accent)] group-hover:text-white transition-colors">
                          <Plus className="h-3.5 w-3.5" />
                        </span>
                      </div>
                    </button>
                  );
                })
              )}
            </div>

            {/* Custom Model Input */}
            <div className="flex gap-2 pt-1 border-t border-[var(--border-subtle)]/50">
              <input
                type="text"
                placeholder="Or enter custom model ID (e.g. ocz/claude-sonnet-4)..."
                value={customModelInput}
                onChange={(e) => setCustomModelInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addModelToChain(customModelInput);
                    setCustomModelInput("");
                  }
                }}
                className={`${INPUT_CLASS} flex-1 font-mono text-xs`}
              />
              <Button
                variant="secondary"
                size="sm"
                onClick={() => {
                  addModelToChain(customModelInput);
                  setCustomModelInput("");
                }}
              >
                <Plus className="h-3.5 w-3.5" />
                Add Custom
              </Button>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-2 border-t border-[var(--border-subtle)]">
            <Button variant="secondary" size="md" onClick={() => setModalOpen(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="primary"
              size="md"
              disabled={saving || !name.trim() || chain.length === 0}
            >
              {saving ? "Saving..." : editing ? "Save combo" : "Create combo"}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
