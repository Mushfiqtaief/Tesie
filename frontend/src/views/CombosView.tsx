import { useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowDown,
  ArrowRight,
  ArrowUp,
  Check,
  Copy,
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
  UpdateComboPayload,
} from "../types";
import { getProviderLogo } from "../utils/format";

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
}

interface ChainDraftItem {
  uid: string;
  baseModel: string;
  level: string | null;
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
  loading,
  onCreateCombo,
  onUpdateCombo,
  onDeleteCombo,
  onTestModel,
}: {
  combos: ComboRecord[];
  models: OpenAiModelEntry[];
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
          : `oc/${m.id}`;
      if (map.has(prefixedId)) continue;
      const meta: CatalogModelMeta = {
        prefixedId,
        displayName: m.display_name || m.id,
        supportedLevels: Array.isArray(m.supported_levels) ? m.supported_levels : [],
        defaultLevel: m.default_level ?? null,
      };
      map.set(prefixedId, meta);
      list.push(meta);
    }
    return { map, list };
  }, [models]);

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
          {filteredCombos.map((combo) => (
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
                    return (
                      <div key={`${m}-${idx}`} className="flex items-center gap-2">
                        <div
                          title={warning ? `${m} — ${warning}` : m}
                          className={`inline-flex items-center gap-2 rounded-lg border px-2.5 py-1.5 ${
                            warning
                              ? "border-[color-mix(in_srgb,var(--warning)_45%,transparent)] bg-[var(--warning-soft)]"
                              : "border-[var(--border-subtle)] bg-[var(--bg-elevated)]"
                          }`}
                        >
                          <span className="font-mono text-[10px] font-semibold text-[var(--text-muted)]">
                            {idx + 1}
                          </span>
                          <ProviderIcon src={getProviderLogo(parsed.baseModel)} name={m} size={18} />
                          <span className="text-xs font-medium text-[var(--text-primary)]">
                            {friendlyName}
                            {parsed.level && (
                              <span className="ml-1 font-mono text-[11px] font-semibold text-[var(--accent)]">
                                ({parsed.level})
                              </span>
                            )}
                          </span>
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
          ))}
        </div>
      )}

      {/* Create / Edit Combo Modal */}
      <Modal
        open={modalOpen}
        title={editing ? `Edit combo: ${editing.name}` : "Create model combo"}
        onClose={() => setModalOpen(false)}
        maxWidth="max-w-2xl"
      >
        <form onSubmit={handleSave} className="space-y-4">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Combo model ID
              </label>
              <input
                type="text"
                required
                placeholder="e.g. smart-free, fast-coding"
                value={name}
                onChange={(e) => setName(e.target.value)}
                className={`${INPUT_CLASS} font-mono`}
              />
              <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                Letters, numbers, hyphens, underscores, or dots
              </p>
            </div>

            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Routing strategy
              </label>
              <select
                value={strategy}
                onChange={(e) => setStrategy(e.target.value as ComboStrategy)}
                className={INPUT_CLASS}
              >
                <option value="fallback">Fallback — try in order</option>
                <option value="round-robin">Round Robin — rotate evenly</option>
                <option value="random">Random — distribute load</option>
              </select>
            </div>
          </div>

          {/* Ordered Chain */}
          <div className="space-y-2">
            <label className="block text-xs font-medium text-[var(--text-secondary)]">
              Ordered target models ({chain.length})
            </label>
            {chain.length === 0 ? (
              <div className="rounded-lg border border-dashed border-[var(--border-strong)] bg-[var(--bg-elevated)] p-3 text-center text-xs text-[var(--text-muted)]">
                Click models below to add them to this combo chain
              </div>
            ) : (
              <div className="space-y-1.5">
                {chain.map((item, idx) => {
                  const meta = catalogEntries.map.get(item.baseModel);
                  const supportedLevels = meta?.supportedLevels ?? [];
                  const hasMultipleLevels = supportedLevels.length > 0;
                  const warning = validateChainEntry(item.baseModel, item.level);
                  const wireSpec = formatComboModelSpec(item.baseModel, item.level);
                  const levelUnsupported =
                    !!item.level &&
                    ((hasMultipleLevels && !supportedLevels.includes(item.level)) ||
                      (!!meta && !hasMultipleLevels));

                  return (
                    <div
                      key={item.uid}
                      className={`flex flex-wrap items-center justify-between gap-2 rounded-lg border px-3 py-2 ${
                        warning
                          ? "border-[color-mix(in_srgb,var(--warning)_45%,transparent)] bg-[var(--warning-soft)]"
                          : "border-[var(--border-subtle)] bg-[var(--bg-elevated)]"
                      }`}
                    >
                      <div className="flex min-w-0 flex-1 items-center gap-2.5">
                        <span className="w-4 font-mono text-[11px] text-[var(--text-muted)]">
                          {idx + 1}.
                        </span>
                        <ProviderIcon
                          src={getProviderLogo(item.baseModel)}
                          name={item.baseModel}
                          size={22}
                        />
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="truncate text-xs font-medium text-[var(--text-primary)]">
                              {meta?.displayName || item.baseModel}
                              {item.level ? ` (${item.level})` : ""}
                            </span>
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
                          <div className="truncate font-mono text-[10px] text-[var(--text-muted)]">
                            {wireSpec}
                          </div>
                        </div>
                      </div>

                      <div className="flex shrink-0 items-center gap-1.5">
                        {/* Show level selector ONLY when the model has multiple supported levels */}
                        {hasMultipleLevels && (
                          <select
                            aria-label={`Thinking or power level for ${meta?.displayName || item.baseModel}`}
                            value={item.level ?? meta?.defaultLevel ?? supportedLevels[0]}
                            onChange={(e) => updateChainItemLevel(item.uid, e.target.value)}
                            className="rounded-md border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-[11px] text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
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
                        {!hasMultipleLevels && item.level && (
                          <button
                            type="button"
                            onClick={() => updateChainItemLevel(item.uid, null)}
                            className="rounded border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--warning)] hover:text-[var(--text-primary)]"
                            title="Clear unsupported level suffix"
                          >
                            Clear ({item.level})
                          </button>
                        )}
                        <button
                          type="button"
                          disabled={idx === 0}
                          onClick={() => moveInChain(idx, -1)}
                          className="rounded p-1 text-[var(--text-muted)] hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)] disabled:opacity-30"
                          title="Move up"
                        >
                          <ArrowUp className="h-3.5 w-3.5" />
                        </button>
                        <button
                          type="button"
                          disabled={idx === chain.length - 1}
                          onClick={() => moveInChain(idx, 1)}
                          className="rounded p-1 text-[var(--text-muted)] hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)] disabled:opacity-30"
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
                  );
                })}
              </div>
            )}
          </div>

          {/* Add From Catalog or Custom */}
          <div className="space-y-2 border-t border-[var(--border-subtle)] pt-3">
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-[var(--text-secondary)]">
                Add models from catalog
              </span>
              <input
                type="text"
                placeholder="Filter catalog..."
                value={pickerSearch}
                onChange={(e) => setPickerSearch(e.target.value)}
                className={`${INPUT_CLASS} max-w-[200px] py-1`}
              />
            </div>

            <div className="flex max-h-40 flex-wrap gap-1.5 overflow-y-auto rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-2.5">
              {pickerFiltered.length === 0 ? (
                <span className="p-1 text-xs text-[var(--text-muted)]">
                  No matching catalog models available.
                </span>
              ) : (
                pickerFiltered.map((meta) => (
                  <button
                    key={meta.prefixedId}
                    type="button"
                    onClick={() => addModelToChain(meta.prefixedId)}
                    title={
                      meta.supportedLevels.length > 0
                        ? `Supports levels: ${meta.supportedLevels.join(", ")} (click to add another tier)`
                        : meta.prefixedId
                    }
                    className="inline-flex items-center gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2.5 py-1 text-[11px] text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:text-[var(--text-primary)]"
                  >
                    <Plus className="h-3 w-3 text-[var(--accent)]" />
                    <span className="font-mono">{meta.prefixedId}</span>
                    {meta.supportedLevels.length > 0 && (
                      <span className="rounded bg-[var(--bg-elevated)] px-1 py-0.2 font-mono text-[9px] text-[var(--accent)]">
                        {meta.supportedLevels.length} lvls
                      </span>
                    )}
                  </button>
                ))
              )}
            </div>

            <div className="flex gap-2">
              <input
                type="text"
                placeholder="Or add custom model spec (e.g. ocz/claude-sonnet-4)..."
                value={customModelInput}
                onChange={(e) => setCustomModelInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addModelToChain(customModelInput);
                    setCustomModelInput("");
                  }
                }}
                className={`${INPUT_CLASS} flex-1 font-mono`}
              />
              <Button
                variant="secondary"
                onClick={() => {
                  addModelToChain(customModelInput);
                  setCustomModelInput("");
                }}
              >
                Add
              </Button>
            </div>
          </div>

          <div className="flex justify-end gap-2 pt-2">
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
