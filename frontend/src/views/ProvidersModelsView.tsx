import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  ArrowRight,
  Brain,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  AlertCircle,
  Copy,
  ExternalLink,
  Gauge,
  Globe,
  KeyRound,
  LayoutGrid,
  Link2,
  List,
  Loader2,
  Lock,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  Sparkles,
  Trash2,
  UserCheck,
  Zap,
} from "lucide-react";
import { api } from "../api";
import { ProviderIcon } from "../components/ProviderIcon";
import { ProbeSettingsBar } from "../components/ProbeSettingsBar";
import {
  ModelCapabilityBadges,
  ModelCapabilityIcons,
  ModelLimitBadge,
} from "../components/ModelCapabilityBadges";
import {
  getModelCapabilities,
  getModelLimits,
  getModelCardClasses,
  getModelChipClasses,
} from "../utils/modelCapabilities";
import {
  ProviderProxySelector,
  type ProxySelectionValue,
} from "../components/ProviderProxySelector";
import { TableSkeleton } from "../components/Skeleton";
import {
  Button,
  Card,
  EmptyState,
  INPUT_CLASS,
  Modal,
  SegmentedControl,
  StatusPill,
} from "../components/UiPrimitives";
import type {
  AntigravityAccountView,
  CreateCustomProviderPayload,
  IpQuotaWithProxies,
  OAuthSessionPollStatus,
  OpenAiModelEntry,
  ProviderCatalogItem,
  ProviderConfig,
  ProviderTestResult,
  ProxyWithQuota,
  QuotaProbeStatus,
  RoutingStrategy,
  StartedOAuthFlow,
  ThinkingLevel,
  UpdateCustomProviderPayload,
  UpdateProviderConfigPayload,
} from "../types";
import {
  formatCooldownReason,
  formatCountdown,
  formatTimeAgo,
  proxyModeLabel,
  routingStrategyLabel,
} from "../utils/format";

type ConnectTab = "browser" | "callback" | "token";

function normalizeStrategyUi(raw: string | undefined | null, providerId: string): RoutingStrategy {
  const clean = (raw || "").trim().toLowerCase();
  if (clean === "round-robin" || clean === "round_robin" || clean === "rr") {
    return "round-robin";
  }
  if (clean === "random" || clean === "rand") {
    return "random";
  }
  if (clean === "fallback" || clean === "smart_fallback" || clean === "smart-fallback") {
    return "fallback";
  }
  return providerId === "opencode" || providerId === "ovh" ? "round-robin" : "fallback";
}

function suggestPrefixFromNameOrUrl(name: string, baseUrl: string): string {
  const source = name.trim() || (() => {
    try {
      return new URL(baseUrl.trim()).hostname.replace(/^www\./, "").split(".")[0] || "";
    } catch {
      return "";
    }
  })();
  return source
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 16);
}

function parseLineList(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const line of raw.split(/[\r\n,]+/)) {
    const clean = line.trim();
    if (clean && !seen.has(clean)) {
      seen.add(clean);
      out.push(clean);
    }
  }
  return out;
}

function parseHeadersText(raw: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of raw.split(/\r?\n/)) {
    const idx = line.indexOf(":");
    if (idx <= 0) continue;
    const k = line.slice(0, idx).trim();
    const v = line.slice(idx + 1).trim();
    if (k && v) {
      out[k] = v;
    }
  }
  return out;
}

function configToProxySelection(cfg: ProviderConfig): ProxySelectionValue {
  const mode = (cfg.proxy_mode || "all").toLowerCase();
  if (mode === "direct" || mode === "none") {
    return { proxyMode: "direct", selectedProxyIds: [] };
  }
  if (mode === "selected") {
    if (!cfg.selected_proxy_ids || cfg.selected_proxy_ids.length === 0) {
      return { proxyMode: "direct", selectedProxyIds: [] };
    }
    return {
      proxyMode: "selected",
      selectedProxyIds: cfg.selected_proxy_ids,
    };
  }
  return { proxyMode: "all", selectedProxyIds: [] };
}

export function ProvidersModelsView({
  providers,
  proxies,
  models,
  ipQuotas,
  antigravityAccounts,
  initialProviderId,
  onSelectProvider,
  lastQuotaSyncAt,
  quotaProbeStatus,
  probingQuotas,
  loading,
  nowMs,
  onTestModel,
  onOpenQuotaTracker,
  onOpenProxyPool,
  onRefreshQuotas,
  onProbeQuotasNow,
  onUpdateProviderConfig,
  onTestProvider,
  onStartAntigravityOAuth,
  onPollAntigravityOAuth,
  onCompleteAntigravityOAuth,
  onImportAntigravityAccount,
  onRefreshAntigravityQuotas,
  onRefreshSingleAntigravityQuota,
  onUpdateAccountConnection,
  onDeleteAccountConnection,
  onCreateCustomProvider,
  onUpdateCustomProvider,
  onDeleteCustomProvider,
  onRefreshCustomFavicon,
  onFetchCustomModels,
}: {
  providers: ProviderCatalogItem[];
  proxies: ProxyWithQuota[];
  models: OpenAiModelEntry[];
  ipQuotas: IpQuotaWithProxies[];
  antigravityAccounts: AntigravityAccountView[];
  initialProviderId?: string | null;
  onSelectProvider?: (providerId: string | null) => void;
  lastQuotaSyncAt: number;
  quotaProbeStatus?: QuotaProbeStatus | null;
  probingQuotas?: boolean;
  loading: boolean;
  nowMs: number;
  onTestModel: (modelId: string) => void;
  onOpenQuotaTracker: (providerId: string) => void;
  onOpenProxyPool: () => void;
  onRefreshQuotas: () => Promise<void>;
  onProbeQuotasNow?: () => Promise<void>;
  onUpdateProviderConfig: (
    id: string,
    payload: UpdateProviderConfigPayload,
  ) => Promise<void>;
  onTestProvider: (id: string) => Promise<ProviderTestResult>;
  onStartAntigravityOAuth: () => Promise<StartedOAuthFlow>;
  onPollAntigravityOAuth: (state: string) => Promise<OAuthSessionPollStatus>;
  onCompleteAntigravityOAuth: (payload: {
    callback_url: string;
    redirect_uri?: string;
  }) => Promise<void>;
  onImportAntigravityAccount: (payload: {
    name?: string;
    email?: string;
    refresh_token: string;
    project_id?: string;
  }) => Promise<void>;
  onRefreshAntigravityQuotas: () => Promise<void>;
  onRefreshSingleAntigravityQuota: (id: string) => Promise<void>;
  onUpdateAccountConnection: (
    id: string,
    payload: { name?: string; enabled?: boolean; reset_cooldown?: boolean },
  ) => Promise<void>;
  onDeleteAccountConnection: (id: string, label: string) => Promise<void>;
  onCreateCustomProvider?: (payload: CreateCustomProviderPayload) => Promise<string | void>;
  onUpdateCustomProvider?: (
    id: string,
    payload: UpdateCustomProviderPayload,
  ) => Promise<void>;
  onDeleteCustomProvider?: (id: string, name: string) => Promise<void>;
  onRefreshCustomFavicon?: (id: string) => Promise<void>;
  onFetchCustomModels?: (id: string) => Promise<void>;
}) {
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(
    initialProviderId ?? null,
  );
  const [search, setSearch] = useState("");
  const [modelsViewMode, setModelsViewMode] = useState<"table" | "list">(() => {
    try {
      const saved = localStorage.getItem("tesie_provider_models_view");
      if (saved === "table" || saved === "list") return saved;
    } catch {
      // ignore
    }
    return "table";
  });
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [testingProvider, setTestingProvider] = useState(false);
  const [syncingQuotas, setSyncingQuotas] = useState(false);
  const [testResults, setTestResults] = useState<Record<string, ProviderTestResult>>({});
  const [expandedAccountIds, setExpandedAccountIds] = useState<Set<string>>(new Set());

  function toggleAccountExpanded(id: string) {
    setExpandedAccountIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  // Provider config form state
  const [proxySelection, setProxySelection] = useState<ProxySelectionValue>({
    proxyMode: "all",
    selectedProxyIds: [],
  });
  const [winMinutes, setWinMinutes] = useState("60");
  const [maxReqs, setMaxReqs] = useState("");
  const [maxToks, setMaxToks] = useState("");
  const [sessionTtlHours, setSessionTtlHours] = useState("4");
  const [savingCfg, setSavingCfg] = useState(false);

  // Antigravity Connect Modal & OAuth state
  const [connectOpen, setConnectOpen] = useState(false);
  const [connectTab, setConnectTab] = useState<ConnectTab>("browser");
  const [oauthSession, setOauthSession] = useState<StartedOAuthFlow | null>(null);
  const [oauthPolling, setOauthPolling] = useState(false);
  const [oauthError, setOauthError] = useState<string | null>(null);
  const [callbackInput, setCallbackInput] = useState("");
  const [completingCallback, setCompletingCallback] = useState(false);

  const [importName, setImportName] = useState("");
  const [importEmail, setImportEmail] = useState("");
  const [importRefreshToken, setImportRefreshToken] = useState("");
  const [importProjectId, setImportProjectId] = useState("");
  const [importingToken, setImportingToken] = useState(false);

  const [refreshingAllAg, setRefreshingAllAg] = useState(false);
  const [refreshingAccountId, setRefreshingAccountId] = useState<string | null>(null);

  // Add / Edit Custom Provider modal state
  const [customModalOpen, setCustomModalOpen] = useState(false);
  const [editingCustomId, setEditingCustomId] = useState<string | null>(null);
  const [cpName, setCpName] = useState("");
  const [cpPrefix, setCpPrefix] = useState("");
  const [cpPrefixTouched, setCpPrefixTouched] = useState(false);
  const [cpProtocol, setCpProtocol] = useState<"openai" | "anthropic">("openai");
  const [cpApiType, setCpApiType] = useState<"chat" | "responses">("chat");
  const [cpBaseUrl, setCpBaseUrl] = useState("");
  const [cpApiKeysText, setCpApiKeysText] = useState("");
  const [cpModelsText, setCpModelsText] = useState("");
  const [cpHeadersText, setCpHeadersText] = useState("");
  const [cpColor, setCpColor] = useState("#0EA5E9");
  const [cpFaviconUrl, setCpFaviconUrl] = useState<string | null>(null);
  const [discoveringFavicon, setDiscoveringFavicon] = useState(false);
  const [cpFaviconHint, setCpFaviconHint] = useState<string | null>(null);
  const [fetchingPreviewModels, setFetchingPreviewModels] = useState(false);
  const [savingCustom, setSavingCustom] = useState(false);
  const [cpModalError, setCpModalError] = useState<string | null>(null);
  const [fetchingSubpageModels, setFetchingSubpageModels] = useState(false);
  const [refreshingSubpageIcon, setRefreshingSubpageIcon] = useState(false);

  const pollTimerRef = useRef<number | null>(null);
  const pollCallbackRef = useRef(onPollAntigravityOAuth);
  pollCallbackRef.current = onPollAntigravityOAuth;
  const oauthPopupRef = useRef<Window | null>(null);
  const openedAccountsSigRef = useRef<string>("");

  useEffect(() => {
    setSelectedProviderId(initialProviderId ?? null);
  }, [initialProviderId]);

  function chooseProvider(nextId: string | null) {
    setSelectedProviderId(nextId);
    setSearch("");
    onSelectProvider?.(nextId);
  }

  const activeProvider = useMemo(() => {
    if (!selectedProviderId) return null;
    return providers.find((p) => p.id === selectedProviderId) ?? null;
  }, [providers, selectedProviderId]);

  // Debounced automatic website favicon lookup when entering Base URL in Custom Provider modal
  useEffect(() => {
    if (!customModalOpen) return;
    const raw = cpBaseUrl.trim();
    if (!/^https?:\/\/[^/\s]+/i.test(raw)) {
      if (!editingCustomId) {
        setCpFaviconUrl(null);
      }
      setCpFaviconHint(null);
      return;
    }

    let cancelled = false;
    setDiscoveringFavicon(true);
    const timer = window.setTimeout(async () => {
      try {
        const res = await api.discoverCustomProviderFavicon(raw);
        if (cancelled) return;
        if (res.favicon_url) {
          setCpFaviconUrl(res.favicon_url);
          setCpFaviconHint("Fetched website favicon automatically");
        } else {
          setCpFaviconUrl(null);
          setCpFaviconHint("No website favicon found — monogram badge will be used");
        }
      } catch {
        if (!cancelled) {
          setCpFaviconHint(null);
        }
      } finally {
        if (!cancelled) {
          setDiscoveringFavicon(false);
        }
      }
    }, 450);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      setDiscoveringFavicon(false);
    };
  }, [cpBaseUrl, customModalOpen, editingCustomId]);

  useEffect(() => {
    if (!activeProvider) return;
    setProxySelection(configToProxySelection(activeProvider.config));
    setWinMinutes(
      String(Math.max(1, Math.round(activeProvider.config.window_duration_ms / 60_000))),
    );
    setMaxReqs(
      activeProvider.config.max_requests_per_window
        ? String(activeProvider.config.max_requests_per_window)
        : "",
    );
    setMaxToks(
      activeProvider.config.max_tokens_per_window
        ? String(activeProvider.config.max_tokens_per_window)
        : "",
    );
    setSessionTtlHours(
      String(Math.max(1, Math.round(activeProvider.config.session_ttl_ms / 3_600_000))),
    );
  }, [activeProvider]);

  const accounts = useMemo(() => {
    if (antigravityAccounts.length > 0) return antigravityAccounts;
    const agProv = providers.find((p) => p.id === "antigravity");
    return agProv?.accounts ?? [];
  }, [antigravityAccounts, providers]);

  const accountsSig = useMemo(
    () => accounts.map((a) => `${a.id}:${a.email ?? ""}:${a.updated_at}`).join("|"),
    [accounts],
  );

  // Close OAuth modal immediately if a new/updated Antigravity account arrives via background refresh
  useEffect(() => {
    if (!connectOpen || !oauthSession) return;
    if (openedAccountsSigRef.current && accountsSig !== openedAccountsSigRef.current) {
      try {
        oauthPopupRef.current?.close();
      } catch {
        // ignore cross-origin popup close
      }
      oauthPopupRef.current = null;
      setOauthPolling(false);
      setOauthSession(null);
      setConnectOpen(false);
    }
  }, [accountsSig, connectOpen, oauthSession]);

  // Poll pending browser OAuth flow when active (isolated from 1s nowMs re-renders via pollCallbackRef)
  useEffect(() => {
    if (!connectOpen || !oauthSession || !oauthPolling) {
      if (pollTimerRef.current) {
        window.clearInterval(pollTimerRef.current);
        pollTimerRef.current = null;
      }
      return;
    }

    const stateId = oauthSession.state;
    let cancelled = false;

    const tick = async () => {
      try {
        const st = await pollCallbackRef.current(stateId);
        if (cancelled) return;
        if (st.status === "completed") {
          try {
            oauthPopupRef.current?.close();
          } catch {
            // ignore
          }
          oauthPopupRef.current = null;
          setOauthPolling(false);
          setOauthSession(null);
          setConnectOpen(false);
        } else if (st.status === "error") {
          setOauthPolling(false);
          setOauthError(st.error || "OAuth authorization failed");
        }
      } catch {
        // ignore transient poll glitch
      }
    };

    void tick();
    const id = window.setInterval(() => {
      void tick();
    }, 1000);
    pollTimerRef.current = id;

    return () => {
      cancelled = true;
      window.clearInterval(id);
      pollTimerRef.current = null;
    };
  }, [connectOpen, oauthSession, oauthPolling]);

  const externalPoolProxies = useMemo(
    () => proxies.filter((p) => !p.is_direct),
    [proxies],
  );

  // Calculate live Smart Proxy Distribution preview for Antigravity
  const smartSplitPreview = useMemo(() => {
    const selectedPoolCount =
      proxySelection.proxyMode === "direct"
        ? 0
        : proxySelection.proxyMode === "all"
          ? externalPoolProxies.length
          : proxySelection.selectedProxyIds.length;
    const enabledAccts = accounts.filter((a) => a.enabled);
    const acctCount = Math.max(1, enabledAccts.length);
    if (selectedPoolCount === 0) {
      return {
        selectedPoolCount: 0,
        acctCount: enabledAccts.length,
        sliceText: "Direct Connection (0 proxies selected)",
        shared: false,
      };
    }
    if (enabledAccts.length === 0) {
      return {
        selectedPoolCount,
        acctCount: 0,
        sliceText: `${selectedPoolCount} proxies ready — e.g. 2 accounts will split ${Math.floor(selectedPoolCount / 2)} / ${selectedPoolCount - Math.floor(selectedPoolCount / 2)} dedicated nodes`,
        shared: false,
      };
    }
    if (selectedPoolCount >= acctCount) {
      const base = Math.floor(selectedPoolCount / acctCount);
      const rem = selectedPoolCount % acctCount;
      const slices = enabledAccts.map((_, idx) => base + (idx < rem ? 1 : 0));
      return {
        selectedPoolCount,
        acctCount,
        sliceText: `${slices.join(" / ")} dedicated proxies per account · Sticky Fallback, No Round-Robin`,
        shared: false,
      };
    }
    return {
      selectedPoolCount,
      acctCount,
      sliceText: `Only ${selectedPoolCount} proxy for ${acctCount} accounts — add ${acctCount - selectedPoolCount} more proxy to isolate accounts`,
      shared: true,
    };
  }, [proxySelection, externalPoolProxies.length, accounts]);

  // Aggregate Antigravity unlock + quota status across enabled accounts for model catalog rendering
  const agModelStateMap = useMemo(() => {
    const map = new Map<string, { unlocked: boolean; maxRemaining: number }>();
    const enabledAccts = accounts.filter((a) => a.enabled && a.quota?.models?.length);
    if (enabledAccts.length === 0) return map;
    for (const acct of enabledAccts) {
      for (const mq of acct.quota?.models ?? []) {
        const key = mq.model.toLowerCase();
        const prev = map.get(key);
        const isUnlocked = mq.unlocked !== false;
        const pct = isUnlocked ? mq.remaining_percentage : 0;
        if (!prev) {
          map.set(key, { unlocked: isUnlocked, maxRemaining: pct });
        } else {
          map.set(key, {
            unlocked: prev.unlocked || isUnlocked,
            maxRemaining: Math.max(prev.maxRemaining, pct),
          });
        }
      }
    }
    return map;
  }, [accounts]);

  const modelsEntryMap = useMemo(() => {
    const map = new Map<string, OpenAiModelEntry>();
    for (const m of models) {
      map.set(m.id.toLowerCase(), m);
      const slash = m.id.indexOf("/");
      if (slash >= 0) {
        map.set(m.id.slice(slash + 1).toLowerCase(), m);
      }
    }
    return map;
  }, [models]);

  const providerModels = useMemo(() => {
    if (!activeProvider) return [];
    const prefixSlash = `${activeProvider.prefix}/`;
    const fromOpenAi = models.filter((m) => m.owned_by === activeProvider.id);
    if (fromOpenAi.length > 0) {
      return fromOpenAi.map((m) =>
        m.id.startsWith(prefixSlash) ? m.id : `${prefixSlash}${m.id}`,
      );
    }
    return (activeProvider.models ?? []).map((id) =>
      id.startsWith(prefixSlash) ? id : `${prefixSlash}${id}`,
    );
  }, [models, activeProvider]);

  const currentThinkingLevel = (
    activeProvider?.config.thinking_level || "auto"
  ).toLowerCase() as ThinkingLevel;

  if (loading && providers.length === 0) {
    return <TableSkeleton rows={6} />;
  }

  function openNewCustomModal() {
    setEditingCustomId(null);
    setCpName("");
    setCpPrefix("");
    setCpPrefixTouched(false);
    setCpProtocol("openai");
    setCpApiType("chat");
    setCpBaseUrl("");
    setCpApiKeysText("");
    setCpModelsText("");
    setCpHeadersText("");
    setCpColor("#0EA5E9");
    setCpFaviconUrl(null);
    setCpFaviconHint(null);
    setCpModalError(null);
    setCustomModalOpen(true);
  }

  function openEditCustomModal(prov: ProviderCatalogItem) {
    const cp = prov.custom_provider;
    if (!cp) return;
    setEditingCustomId(cp.id);
    setCpName(cp.name);
    setCpPrefix(cp.prefix);
    setCpPrefixTouched(true);
    setCpProtocol(cp.protocol === "anthropic" ? "anthropic" : "openai");
    setCpApiType(cp.api_type === "responses" ? "responses" : "chat");
    setCpBaseUrl(cp.base_url);
    setCpApiKeysText((cp.api_keys ?? []).join("\n"));
    setCpModelsText((cp.models ?? []).join("\n"));
    setCpHeadersText(
      Object.entries(cp.custom_headers ?? {})
        .map(([k, v]) => `${k}: ${v}`)
        .join("\n"),
    );
    setCpColor(cp.color || "#0EA5E9");
    setCpFaviconUrl(cp.favicon_url ?? prov.favicon_url ?? null);
    setCpFaviconHint(null);
    setCpModalError(null);
    setCustomModalOpen(true);
  }

  async function handleManualDetectFavicon() {
    const raw = cpBaseUrl.trim();
    if (!raw) return;
    setDiscoveringFavicon(true);
    setCpFaviconHint(null);
    try {
      const res = await api.discoverCustomProviderFavicon(raw);
      if (res.favicon_url) {
        setCpFaviconUrl(res.favicon_url);
        setCpFaviconHint("Fetched website favicon");
      } else {
        setCpFaviconUrl(null);
        setCpFaviconHint("No icon found on origin — monogram badge will be used");
      }
    } finally {
      setDiscoveringFavicon(false);
    }
  }

  async function handleFetchPreviewModels() {
    if (!cpBaseUrl.trim()) {
      setCpModalError("Enter a valid Base URL before fetching /models.");
      return;
    }
    setCpModalError(null);
    setFetchingPreviewModels(true);
    try {
      const discovered = await api.fetchCustomProviderModelsPreview({
        base_url: cpBaseUrl.trim(),
        protocol: cpProtocol,
        api_type: cpProtocol === "openai" ? cpApiType : "chat",
        api_keys: parseLineList(cpApiKeysText),
        custom_headers: parseHeadersText(cpHeadersText),
        proxy_mode: activeProvider?.config.proxy_mode,
      });
      if (discovered.length === 0) {
        setCpModalError("Endpoint responded but returned zero models.");
      } else {
        const merged = parseLineList([cpModelsText, discovered.join("\n")].filter(Boolean).join("\n"));
        setCpModelsText(merged.join("\n"));
      }
    } catch (err) {
      setCpModalError(err instanceof Error ? err.message : "Failed to fetch /models");
    } finally {
      setFetchingPreviewModels(false);
    }
  }

  async function handleSaveCustomProvider(e: React.FormEvent) {
    e.preventDefault();
    setCpModalError(null);
    const cleanName = cpName.trim();
    const cleanPrefix = (cpPrefix.trim() || suggestPrefixFromNameOrUrl(cleanName, cpBaseUrl)).toLowerCase();
    const cleanBase = cpBaseUrl.trim();
    if (!cleanName || !cleanPrefix || !cleanBase) {
      setCpModalError("Name, Prefix, and Base URL are required.");
      return;
    }

    const keys = parseLineList(cpApiKeysText);
    const mods = parseLineList(cpModelsText);
    const hdrs = parseHeadersText(cpHeadersText);

    setSavingCustom(true);
    try {
      if (editingCustomId && onUpdateCustomProvider) {
        await onUpdateCustomProvider(editingCustomId, {
          name: cleanName,
          prefix: cleanPrefix,
          protocol: cpProtocol,
          api_type: cpProtocol === "openai" ? cpApiType : "chat",
          base_url: cleanBase,
          api_keys: keys,
          models: mods,
          custom_headers: hdrs,
          favicon_url: cpFaviconUrl,
          color: cpColor,
        });
      } else if (onCreateCustomProvider) {
        const createdId = await onCreateCustomProvider({
          name: cleanName,
          prefix: cleanPrefix,
          protocol: cpProtocol,
          api_type: cpProtocol === "openai" ? cpApiType : "chat",
          base_url: cleanBase,
          api_keys: keys,
          models: mods,
          custom_headers: hdrs,
          favicon_url: cpFaviconUrl,
          color: cpColor,
          enabled: true,
        });
        if (createdId) {
          chooseProvider(createdId);
        }
      }
      setCustomModalOpen(false);
    } catch (err) {
      setCpModalError(err instanceof Error ? err.message : "Failed to save custom provider");
    } finally {
      setSavingCustom(false);
    }
  }

  const customProviderModal = (
    <Modal
      open={customModalOpen}
      title={editingCustomId ? "Edit Custom Provider" : "Add Custom Provider"}
      maxWidth="max-w-2xl"
      onClose={() => setCustomModalOpen(false)}
    >
      <form onSubmit={handleSaveCustomProvider} className="space-y-4">
        {cpModalError && (
          <div className="rounded-xl border border-[color-mix(in_srgb,var(--danger)_35%,transparent)] bg-[var(--danger-soft)] px-3.5 py-2.5 text-xs text-[var(--danger)]">
            {cpModalError}
          </div>
        )}

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              Provider Name
            </label>
            <input
              type="text"
              required
              placeholder="e.g. Logfare AI"
              value={cpName}
              onChange={(e) => {
                const nextName = e.target.value;
                setCpName(nextName);
                if (!editingCustomId && !cpPrefixTouched) {
                  setCpPrefix(suggestPrefixFromNameOrUrl(nextName, cpBaseUrl));
                }
              }}
              className={INPUT_CLASS}
            />
          </div>

          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              Model Prefix (routes as{" "}
              <code className="font-mono">{cpPrefix || "prefix"}/model</code>)
            </label>
            <input
              type="text"
              required
              placeholder="e.g. lf"
              value={cpPrefix}
              onChange={(e) => {
                setCpPrefixTouched(true);
                setCpPrefix(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ""));
              }}
              className={`${INPUT_CLASS} font-mono`}
            />
          </div>
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              Upstream Protocol
            </label>
            <SegmentedControl<"openai" | "anthropic">
              size="sm"
              value={cpProtocol}
              onChange={setCpProtocol}
              options={[
                { value: "openai", label: "OpenAI Compatible" },
                { value: "anthropic", label: "Anthropic Compatible" },
              ]}
            />
          </div>

          {cpProtocol === "openai" && (
            <div>
              <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                Wire API Format
              </label>
              <SegmentedControl<"chat" | "responses">
                size="sm"
                value={cpApiType}
                onChange={setCpApiType}
                options={[
                  { value: "chat", label: "/chat/completions" },
                  { value: "responses", label: "/responses" },
                ]}
              />
            </div>
          )}
        </div>

        <div>
          <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
            Base URL
          </label>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]">
              {discoveringFavicon ? (
                <Loader2 className="h-4 w-4 animate-spin text-[var(--accent)]" />
              ) : (
                <ProviderIcon
                  src={cpFaviconUrl || undefined}
                  name={cpName || cpPrefix || "CP"}
                  color={cpColor}
                  size={30}
                />
              )}
            </div>
            <input
              type="url"
              required
              placeholder="https://logfare.ai/v1"
              value={cpBaseUrl}
              onChange={(e) => {
                const nextUrl = e.target.value;
                setCpBaseUrl(nextUrl);
                if (!editingCustomId && !cpPrefixTouched && !cpName.trim()) {
                  setCpPrefix(suggestPrefixFromNameOrUrl("", nextUrl));
                }
              }}
              className={`${INPUT_CLASS} flex-1 font-mono`}
            />
            <Button
              type="button"
              variant="secondary"
              size="md"
              disabled={!cpBaseUrl.trim() || discoveringFavicon}
              onClick={() => void handleManualDetectFavicon()}
            >
              <Globe className="h-3.5 w-3.5" />
              Detect Icon
            </Button>
          </div>
          {cpFaviconHint && (
            <div className="mt-1 text-[11px] text-[var(--text-muted)]">{cpFaviconHint}</div>
          )}
        </div>

        <div>
          <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
            API Keys (one per line — multiple keys rotate automatically)
          </label>
          <textarea
            rows={2}
            placeholder="sk-..."
            value={cpApiKeysText}
            onChange={(e) => setCpApiKeysText(e.target.value)}
            className={`${INPUT_CLASS} font-mono text-xs`}
          />
        </div>

        <div>
          <div className="mb-1 flex items-center justify-between gap-2">
            <label className="text-xs font-medium text-[var(--text-secondary)]">
              Models (one per line)
            </label>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={fetchingPreviewModels || !cpBaseUrl.trim()}
              onClick={() => void handleFetchPreviewModels()}
            >
              {fetchingPreviewModels ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="h-3.5 w-3.5" />
              )}
              Fetch /models
            </Button>
          </div>
          <textarea
            rows={3}
            placeholder={"gpt-4o-mini\nclaude-sonnet-4-5"}
            value={cpModelsText}
            onChange={(e) => setCpModelsText(e.target.value)}
            className={`${INPUT_CLASS} font-mono text-xs`}
          />
        </div>

        <details className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/45 px-3.5 py-2.5">
          <summary className="cursor-pointer text-xs font-medium text-[var(--text-secondary)] select-none">
            Optional custom headers &amp; badge color
          </summary>
          <div className="mt-3 space-y-3 border-t border-[var(--border-subtle)] pt-3">
            <div>
              <label className="mb-1 block text-xs text-[var(--text-muted)]">
                Custom Headers (<code className="font-mono">Header-Name: value</code> per line)
              </label>
              <textarea
                rows={2}
                placeholder="HTTP-Referer: https://myapp.local"
                value={cpHeadersText}
                onChange={(e) => setCpHeadersText(e.target.value)}
                className={`${INPUT_CLASS} font-mono text-xs`}
              />
            </div>
            <div className="flex items-center gap-3">
              <label className="text-xs text-[var(--text-muted)]">Fallback Badge Color</label>
              <input
                type="color"
                value={cpColor}
                onChange={(e) => setCpColor(e.target.value)}
                className="h-8 w-12 cursor-pointer rounded border border-[var(--border-subtle)] bg-transparent"
              />
            </div>
          </div>
        </details>

        <div className="flex justify-end gap-2 pt-2">
          <Button variant="secondary" size="md" onClick={() => setCustomModalOpen(false)}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" size="md" disabled={savingCustom}>
            <Save className="h-3.5 w-3.5" />
            {savingCustom
              ? "Saving…"
              : editingCustomId
                ? "Save Provider"
                : "Create Provider"}
          </Button>
        </div>
      </form>
    </Modal>
  );

  // ================= PROVIDERS DIRECTORY (NO PROVIDER SELECTED YET) =================
  if (!activeProvider) {
    return (
      <>
        <div className="space-y-4 animate-tab-switch">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-5 py-3.5">
            <div className="min-w-0">
              <div className="text-sm font-semibold text-[var(--text-primary)]">
                Configured Providers ({providers.length})
              </div>
              <div className="text-xs text-[var(--text-muted)]">
                Select a provider to manage routing strategies, proxy slices, credentials, or
                compatible custom endpoints.
              </div>
            </div>
            {onCreateCustomProvider && (
              <Button variant="primary" size="sm" onClick={openNewCustomModal}>
                <Plus className="h-4 w-4" />
                Add Custom Provider
              </Button>
            )}
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {providers.map((prov) => {
              const isAg = prov.id === "antigravity";
              const isCustomProv = Boolean(prov.is_custom);
              const readyUnits = prov.active_units;
              const totalUnits = prov.total_units;
              const stratLabel = routingStrategyLabel(
                prov.config.routing_strategy ?? prov.routing_strategy ?? prov.selection_strategy,
              );
              return (
                <button
                  key={prov.id}
                  type="button"
                  onClick={() => chooseProvider(prov.id)}
                  className="group flex flex-col justify-between gap-4 rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-5 text-left transition-[border-color,background-color] duration-150 hover:border-[var(--border-strong)] hover:bg-[var(--bg-hover)]/35"
                >
                  <div className="flex min-w-0 items-center gap-3.5">
                    <ProviderIcon
                      src={prov.favicon_url || prov.logo}
                      name={prov.name}
                      color={prov.color}
                      size={44}
                    />
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="truncate text-base font-semibold text-[var(--text-primary)] transition-colors group-hover:text-[var(--accent)]">
                          {prov.name}
                        </span>
                        <span className="font-mono text-xs text-[var(--text-muted)]">
                          {prov.prefix}/*
                        </span>
                      </div>
                      <div className="mt-0.5 truncate text-xs text-[var(--text-muted)]">
                        {isAg
                          ? `Cloud Code OAuth · ${stratLabel}`
                          : isCustomProv
                            ? `${(prov.protocol || "openai").toUpperCase()} (${prov.api_type || "chat"}) · ${stratLabel}`
                            : `Anonymous Free Pool · ${stratLabel}`}
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center justify-between border-t border-[var(--border-subtle)] pt-3.5 text-[13px] tabular-nums">
                    <div>
                      {!prov.config.enabled ? (
                        <StatusPill tone="neutral">Disabled</StatusPill>
                      ) : isAg ? (
                        totalUnits === 0 ? (
                          <StatusPill tone="neutral">No accounts</StatusPill>
                        ) : readyUnits > 0 ? (
                          <StatusPill tone="success" dot>
                            {readyUnits}/{totalUnits} ready
                          </StatusPill>
                        ) : (
                          <StatusPill tone="warning" dot>
                            {totalUnits} cooling
                          </StatusPill>
                        )
                      ) : isCustomProv ? (
                        <StatusPill tone={totalUnits > 0 ? "success" : "warning"} dot>
                          {totalUnits} {totalUnits === 1 ? "key" : "keys"} &middot;{" "}
                          {proxyModeLabel(
                            prov.config.proxy_mode,
                            prov.config.selected_proxy_ids?.length,
                          )}
                        </StatusPill>
                      ) : prov.config.proxy_mode === "direct" ? (
                        <StatusPill tone="warning" dot>
                          Direct host
                        </StatusPill>
                      ) : prov.healthy_proxies > 0 ? (
                        <StatusPill tone="success" dot>
                          {prov.healthy_proxies} proxies ready
                        </StatusPill>
                      ) : (
                        <StatusPill tone="warning" dot>
                          Direct fallback
                        </StatusPill>
                      )}
                    </div>

                    <span className="inline-flex items-center gap-1 text-[var(--text-muted)] transition-colors group-hover:text-[var(--accent)]">
                      <span>{prov.models.length} models</span>
                      <ArrowRight className="h-4 w-4 transition-transform group-hover:translate-x-0.5" />
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {customProviderModal}
      </>
    );
  }

  const isAntigravity = activeProvider.id === "antigravity";
  const isCustom = Boolean(activeProvider.is_custom);
  const currentRoutingStrategy = normalizeStrategyUi(
    activeProvider.config.routing_strategy ??
      activeProvider.routing_strategy ??
      activeProvider.selection_strategy,
    activeProvider.id,
  );
  const currentTestResult = testResults[activeProvider.id] ?? null;

  function copyText(id: string, val: string) {
    void navigator.clipboard.writeText(val);
    setCopiedId(id);
    setTimeout(() => setCopiedId((cur) => (cur === id ? null : cur)), 1500);
  }

  async function handleRunTest() {
    if (!activeProvider) return;
    setTestingProvider(true);
    try {
      const res = await onTestProvider(activeProvider.id);
      setTestResults((prev) => ({ ...prev, [activeProvider.id]: res }));
    } finally {
      setTestingProvider(false);
    }
  }

  async function handleManualQuotaRefresh() {
    setSyncingQuotas(true);
    try {
      await onRefreshQuotas();
    } finally {
      setSyncingQuotas(false);
    }
  }

  async function handleSaveConfig(e: React.FormEvent) {
    e.preventDefault();
    if (!activeProvider) return;
    setSavingCfg(true);
    try {
      const payload: UpdateProviderConfigPayload = {
        proxy_mode: proxySelection.proxyMode,
        selected_proxy_ids:
          proxySelection.proxyMode === "selected"
            ? proxySelection.selectedProxyIds
            : [],
      };
      if (!isAntigravity && !isCustom) {
        payload.window_duration_ms = Math.max(1, Number(winMinutes) || 60) * 60_000;
        payload.max_requests_per_window = maxReqs.trim() ? Number(maxReqs) : null;
        payload.max_tokens_per_window = maxToks.trim() ? Number(maxToks) : null;
        payload.session_ttl_ms = Math.max(1, Number(sessionTtlHours) || 4) * 3_600_000;
      }
      await onUpdateProviderConfig(activeProvider.id, payload);
    } finally {
      setSavingCfg(false);
    }
  }

  function openConnectModal() {
    openedAccountsSigRef.current = accountsSig;
    setOauthError(null);
    setOauthSession(null);
    setOauthPolling(false);
    setConnectOpen(true);
  }

  async function handleStartBrowserOAuth() {
    setOauthError(null);
    openedAccountsSigRef.current = accountsSig;
    // Open popup synchronously on click gesture so browser popup blockers never suppress Google OAuth
    const popup = window.open("about:blank", "_blank");
    oauthPopupRef.current = popup;
    try {
      const flow = await onStartAntigravityOAuth();
      setOauthSession(flow);
      setOauthPolling(true);
      if (popup && !popup.closed) {
        popup.location.href = flow.auth_url;
      } else {
        oauthPopupRef.current = window.open(flow.auth_url, "_blank");
      }
    } catch (err) {
      try {
        popup?.close();
      } catch {
        // ignore
      }
      oauthPopupRef.current = null;
      setOauthError(err instanceof Error ? err.message : "Failed to start Google OAuth");
    }
  }

  async function handleCompleteManualCallback(e: React.FormEvent) {
    e.preventDefault();
    if (!callbackInput.trim()) return;
    setCompletingCallback(true);
    setOauthError(null);
    try {
      await onCompleteAntigravityOAuth({
        callback_url: callbackInput.trim(),
        redirect_uri: oauthSession?.redirect_uri,
      });
      setCallbackInput("");
      setOauthPolling(false);
      setOauthSession(null);
      setConnectOpen(false);
    } catch (err) {
      setOauthError(err instanceof Error ? err.message : "OAuth code exchange failed");
    } finally {
      setCompletingCallback(false);
    }
  }

  async function handleImportTokenSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!importRefreshToken.trim()) return;
    setImportingToken(true);
    setOauthError(null);
    try {
      await onImportAntigravityAccount({
        name: importName.trim() || undefined,
        email: importEmail.trim() || undefined,
        refresh_token: importRefreshToken.trim(),
        project_id: importProjectId.trim() || undefined,
      });
      setImportName("");
      setImportEmail("");
      setImportRefreshToken("");
      setImportProjectId("");
      setConnectOpen(false);
    } catch (err) {
      setOauthError(err instanceof Error ? err.message : "Refresh token import failed");
    } finally {
      setImportingToken(false);
    }
  }

  async function handleRefreshAllAg() {
    setRefreshingAllAg(true);
    try {
      await onRefreshAntigravityQuotas();
    } finally {
      setRefreshingAllAg(false);
    }
  }

  async function handleRefreshOneAg(id: string) {
    setRefreshingAccountId(id);
    try {
      await onRefreshSingleAntigravityQuota(id);
    } finally {
      setRefreshingAccountId(null);
    }
  }

  const isProbingLive = Boolean(probingQuotas || quotaProbeStatus?.probe_running);
  const activeQuotas = ipQuotas.filter(
    (q) => !(q.status === "cooldown" && q.cooldown_until && q.cooldown_until > nowMs),
  ).length;
  const coolingQuotas = ipQuotas.length - activeQuotas;

  function handleChangeModelsView(next: "table" | "list") {
    setModelsViewMode(next);
    try {
      localStorage.setItem("tesie_provider_models_view", next);
    } catch {
      // ignore
    }
  }

  const qLower = search.trim().toLowerCase();
  const filteredModels = providerModels.filter(
    (full) => !qLower || full.toLowerCase().includes(qLower),
  );

  return (
    <div className="space-y-5 animate-tab-switch">
      {/* Unified Subpage Header Card (Breadcrumb + Identity + Actions + Probe Controls) */}
      <section className="overflow-hidden rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)]">
        <div className="flex flex-wrap items-center justify-between gap-4 px-5 py-4">
          <div className="flex min-w-0 items-center gap-3.5">
            <button
              type="button"
              onClick={() => chooseProvider(null)}
              className="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-2 text-[var(--text-secondary)] transition-colors hover:border-[var(--border-strong)] hover:text-[var(--text-primary)]"
              title="Back to Providers"
              aria-label="Back to Providers"
            >
              <ArrowLeft className="h-4 w-4" />
            </button>
            <ProviderIcon
              src={activeProvider.favicon_url || activeProvider.logo}
              name={activeProvider.name}
              color={activeProvider.color}
              size={40}
            />
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="text-base font-semibold text-[var(--text-primary)]">
                  {activeProvider.name}
                </h2>
                <span className="font-mono text-[11px] text-[var(--text-muted)]">
                  {activeProvider.prefix}/*
                </span>
                {!activeProvider.config.enabled ? (
                  <StatusPill tone="neutral">Disabled</StatusPill>
                ) : isAntigravity ? (
                  accounts.length === 0 ? (
                    <StatusPill tone="warning" dot>
                      Connect account
                    </StatusPill>
                  ) : (
                    <StatusPill tone="success" dot>
                      {accounts.filter((a) => a.enabled && a.status === "ready").length}/
                      {accounts.length} accounts ready
                    </StatusPill>
                  )
                ) : isCustom ? (
                  <StatusPill
                    tone={activeProvider.active_units > 0 ? "success" : "warning"}
                    dot
                  >
                    {activeProvider.active_units}{" "}
                    {activeProvider.active_units === 1 ? "API key" : "API keys"}
                  </StatusPill>
                ) : activeProvider.config.proxy_mode === "direct" ? (
                  <StatusPill tone="warning" dot>
                    Direct
                  </StatusPill>
                ) : activeProvider.healthy_proxies > 0 ? (
                  <StatusPill tone="success" dot>
                    {activeProvider.healthy_proxies} proxies healthy
                  </StatusPill>
                ) : (
                  <StatusPill tone="danger" dot>
                    No healthy proxies
                  </StatusPill>
                )}
              </div>
              <p className="mt-0.5 truncate text-xs text-[var(--text-muted)]">
                {activeProvider.description}
              </p>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {isAntigravity && (
              <Button variant="primary" size="sm" onClick={openConnectModal}>
                <Plus className="h-3.5 w-3.5" />
                Connect Google Account
              </Button>
            )}
            {isCustom && activeProvider.custom_provider && (
              <>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => openEditCustomModal(activeProvider)}
                >
                  <Pencil className="h-3.5 w-3.5" />
                  Edit
                </Button>
                {onRefreshCustomFavicon && (
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={refreshingSubpageIcon}
                    onClick={async () => {
                      setRefreshingSubpageIcon(true);
                      try {
                        await onRefreshCustomFavicon(activeProvider.id);
                      } finally {
                        setRefreshingSubpageIcon(false);
                      }
                    }}
                    title="Re-fetch website favicon from Base URL"
                  >
                    <Globe
                      className={`h-3.5 w-3.5 ${refreshingSubpageIcon ? "animate-spin" : ""}`}
                    />
                    Refresh Icon
                  </Button>
                )}
                {onDeleteCustomProvider && (
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={async () => {
                      await onDeleteCustomProvider(activeProvider.id, activeProvider.name);
                      chooseProvider(null);
                    }}
                  >
                    <Trash2 className="h-3.5 w-3.5 text-[var(--danger)]" />
                    Delete
                  </Button>
                )}
              </>
            )}
            {onFetchCustomModels &&
              (isCustom || activeProvider.id === "opencode" || activeProvider.id === "ovh") && (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={fetchingSubpageModels}
                  onClick={async () => {
                    setFetchingSubpageModels(true);
                    try {
                      await onFetchCustomModels(activeProvider.id);
                    } finally {
                      setFetchingSubpageModels(false);
                    }
                  }}
                  title={`Fetch latest live model catalog for ${activeProvider.name}`}
                >
                  {fetchingSubpageModels ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Sparkles className="h-3.5 w-3.5" />
                  )}
                  Fetch /models
                </Button>
              )}
            {!isCustom && (!isAntigravity || accounts.length > 0) && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => onOpenQuotaTracker(activeProvider.id)}
              >
                <Gauge className="h-3.5 w-3.5" />
                Quota tracker
              </Button>
            )}
            <Button
              variant="secondary"
              size="sm"
              disabled={testingProvider}
              onClick={() => void handleRunTest()}
            >
              <Zap className="h-3.5 w-3.5" />
              {testingProvider ? "Testing…" : "Test"}
            </Button>
            <Button
              variant={activeProvider.config.enabled ? "secondary" : "primary"}
              size="sm"
              onClick={() =>
                void onUpdateProviderConfig(activeProvider.id, {
                  enabled: !activeProvider.config.enabled,
                })
              }
            >
              {activeProvider.config.enabled ? "Disable" : "Enable"}
            </Button>
          </div>
        </div>

        {currentTestResult && (
          <div
            className={`border-t border-[var(--border-subtle)] px-5 py-2 text-xs ${
              currentTestResult.ok
                ? "bg-[var(--success-soft)]/50 text-[var(--success)]"
                : "bg-[var(--danger-soft)]/50 text-[var(--danger)]"
            }`}
          >
            {currentTestResult.message}
          </div>
        )}

        {/* Quiet Inline Strategy Summary + Probe Controls */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 px-5 py-2.5">
          <div className="text-xs text-[var(--text-muted)] tabular-nums">
            {isCustom && activeProvider.custom_provider ? (
              <span>
                <span className="font-medium text-[var(--text-secondary)]">
                  {routingStrategyLabel(currentRoutingStrategy)}
                </span>{" "}
                &middot; {activeProvider.custom_provider.protocol.toUpperCase()}{" "}
                ({activeProvider.custom_provider.api_type}) &middot;{" "}
                <code className="font-mono">{activeProvider.custom_provider.base_url}</code>
              </span>
            ) : isAntigravity ? (
              <span>
                <span className="font-medium text-[var(--text-secondary)]">
                  {routingStrategyLabel(currentRoutingStrategy)}
                </span>{" "}
                &middot; {smartSplitPreview.sliceText}
              </span>
            ) : (
              <span>
                <span className="font-medium text-[var(--text-secondary)]">
                  {routingStrategyLabel(currentRoutingStrategy)}
                </span>{" "}
                &middot; {activeProvider.proxy_test_interval_sec}s TesieCore health &middot;{" "}
                <span className="font-mono text-[var(--success)]">{activeQuotas}</span> ready
                {coolingQuotas > 0 && (
                  <>
                    {" / "}
                    <span className="font-mono text-[var(--warning)]">{coolingQuotas}</span> queued
                  </>
                )}
              </span>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-3">
            {!isAntigravity && (
              <div
                className="flex items-center gap-1.5 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-2.5 py-1 text-xs"
                title="Default reasoning effort sent to upstream when client does not specify it"
              >
                <Brain className="h-3.5 w-3.5 text-[var(--warning)]" />
                <span className="font-medium text-[var(--text-muted)]">Reasoning:</span>
                <select
                  aria-label="Provider default reasoning effort"
                  value={currentThinkingLevel}
                  onChange={(e) =>
                    void onUpdateProviderConfig(activeProvider.id, {
                      thinking_level: e.target.value,
                    })
                  }
                  className="bg-transparent font-medium text-[var(--text-primary)] outline-none cursor-pointer"
                >
                  <option value="auto">Auto (client default)</option>
                  <option value="none">None (disable thinking)</option>
                  <option value="minimal">Minimal</option>
                  <option value="low">Low</option>
                  <option value="medium">Medium</option>
                  <option value="high">High</option>
                  <option value="xhigh">X-High</option>
                  <option value="max">Max</option>
                </select>
              </div>
            )}

            {!isCustom && (
              <ProbeSettingsBar
                compact
                providerId={activeProvider.id}
                providerName={activeProvider.name}
                probeEnabled={activeProvider.config.probe_enabled ?? true}
                probeIntervalSec={activeProvider.config.probe_interval_sec ?? 30}
                onUpdateConfig={(patch) => onUpdateProviderConfig(activeProvider.id, patch)}
                onManualProbe={
                  isAntigravity
                    ? accounts.length > 0
                      ? () => void handleRefreshAllAg()
                      : undefined
                    : () =>
                        void (onProbeQuotasNow ? onProbeQuotasNow() : handleManualQuotaRefresh())
                }
                probing={isAntigravity ? refreshingAllAg : syncingQuotas || isProbingLive}
                lastProbeAt={
                  isAntigravity
                    ? accounts[0]?.quota?.fetched_at ?? null
                    : quotaProbeStatus?.last_probe_at ?? lastQuotaSyncAt
                }
                nowMs={nowMs}
              />
            )}
          </div>
        </div>
      </section>

      {/* Antigravity Connected Accounts & Smart Proxy Distribution */}
      {isAntigravity && (
        <section className="space-y-3.5">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-5 py-3.5">
            <div className="min-w-0">
              <h3 className="text-[15px] font-semibold tracking-tight text-[var(--text-primary)]">
                Antigravity Accounts ({accounts.length})
              </h3>
              <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                {smartSplitPreview.sliceText}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              {accounts.length > 0 && (
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={refreshingAllAg}
                  onClick={() => void handleRefreshAllAg()}
                >
                  <RefreshCw
                    className={`h-3.5 w-3.5 ${refreshingAllAg ? "animate-spin" : ""}`}
                  />
                  Refresh quotas
                </Button>
              )}
              <Button variant="primary" size="sm" onClick={openConnectModal}>
                <Plus className="h-4 w-4" />
                Connect Account
              </Button>
            </div>
          </div>

          {accounts.length === 0 ? (
            <div className="rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-6">
              <EmptyState
                icon={<UserCheck className="h-6 w-6" />}
                title="No Antigravity accounts connected yet"
                description="Sign in with Google OAuth (or import a refresh token) to unlock Cloud Code Gemini, Claude 4.6, and GPT-OSS models with automatic per-account proxy isolation."
                action={
                  <Button variant="primary" size="md" onClick={openConnectModal}>
                    <Plus className="h-4 w-4" />
                    Connect First Google Account
                  </Button>
                }
              />
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-3.5 xl:grid-cols-2">
              {accounts.map((acct, idx) => {
                const isBanned =
                  acct.status === "banned" ||
                  (!!acct.last_error &&
                    /violation of terms|disabled in this account|account_disabled_tos/i.test(
                      acct.last_error,
                    ));
                const cooling =
                  acct.enabled &&
                  !isBanned &&
                  acct.status === "cooldown" &&
                  !!acct.cooldown_until &&
                  acct.cooldown_until > nowMs;
                const part = acct.partition;
                const assigned = part?.assigned_proxies ?? [];
                const primaryPx = part?.primary_proxy ?? null;
                const firstNode = assigned[0]?.name;
                const lastNode = assigned[assigned.length - 1]?.name;
                const availableModels = (acct.quota?.models ?? []).filter(
                  (m) => m.unlocked !== false,
                );
                const cooldownMap = new Map(
                  (acct.model_cooldowns ?? []).map((mc) => [mc.model, mc]),
                );
                const mainPools = (acct.quota?.pools ?? []).filter(
                  (p) => p.unlocked !== false,
                );
                const gemWeekly = mainPools.find(
                  (p) =>
                    p.group_name.toLowerCase().includes("gemini") &&
                    (p.key.includes("weekly") || p.window_label.toLowerCase().includes("week")),
                );
                const claudeWeekly = mainPools.find(
                  (p) =>
                    (p.group_name.toLowerCase().includes("claude") ||
                      p.group_name.toLowerCase().includes("gpt")) &&
                    (p.key.includes("weekly") || p.window_label.toLowerCase().includes("week")),
                );

                const isExpanded = expandedAccountIds.has(acct.id);

                // Sort models: priority to cooling/failed models, then flagship models, then lite
                const sortedModels = [...availableModels].sort((a, b) => {
                  const aCool = (cooldownMap.get(a.model)?.cooldown_until ?? 0) > nowMs ? 1 : 0;
                  const bCool = (cooldownMap.get(b.model)?.cooldown_until ?? 0) > nowMs ? 1 : 0;
                  if (aCool !== bCool) return bCool - aCool;
                  const aFlag = /gemini-3.8|sonnet|opus|gpt-oss/i.test(a.model) ? 1 : 0;
                  const bFlag = /gemini-3.8|sonnet|opus|gpt-oss/i.test(b.model) ? 1 : 0;
                  if (aFlag !== bFlag) return bFlag - aFlag;
                  return 0;
                });

                const visibleModels = isExpanded
                  ? sortedModels
                  : sortedModels.filter((m, i) => i < 4 || (cooldownMap.get(m.model)?.cooldown_until ?? 0) > nowMs);
                const hasHiddenModels = !isExpanded && sortedModels.length > visibleModels.length;

                const cardSurface = "border-[var(--border-subtle)] bg-[var(--bg-surface)] hover:border-[var(--border-strong)]";

                return (
                  <article
                    key={acct.id}
                    className={`group relative overflow-hidden flex flex-col justify-between gap-3.5 rounded-2xl border p-4 sm:p-5 transition-colors duration-150 ${cardSurface} ${
                      !acct.enabled ? "opacity-75" : ""
                    }`}
                  >
                    {/* Status Corner Ribbon (Unified across all account states) */}
                    {isBanned ? (
                      <div className="absolute top-0 right-0 z-10 flex items-center gap-1.5 rounded-bl-xl border-b border-l border-rose-500/35 bg-rose-500/15 px-3 py-1 font-mono text-[11px] font-semibold text-rose-400 backdrop-blur-md shadow-sm">
                        <span className="h-1.5 w-1.5 rounded-full bg-rose-400" />
                        <span>Banned · Auto-disabled</span>
                      </div>
                    ) : cooling ? (
                      <div className="absolute top-0 right-0 z-10 flex items-center gap-1.5 rounded-bl-xl border-b border-l border-amber-500/35 bg-amber-500/15 px-3 py-1 font-mono text-[11px] font-semibold text-amber-400 backdrop-blur-md shadow-sm">
                        <span className="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse" />
                        <span>Cooldown ({formatCountdown(acct.cooldown_until, nowMs)})</span>
                      </div>
                    ) : !acct.enabled ? (
                      acct.last_error && /license|gemini code assist/i.test(acct.last_error) ? (
                        <div className="absolute top-0 right-0 z-10 flex items-center gap-1.5 rounded-bl-xl border-b border-l border-amber-500/35 bg-amber-500/15 px-3 py-1 font-mono text-[11px] font-semibold text-amber-400 backdrop-blur-md shadow-sm">
                          <span className="h-1.5 w-1.5 rounded-full bg-amber-400" />
                          <span>Disabled · License Inactive</span>
                        </div>
                      ) : (
                        <div className="absolute top-0 right-0 z-10 flex items-center gap-1.5 rounded-bl-xl border-b border-l border-zinc-500/35 bg-zinc-500/15 px-3 py-1 font-mono text-[11px] font-semibold text-zinc-400 backdrop-blur-md shadow-sm">
                          <span className="h-1.5 w-1.5 rounded-full bg-zinc-400" />
                          <span>Disabled · Inactive</span>
                        </div>
                      )
                    ) : (
                      <div className="absolute top-0 right-0 z-10 flex items-center gap-1.5 rounded-bl-xl border-b border-l border-emerald-500/35 bg-emerald-500/15 px-3 py-1 font-mono text-[11px] font-semibold text-emerald-400 backdrop-blur-md shadow-sm">
                        <span className="h-1.5 w-1.5 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,0.6)]" />
                        <span>Active · Serving Tokens</span>
                      </div>
                    )}

                    {/* Header: Identity & Unified Action Toolbar */}
                    <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-3 sm:pr-48">
                      {/* Identity Column */}
                      <div className="flex flex-col gap-1.5 min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="rounded-md bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-xs font-semibold text-[var(--text-muted)]">
                            #{idx + 1}
                          </span>
                          <span className="truncate text-[15px] font-semibold text-[var(--text-primary)]" title={acct.email || acct.name}>
                            {acct.email || acct.name}
                          </span>
                          {acct.quota?.plan && (
                            <span className="rounded-md border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-[11px] font-semibold uppercase text-[var(--accent)]">
                              {acct.quota.plan}
                            </span>
                          )}
                        </div>

                        {/* Routing & Sync Metadata */}
                        <div className="flex flex-wrap items-center gap-2 font-mono text-xs text-[var(--text-muted)]">
                          <span className="flex items-center gap-1">
                            <Globe className="h-3.5 w-3.5 text-[var(--text-muted)]" />
                            <span className="text-[var(--text-secondary)] font-medium">{primaryPx ? primaryPx.name : "Direct"}</span>
                            {assigned.length > 1 && !assigned[0]?.is_direct
                              ? ` (${firstNode} → ${lastNode}, ${assigned.length})`
                              : ""}
                          </span>
                          <span>·</span>
                          <span>
                            Synced {acct.quota?.fetched_at ? formatTimeAgo(acct.quota.fetched_at, nowMs) : "pending"}
                          </span>
                        </div>
                      </div>

                      {/* Unified Action Toolbar */}
                      <div className="flex shrink-0 items-center gap-1.5 self-start sm:self-center">
                        {cooling && (
                          <button
                            type="button"
                            onClick={() => void onUpdateAccountConnection(acct.id, { reset_cooldown: true })}
                            className="inline-flex items-center gap-1.5 rounded-lg border border-amber-500/30 bg-amber-500/10 px-2.5 py-1 text-xs font-medium text-amber-400 hover:bg-amber-500/20 transition-colors"
                            title="Clear account cooldown"
                          >
                            <RotateCcw className="h-3.5 w-3.5" />
                            <span>Reset</span>
                          </button>
                        )}

                        <button
                          type="button"
                          disabled={refreshingAccountId === acct.id}
                          onClick={() => void handleRefreshOneAg(acct.id)}
                          className="inline-flex items-center justify-center rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-1.5 text-[var(--text-secondary)] hover:border-[var(--border-strong)] hover:text-[var(--text-primary)] transition-colors disabled:opacity-45"
                          title="Sync quotas for this account"
                          aria-label="Sync quotas"
                        >
                          <RefreshCw className={`h-3.5 w-3.5 ${refreshingAccountId === acct.id ? "animate-spin text-[var(--accent)]" : ""}`} />
                        </button>

                        <button
                          type="button"
                          onClick={() => void onUpdateAccountConnection(acct.id, { enabled: !acct.enabled })}
                          className={`inline-flex items-center gap-1 rounded-lg border px-2.5 py-1 text-xs font-medium transition-colors ${
                            acct.enabled
                              ? "border-[var(--border-subtle)] bg-[var(--bg-elevated)] text-[var(--text-secondary)] hover:text-[var(--text-primary)] hover:border-[var(--border-strong)]"
                              : "border-blue-500/30 bg-blue-500/10 text-blue-400 hover:bg-blue-500/20"
                          }`}
                        >
                          {acct.enabled ? "Disable" : "Enable"}
                        </button>

                        <button
                          type="button"
                          onClick={() => void onDeleteAccountConnection(acct.id, acct.email || acct.name)}
                          className="inline-flex items-center justify-center rounded-lg border border-transparent p-1.5 text-[var(--text-muted)] hover:border-rose-500/30 hover:bg-rose-500/10 hover:text-rose-400 transition-colors"
                          title="Remove account"
                          aria-label="Remove account"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </div>

                    {/* Quota Weekly Pools (Compact, high-contrast visual gauges) */}
                    {(gemWeekly || claudeWeekly) && (
                      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/30 p-2.5">
                        {gemWeekly && (() => {
                          const rawGem = Math.round(gemWeekly.remaining_percentage);
                          const gemPct = isBanned ? 0 : !acct.enabled ? 0 : rawGem;
                          return (
                            <div className="flex items-center justify-between gap-3 px-1">
                              <div className="flex items-center gap-2 min-w-0">
                                <span className={`h-2 w-2 rounded-full shrink-0 ${isBanned ? "bg-rose-400" : !acct.enabled ? "bg-zinc-500" : "bg-blue-400"}`} />
                                <span className="text-xs font-medium text-[var(--text-secondary)] truncate">Gemini Weekly Pool</span>
                              </div>
                              <div className="flex items-center gap-2 shrink-0">
                                <div className="w-16 h-1.5 bg-[var(--bg-base)] rounded-full overflow-hidden">
                                  <div
                                    className={`h-full rounded-full transition-all duration-300 ${
                                      isBanned
                                        ? "bg-rose-500/40"
                                        : !acct.enabled
                                          ? "bg-zinc-600"
                                          : gemPct <= 20
                                            ? "bg-rose-500"
                                            : gemPct <= 50
                                              ? "bg-amber-400"
                                              : "bg-blue-400"
                                    }`}
                                    style={{ width: `${gemPct}%` }}
                                  />
                                </div>
                                <span className="font-mono text-xs font-semibold text-[var(--text-primary)] tabular-nums">
                                  {isBanned ? (
                                    <span className="text-rose-400">0%</span>
                                  ) : !acct.enabled ? (
                                    <span className="text-[var(--text-muted)]">0%</span>
                                  ) : (
                                    `${gemPct}%`
                                  )}
                                </span>
                              </div>
                            </div>
                          );
                        })()}
                        {claudeWeekly && (() => {
                          const rawClaude = Math.round(claudeWeekly.remaining_percentage);
                          const claudePct = isBanned ? 0 : !acct.enabled ? 0 : rawClaude;
                          return (
                            <div className="flex items-center justify-between gap-3 px-1">
                              <div className="flex items-center gap-2 min-w-0">
                                <span className={`h-2 w-2 rounded-full shrink-0 ${isBanned ? "bg-rose-400" : !acct.enabled ? "bg-zinc-500" : "bg-purple-400"}`} />
                                <span className="text-xs font-medium text-[var(--text-secondary)] truncate">Claude / GPT Pool</span>
                              </div>
                              <div className="flex items-center gap-2 shrink-0">
                                <div className="w-16 h-1.5 bg-[var(--bg-base)] rounded-full overflow-hidden">
                                  <div
                                    className={`h-full rounded-full transition-all duration-300 ${
                                      isBanned
                                        ? "bg-rose-500/40"
                                        : !acct.enabled
                                          ? "bg-zinc-600"
                                          : claudePct <= 20
                                            ? "bg-rose-500"
                                            : claudePct <= 50
                                              ? "bg-amber-400"
                                              : "bg-purple-400"
                                    }`}
                                    style={{ width: `${claudePct}%` }}
                                  />
                                </div>
                                <span className="font-mono text-xs font-semibold text-[var(--text-primary)] tabular-nums">
                                  {isBanned ? (
                                    <span className="text-rose-400">0%</span>
                                  ) : !acct.enabled ? (
                                    <span className="text-[var(--text-muted)]">0%</span>
                                  ) : (
                                    `${claudePct}%`
                                  )}
                                </span>
                              </div>
                            </div>
                          );
                        })()}
                      </div>
                    )}

                    {/* Models Section: Organized & Noise-free */}
                    {availableModels.length > 0 ? (
                      <div className="space-y-2">
                        <div className="flex items-center justify-between text-xs text-[var(--text-muted)]">
                          <span className="font-medium">Model Availability ({availableModels.length})</span>
                          {sortedModels.length > 4 && (
                            <button
                              type="button"
                              onClick={() => toggleAccountExpanded(acct.id)}
                              className="inline-flex items-center gap-1 font-medium text-[var(--accent)] hover:underline"
                            >
                              <span>{isExpanded ? "Show less" : `+${hasHiddenModels ? sortedModels.length - visibleModels.length : 0} more`}</span>
                              {isExpanded ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                            </button>
                          )}
                        </div>

                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-1.5">
                          {visibleModels.map((mq) => {
                            const rawPct = Math.max(0, Math.min(100, Math.round(mq.remaining_percentage)));
                            const isInactive = isBanned || !acct.enabled;
                            const pct = isInactive ? 0 : rawPct;
                            const mc = cooldownMap.get(mq.model);
                            const coolUntil = mq.failure_cooldown_until ?? mc?.cooldown_until ?? null;
                            const failCount = mq.failure_count ?? mc?.consecutive_failures ?? 0;
                            const modelCooling = !isInactive && !!coolUntil && coolUntil > nowMs;
                            const chipCaps = getModelCapabilities(mq.model, modelsEntryMap.get(mq.model.toLowerCase()));

                            let chipClass = "border-[var(--border-subtle)] bg-[var(--bg-elevated)]/50";
                            if (isBanned) {
                              chipClass = "border-rose-500/25 bg-rose-500/5 opacity-65";
                            } else if (!acct.enabled) {
                              chipClass = "border-[var(--border-subtle)] bg-[var(--bg-elevated)]/25 opacity-55";
                            } else if (modelCooling) {
                              chipClass = "border-amber-500/30 bg-amber-500/10";
                            } else {
                              chipClass = getModelChipClasses(chipCaps);
                            }

                            const pctColor = isBanned
                              ? "text-rose-400/80 font-medium line-through"
                              : !acct.enabled
                                ? "text-[var(--text-muted)] line-through"
                                : pct <= 0
                                  ? "text-rose-400 font-semibold"
                                  : modelCooling
                                    ? "text-amber-400 font-semibold"
                                    : pct <= 35
                                      ? "text-amber-400 font-semibold"
                                      : "text-emerald-400 font-medium";

                            return (
                              <div
                                key={mq.model}
                                className={`flex items-center justify-between gap-2 rounded-lg border px-2.5 py-1.5 text-xs transition-colors ${chipClass}`}
                              >
                                <div className="flex items-center gap-1.5 min-w-0">
                                  <span
                                    className={`truncate font-medium ${isInactive ? "text-[var(--text-muted)] line-through" : "text-[var(--text-primary)]"}`}
                                    title={mq.display_name || mq.model}
                                  >
                                    {mq.display_name || mq.model}
                                  </span>
                                  {!isInactive && <ModelCapabilityBadges caps={chipCaps} compact />}
                                </div>
                                <div className="flex items-center gap-1.5 shrink-0 font-mono text-[11px] tabular-nums">
                                  {modelCooling && (
                                    <span className="text-amber-400">
                                      ({failCount}f · {formatCountdown(coolUntil, nowMs)})
                                    </span>
                                  )}
                                  <span className={pctColor}>
                                    {isBanned ? "banned" : !acct.enabled ? "disabled" : `${pct}%`}
                                  </span>
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    ) : (
                      <div className="text-xs text-[var(--text-muted)] italic">
                        Click Refresh to discover available Cloud Code models for this account.
                      </div>
                    )}

                    {/* Notices & Banned Alerts */}
                    {(part?.shared_warning || acct.last_error) && (
                      <div className="space-y-1.5 pt-1">
                        {part?.shared_warning && (
                          <div className="flex items-center gap-2 rounded-lg border border-amber-500/25 bg-amber-500/10 px-3 py-1.5 text-xs text-amber-400">
                            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                            <span>Shared proxy IP — add more proxies for strict 1:1 isolation.</span>
                          </div>
                        )}
                        {acct.last_error && (
                          <div
                            className={`flex items-start gap-2 rounded-lg border px-3 py-1.5 text-xs ${
                              isBanned
                                ? "border-rose-500/25 bg-rose-500/10 text-rose-400"
                                : "border-amber-500/25 bg-amber-500/10 text-amber-400"
                            }`}
                          >
                            <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
                            <div className="min-w-0 flex-1">
                              <span className="font-semibold">{isBanned ? "Account auto-disabled:" : "Upstream notice:"}</span>{" "}
                              <span>
                                {isBanned
                                  ? formatCooldownReason(acct.last_error).replace(/^account\s+auto-disabled:\s*/i, "")
                                  : formatCooldownReason(acct.last_error)}
                              </span>
                            </div>
                          </div>
                        )}
                      </div>
                    )}
                  </article>
                );
              })}
            </div>
          )}
        </section>
      )}

      {/* Provider Routing Strategy & Proxy Settings */}
      <Card
        title={`${activeProvider.name} Routing & Proxy Settings`}
        subtitle={`${routingStrategyLabel(currentRoutingStrategy)} · ${proxyModeLabel(
          activeProvider.config.proxy_mode,
          activeProvider.config.selected_proxy_ids?.length,
        )}`}
        actions={
          <button
            type="button"
            onClick={onOpenProxyPool}
            className="inline-flex items-center gap-1 text-xs font-medium text-[var(--accent)] hover:underline"
          >
            Manage Proxy Pool
            <ArrowRight className="h-3.5 w-3.5" />
          </button>
        }
      >
        <form onSubmit={handleSaveConfig} className="space-y-4">
          <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/45 p-3.5">
            <div className="min-w-0">
              <div className="text-xs font-semibold text-[var(--text-primary)]">
                {isAntigravity
                  ? "Account Routing Strategy"
                  : isCustom
                    ? "Key & Proxy Routing Strategy"
                    : "Exit-IP Routing Strategy"}
              </div>
              <div className="mt-0.5 text-xs text-[var(--text-muted)]">
                {isAntigravity
                  ? "Controls how Tesie chooses among your enabled Google accounts (each keeps its disjoint proxy slice)."
                  : isCustom
                    ? "Controls how Tesie selects API keys and healthy proxies across requests."
                    : "Controls how Tesie selects among healthy OpenCode exit IPs across requests."}
              </div>
            </div>
            <SegmentedControl<RoutingStrategy>
              size="sm"
              value={currentRoutingStrategy}
              onChange={(next) =>
                void onUpdateProviderConfig(activeProvider.id, { routing_strategy: next })
              }
              options={[
                { value: "fallback", label: "Fallback" },
                { value: "round-robin", label: "Round-Robin" },
                { value: "random", label: "Random" },
              ]}
            />
          </div>

          <ProviderProxySelector
            proxies={proxies}
            value={proxySelection}
            onChange={setProxySelection}
            strategyHint={
              isAntigravity
                ? `Disjoint proxy slices across accounts · ${routingStrategyLabel(currentRoutingStrategy)}`
                : `${routingStrategyLabel(currentRoutingStrategy)} across healthy selected nodes`
            }
            onOpenProxyPool={onOpenProxyPool}
          />

          {!isAntigravity && !isCustom && (
            <details className="group rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/40 px-3.5 py-2.5">
              <summary className="flex cursor-pointer list-none items-center justify-between gap-2 text-xs font-medium text-[var(--text-secondary)] select-none hover:text-[var(--text-primary)]">
                <span className="inline-flex items-center gap-1.5">
                  <ChevronRight className="h-3.5 w-3.5 transition-transform group-open:rotate-90" />
                  Advanced per-IP window &amp; soft caps
                </span>
                <span className="font-mono text-[11px] text-[var(--text-muted)] tabular-nums">
                  {winMinutes}m window &middot; {sessionTtlHours}h TTL &middot;{" "}
                  {maxReqs.trim() || "Unlimited"} req &middot; {maxToks.trim() || "Unlimited"} tok
                </span>
              </summary>

              <div className="mt-3 grid grid-cols-1 gap-3 border-t border-[var(--border-subtle)] pt-3 sm:grid-cols-2 lg:grid-cols-4">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Per-IP window (min)
                  </label>
                  <input
                    type="number"
                    min={1}
                    value={winMinutes}
                    onChange={(e) => setWinMinutes(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Sticky session TTL (hrs)
                  </label>
                  <input
                    type="number"
                    min={1}
                    value={sessionTtlHours}
                    onChange={(e) => setSessionTtlHours(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Soft req cap / IP
                  </label>
                  <input
                    type="number"
                    min={0}
                    placeholder="Unlimited"
                    value={maxReqs}
                    onChange={(e) => setMaxReqs(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Soft token cap / IP
                  </label>
                  <input
                    type="number"
                    min={0}
                    placeholder="Unlimited"
                    value={maxToks}
                    onChange={(e) => setMaxToks(e.target.value)}
                    className={`${INPUT_CLASS} font-mono`}
                  />
                </div>
              </div>
            </details>
          )}

          <div className="flex flex-wrap items-center justify-end gap-3 pt-1">
            <Button type="submit" variant="primary" size="md" disabled={savingCfg}>
              <Save className="h-3.5 w-3.5" />
              {savingCfg ? "Saving…" : "Save proxy settings"}
            </Button>
          </div>
        </form>
      </Card>

      {/* Provider Model Catalog — Separate Individual Model Cards (4/line Table | 2/line List) */}
      <Card
        noPadding
        title={`Available Models (${filteredModels.length})`}
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative w-52 sm:w-64">
              <Search className="pointer-events-none absolute top-2.5 left-3 h-4 w-4 text-[var(--text-muted)]" />
              <input
                type="text"
                placeholder="Filter model IDs…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className={`${INPUT_CLASS} py-1.5 pl-9 text-[13px]`}
              />
            </div>

            <div
              className="inline-flex rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-0.5"
              role="group"
              aria-label="Models layout view"
            >
              <button
                type="button"
                onClick={() => handleChangeModelsView("table")}
                title="Table view (4 items per line)"
                className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                  modelsViewMode === "table"
                    ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                }`}
              >
                <LayoutGrid className="h-4 w-4" />
                Table
              </button>
              <button
                type="button"
                onClick={() => handleChangeModelsView("list")}
                title="List view (2 items per line)"
                className={`inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[13px] font-medium transition-colors ${
                  modelsViewMode === "list"
                    ? "bg-[var(--bg-surface)] text-[var(--text-primary)] shadow-xs"
                    : "text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                }`}
              >
                <List className="h-4 w-4" />
                List
              </button>
            </div>
          </div>
        }
      >
        <div className="bg-[var(--bg-elevated)]/25 p-4 space-y-3">
          <div
            className={`grid gap-2.5 ${
              modelsViewMode === "list"
                ? "grid-cols-1 md:grid-cols-2"
                : "grid-cols-1 sm:grid-cols-2 lg:grid-cols-4"
            }`}
          >
            {filteredModels.map((fullSpec) => {
              const slashIdx = fullSpec.indexOf("/");
              const prefix = slashIdx >= 0 ? fullSpec.slice(0, slashIdx + 1) : "";
              const modelSlug = slashIdx >= 0 ? fullSpec.slice(slashIdx + 1) : fullSpec;
              const agState = isAntigravity
                ? agModelStateMap.get(modelSlug.toLowerCase())
                : undefined;
              const isLockedForUser = Boolean(isAntigravity && agState && !agState.unlocked);
              const modelEntry =
                modelsEntryMap.get(fullSpec.toLowerCase()) ||
                modelsEntryMap.get(modelSlug.toLowerCase());
              const caps = getModelCapabilities(fullSpec, modelEntry);
              const limits = getModelLimits(fullSpec, modelEntry);
              const cardAccents = getModelCardClasses(caps);

              return (
                <div
                  key={fullSpec}
                  className={`group relative overflow-hidden flex flex-col justify-between gap-3 rounded-xl px-3.5 py-3 transition-[border-color,background-color,box-shadow] duration-200 ${cardAccents} ${
                    isLockedForUser
                      ? "opacity-45 grayscale"
                      : ""
                  }`}
                  title={
                    isLockedForUser
                      ? `${fullSpec} is not unlocked on your connected Antigravity account(s)`
                      : limits?.tooltip
                      ? `${fullSpec} (${limits.tooltip})`
                      : fullSpec
                  }
                >
                  <div className="min-w-0 space-y-2">
                    {/* Header Row: Model spec on left, capability icons in top-right corner */}
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0 flex-1 truncate font-mono text-[13px]" title={fullSpec}>
                        {prefix && <span className="text-[var(--text-muted)]">{prefix}</span>}
                        <span className="font-semibold text-[var(--text-primary)]">{modelSlug}</span>
                      </div>
                      <ModelCapabilityIcons caps={caps} />
                    </div>

                    {/* Second Row: Token Limit Badge & Quota / Lock Status */}
                    <div className="flex flex-wrap items-center justify-between gap-1.5 pt-0.5">
                      <ModelLimitBadge limits={limits} />
                      {(isLockedForUser || agState) && (
                        <div className="text-[11px] shrink-0">
                          {isLockedForUser ? (
                            <span className="inline-flex items-center gap-1 text-[var(--text-muted)]">
                              <Lock className="h-3 w-3" />
                              Not unlocked
                            </span>
                          ) : agState ? (
                            <span className="font-mono text-[var(--success)] font-medium tabular-nums">
                              {Math.round(agState.maxRemaining)}% left
                            </span>
                          ) : null}
                        </div>
                      )}
                    </div>
                  </div>

                  <div className="flex shrink-0 items-center justify-end gap-1 border-t border-[var(--border-subtle)]/40 pt-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                    <button
                      type="button"
                      onClick={() => copyText(fullSpec, fullSpec)}
                      className="rounded-lg p-1.5 text-[var(--text-muted)] hover:bg-[var(--bg-elevated)] hover:text-[var(--text-primary)]"
                      title="Copy model ID"
                      aria-label={`Copy ${fullSpec}`}
                    >
                      {copiedId === fullSpec ? (
                        <Check className="h-3.5 w-3.5 text-[var(--success)]" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                    </button>
                    <button
                      type="button"
                      disabled={isLockedForUser}
                      onClick={() => onTestModel(fullSpec)}
                      className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-elevated)] hover:text-[var(--accent)] disabled:opacity-45"
                      title={`Test ${fullSpec} in Playground`}
                    >
                      <Play className="h-3.5 w-3.5" />
                      Test
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </Card>

      {/* Connect Antigravity Google Account Modal */}
      <Modal
        open={connectOpen}
        title="Connect Antigravity Google Account"
        maxWidth="max-w-xl"
        onClose={() => {
          setConnectOpen(false);
          setOauthPolling(false);
        }}
      >
        <div className="space-y-4">
          <SegmentedControl<ConnectTab>
            value={connectTab}
            onChange={(next) => {
              setConnectTab(next);
              setOauthError(null);
            }}
            options={[
              {
                value: "browser",
                label: "Browser Sign-In",
                icon: <ExternalLink className="h-3.5 w-3.5" />,
              },
              {
                value: "callback",
                label: "Paste Callback URL",
                icon: <Link2 className="h-3.5 w-3.5" />,
              },
              {
                value: "token",
                label: "Import Refresh Token",
                icon: <KeyRound className="h-3.5 w-3.5" />,
              },
            ]}
          />

          {oauthError && (
            <div className="rounded-xl border border-[color-mix(in_srgb,var(--danger)_35%,transparent)] bg-[var(--danger-soft)] px-3.5 py-2.5 text-xs text-[var(--danger)]">
              {oauthError}
            </div>
          )}

          {connectTab === "browser" && (
            <div className="space-y-4">
              <div className="rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-3.5 text-xs text-[var(--text-secondary)] space-y-2">
                <p>
                  Sign in with Google OAuth to authorize Cloud Code / Antigravity. Tesie opens a
                  temporary local callback listener and automatically assigns a dedicated proxy
                  slice to the new account.
                </p>
                <div className="flex flex-wrap items-center gap-2 pt-1">
                  <Button
                    variant="primary"
                    size="md"
                    onClick={() => void handleStartBrowserOAuth()}
                  >
                    <ExternalLink className="h-3.5 w-3.5" />
                    {oauthSession ? "Re-open Google Sign-In" : "Sign in with Google"}
                  </Button>
                  {oauthSession && (
                    <Button
                      variant="secondary"
                      size="md"
                      onClick={() => copyText("oauth-url", oauthSession.auth_url)}
                    >
                      {copiedId === "oauth-url" ? (
                        <CheckCircle2 className="h-3.5 w-3.5 text-[var(--success)]" />
                      ) : (
                        <Copy className="h-3.5 w-3.5" />
                      )}
                      Copy Auth URL
                    </Button>
                  )}
                </div>
              </div>

              {oauthSession && (
                <div className="space-y-3 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5">
                  <div className="flex items-center gap-2 text-xs text-[var(--accent)]">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    <span className="font-medium">
                      Waiting for Google OAuth callback on {oauthSession.redirect_uri}…
                    </span>
                  </div>

                  <form onSubmit={handleCompleteManualCallback} className="space-y-2 pt-1">
                    <label className="block text-[11px] text-[var(--text-muted)]">
                      Signed in on another browser/machine? Paste the redirected{" "}
                      <code className="font-mono">http://localhost:.../callback?code=...</code> URL
                      here:
                    </label>
                    <div className="flex gap-2">
                      <input
                        type="text"
                        placeholder="http://localhost:.../callback?code=4/0A...&state=..."
                        value={callbackInput}
                        onChange={(e) => setCallbackInput(e.target.value)}
                        className={`${INPUT_CLASS} font-mono`}
                      />
                      <Button
                        type="submit"
                        variant="secondary"
                        size="md"
                        disabled={!callbackInput.trim() || completingCallback}
                      >
                        {completingCallback ? "Verifying…" : "Complete"}
                      </Button>
                    </div>
                  </form>
                </div>
              )}
            </div>
          )}

          {connectTab === "callback" && (
            <form onSubmit={handleCompleteManualCallback} className="space-y-3">
              <p className="text-xs text-[var(--text-muted)]">
                Paste the full Google OAuth callback URL (including{" "}
                <code className="font-mono">code=</code> and{" "}
                <code className="font-mono">state=</code>) to complete token exchange through
                Tesie&apos;s smart proxy pool.
              </p>
              <div>
                <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                  Callback URL or Authorization Code
                </label>
                <input
                  type="text"
                  required
                  placeholder="http://localhost:54321/callback?code=4/0A...&state=..."
                  value={callbackInput}
                  onChange={(e) => setCallbackInput(e.target.value)}
                  className={`${INPUT_CLASS} font-mono`}
                />
              </div>
              <div className="flex justify-end gap-2 pt-2">
                <Button variant="secondary" size="md" onClick={() => setConnectOpen(false)}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  size="md"
                  disabled={!callbackInput.trim() || completingCallback}
                >
                  {completingCallback ? "Completing…" : "Exchange & Connect"}
                </Button>
              </div>
            </form>
          )}

          {connectTab === "token" && (
            <form onSubmit={handleImportTokenSubmit} className="space-y-3">
              <p className="text-xs text-[var(--text-muted)]">
                Import an existing Antigravity / Cloud Code Google OAuth{" "}
                <code className="font-mono">refresh_token</code> (e.g. from{" "}
                <code className="font-mono">9router</code>). Tesie will refresh the token,
                discover the Cloud Code project, and assign its dedicated proxy slice.
              </p>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Account Label (optional)
                  </label>
                  <input
                    type="text"
                    placeholder="Account 1"
                    value={importName}
                    onChange={(e) => setImportName(e.target.value)}
                    className={INPUT_CLASS}
                  />
                </div>
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    Email hint (optional)
                  </label>
                  <input
                    type="email"
                    placeholder="user@gmail.com"
                    value={importEmail}
                    onChange={(e) => setImportEmail(e.target.value)}
                    className={INPUT_CLASS}
                  />
                </div>
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                  Google OAuth Refresh Token
                </label>
                <input
                  type="password"
                  required
                  placeholder="1//0g..."
                  value={importRefreshToken}
                  onChange={(e) => setImportRefreshToken(e.target.value)}
                  className={`${INPUT_CLASS} font-mono`}
                />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                  Cloud Code Project ID (optional — auto-discovered if blank)
                </label>
                <input
                  type="text"
                  placeholder="Auto-discover via loadCodeAssist"
                  value={importProjectId}
                  onChange={(e) => setImportProjectId(e.target.value)}
                  className={`${INPUT_CLASS} font-mono`}
                />
              </div>
              <div className="flex justify-end gap-2 pt-2">
                <Button variant="secondary" size="md" onClick={() => setConnectOpen(false)}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  size="md"
                  disabled={!importRefreshToken.trim() || importingToken}
                >
                  {importingToken ? "Importing…" : "Import Account"}
                </Button>
              </div>
            </form>
          )}
        </div>
      </Modal>

      {customProviderModal}
    </div>
  );
}
