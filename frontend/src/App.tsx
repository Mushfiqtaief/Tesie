import { useCallback, useEffect, useState } from "react";
import {
  Activity,
  AlertCircle,
  Blocks,
  CheckCircle2,
  Gauge,
  HardDrive,
  KeyRound,
  Layers,
  LayoutDashboard,
  Menu,
  Moon,
  Network,
  Power,
  RefreshCw,
  Scissors,
  ScrollText,
  Settings,
  Sparkles,
  Sun,
  Terminal,
  Wrench,
  X,
} from "lucide-react";
import { ApiError, api, getStoredAdminToken, setStoredAdminToken } from "./api";
import { TesieLogo } from "./components/TesieLogo";
import { Button, INPUT_CLASS, Modal, StatusPill } from "./components/UiPrimitives";
import type {
  AntigravityAccountView,
  ApiKeyView,
  ApplyClaudeCliPayload,
  ApplyCodexCliPayload,
  ApplyOpenCodeCliPayload,
  CliToolsStatusMap,
  ComboRecord,
  CreateApiKeyPayload,
  CreateComboPayload,
  CreateCustomProviderPayload,
  CreatedApiKeyResponse,
  CreateProxyPayload,
  IpQuotaWithProxies,
  OAuthSessionPollStatus,
  OpenAiModelEntry,
  ProviderCatalogItem,
  ProviderQuotaSummary,
  ProviderTestResult,
  ProxyWithQuota,
  QuotaProbeStatus,
  StartedOAuthFlow,
  StatusResponse,
  TopologyResponse,
  UpdateApiKeyPayload,
  UpdateComboPayload,
  UpdateCustomProviderPayload,
  UpdateIpQuotaLimitsPayload,
  UpdateProviderConfigPayload,
  UpdateProxyPayload,
  UsagePeriod,
  UsageSummary,
} from "./types";
import { CliToolsView } from "./views/CliToolsView";
import { CombosView } from "./views/CombosView";
import { DashboardView } from "./views/DashboardView";
import { EndpointKeysView } from "./views/EndpointKeysView";
import { LogsView } from "./views/LogsView";
import { PlaygroundView } from "./views/PlaygroundView";
import { ProvidersModelsView } from "./views/ProvidersModelsView";
import { ProxyPoolView } from "./views/ProxyPoolView";
import { QuotaTrackerView } from "./views/QuotaTrackerView";
import { SettingsView } from "./views/SettingsView";
import { TokenSaverView } from "./views/TokenSaverView";
import { MultimodalBridgeView } from "./views/MultimodalBridgeView";

type RoutePath =
  | "/"
  | "/logs"
  | "/providers"
  | "/combos"
  | "/token-saver"
  | "/bridge"
  | "/quotas"
  | "/proxies"
  | "/cli-tools"
  | "/endpoint"
  | "/playground"
  | "/settings";

type ThemeMode = "light" | "dark";

interface NavItem {
  path: RoutePath;
  label: string;
  subtitle: string;
  icon: typeof LayoutDashboard;
}

interface NavGroup {
  label: string;
  items: NavItem[];
}

const NAV_GROUPS: NavGroup[] = [
  {
    label: "Analytics",
    items: [
      {
        path: "/",
        label: "Dashboard",
        subtitle: "Traffic, tokens, latency & live routing",
        icon: LayoutDashboard,
      },
      {
        path: "/logs",
        label: "Live Logs",
        subtitle: "Routed requests & failover inspector",
        icon: ScrollText,
      },
    ],
  },
  {
    label: "Routing & Quotas",
    items: [
      {
        path: "/providers",
        label: "Providers",
        subtitle: "Accounts, proxy selection & model catalogs",
        icon: Layers,
      },
      {
        path: "/combos",
        label: "Combos",
        subtitle: "Virtual fallback & round-robin model chains",
        icon: Blocks,
      },
      {
        path: "/token-saver",
        label: "Token Saver",
        subtitle: "RTK tool-output compression & terse response modes",
        icon: Scissors,
      },
      {
        path: "/bridge",
        label: "Multimodal Bridge",
        subtitle: "Vision & audio adapters for text-only models",
        icon: Sparkles,
      },
      {
        path: "/quotas",
        label: "Quota Tracker",
        subtitle: "Live per-IP & per-account capacity",
        icon: Gauge,
      },
      {
        path: "/proxies",
        label: "Proxy Pool",
        subtitle: "Global TesieCore nodes & health checks",
        icon: Network,
      },
    ],
  },
  {
    label: "Clients & Gateway",
    items: [
      {
        path: "/cli-tools",
        label: "CLI Tools",
        subtitle: "Auto-configure OpenCode, Claude & Codex",
        icon: Wrench,
      },
      {
        path: "/endpoint",
        label: "API Endpoint",
        subtitle: "Local endpoints, API keys & SDK snippets",
        icon: KeyRound,
      },
      {
        path: "/playground",
        label: "Playground",
        subtitle: "Live SSE chat & latency bench",
        icon: Terminal,
      },
      {
        path: "/settings",
        label: "Settings",
        subtitle: "Backup export/import, System Tray & gateway security",
        icon: Settings,
      },
    ],
  },
];

const THEME_STORAGE_KEY = "tesie_theme";
const LEGACY_THEME_STORAGE_KEY = "tesroute_theme";

function normalizeRoutePath(pathname: string): RoutePath {
  const clean = pathname.replace(/\/+$/, "") || "/";
  if (clean === "/keys") return "/endpoint";
  const valid: RoutePath[] = [
    "/",
    "/logs",
    "/providers",
    "/combos",
    "/token-saver",
    "/bridge",
    "/quotas",
    "/proxies",
    "/cli-tools",
    "/endpoint",
    "/playground",
    "/settings",
  ];
  return valid.includes(clean as RoutePath) ? (clean as RoutePath) : "/";
}

function readInitialTheme(): ThemeMode {
  try {
    const saved =
      localStorage.getItem(THEME_STORAGE_KEY) ||
      localStorage.getItem(LEGACY_THEME_STORAGE_KEY);
    if (saved === "light" || saved === "dark") {
      document.documentElement.classList.toggle("dark", saved === "dark");
      return saved;
    }
  } catch {
    // ignore
  }
  document.documentElement.classList.add("dark");
  return "dark";
}

export function App() {
  const [route, setRoute] = useState<RoutePath>(() =>
    normalizeRoutePath(window.location.pathname),
  );
  const [theme, setTheme] = useState<ThemeMode>(() => readInitialTheme());
  const [mobileNavOpen, setMobileNavOpen] = useState(false);

  const [period, setPeriod] = useState<UsagePeriod>("today");
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(() => {
    try {
      return new URLSearchParams(window.location.search).get("provider") || null;
    } catch {
      return null;
    }
  });
  const [selectedQuotaProviderId, setSelectedQuotaProviderId] = useState<string>("opencode");

  // Core gateway state
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [logsRefreshSignal, setLogsRefreshSignal] = useState(0);

  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [topology, setTopology] = useState<TopologyResponse | null>(null);
  const [providersCatalog, setProvidersCatalog] = useState<ProviderCatalogItem[]>([]);
  const [antigravityAccounts, setAntigravityAccounts] = useState<AntigravityAccountView[]>([]);
  const [combos, setCombos] = useState<ComboRecord[]>([]);
  const [cliStatus, setCliStatus] = useState<CliToolsStatusMap | null>(null);
  const [proxies, setProxies] = useState<ProxyWithQuota[]>([]);
  const [proxyTestIntervalSec, setProxyTestIntervalSec] = useState<number>(15);
  const [lastAutoTestAt, setLastAutoTestAt] = useState<number | null>(null);
  const [ipQuotas, setIpQuotas] = useState<IpQuotaWithProxies[]>([]);
  const [providerQuotas, setProviderQuotas] = useState<ProviderQuotaSummary[]>([]);
  const [lastQuotaSyncAt, setLastQuotaSyncAt] = useState<number>(() => Date.now());
  const [quotaProbeStatus, setQuotaProbeStatus] = useState<QuotaProbeStatus | null>(null);
  const [probingQuotas, setProbingQuotas] = useState(false);
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [keys, setKeys] = useState<ApiKeyView[]>([]);
  const [models, setModels] = useState<OpenAiModelEntry[]>([]);

  const [testingAllProxies, setTestingAllProxies] = useState(false);
  const [togglingAutostart, setTogglingAutostart] = useState(false);
  const [playgroundModel, setPlaygroundModel] = useState("oc/deepseek-v4.1-flash-free");

  // Toast feedback
  const [toast, setToast] = useState<{ text: string; isError: boolean } | null>(null);

  // Optional ADMIN_TOKEN unlock dialog
  const [authModalOpen, setAuthModalOpen] = useState(false);
  const [adminTokenInput, setAdminTokenInput] = useState(() => getStoredAdminToken());

  const notify = useCallback((text: string, isError = false) => {
    setToast({ text, isError });
    setTimeout(() => {
      setToast((cur) => (cur?.text === text ? null : cur));
    }, 3600);
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    try {
      localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // ignore
    }
  }, [theme]);

  useEffect(() => {
    const onPopState = () => {
      setRoute(normalizeRoutePath(window.location.pathname));
      try {
        setSelectedProviderId(new URLSearchParams(window.location.search).get("provider") || null);
      } catch {
        setSelectedProviderId(null);
      }
    };
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const navigate = useCallback((targetPath: RoutePath, query?: Record<string, string>) => {
    let url: string = targetPath;
    if (query && Object.keys(query).length > 0) {
      const sp = new URLSearchParams(query);
      url = `${targetPath}?${sp.toString()}`;
    }
    if (targetPath === "/providers") {
      setSelectedProviderId(query?.provider ?? null);
    }
    if (window.location.pathname + window.location.search !== url) {
      window.history.pushState({}, "", url);
    }
    setRoute(targetPath);
    setMobileNavOpen(false);
    window.scrollTo({ top: 0, behavior: "smooth" });
  }, []);

  const refreshLiveQuotas = useCallback(async () => {
    try {
      const [ipRes, pqRes, provCat] = await Promise.all([
        api.getIpQuotasWithMeta(),
        api.getProviderQuotasWithAccounts(),
        api.getProviders(),
      ]);
      setIpQuotas(ipRes.ip_quotas);
      setProviderQuotas(pqRes.providers);
      setAntigravityAccounts(pqRes.antigravity_accounts);
      setProvidersCatalog(provCat);
      if (ipRes.probe_status) {
        setQuotaProbeStatus(ipRes.probe_status);
      }
      setLastQuotaSyncAt(
        ipRes.probe_status?.last_probe_at || ipRes.refreshed_at || Date.now(),
      );
    } catch {
      // handled silently on background tick
    }
  }, []);

  const loadAll = useCallback(
    async (opts?: { silent?: boolean; periodOverride?: UsagePeriod }) => {
      const targetPeriod = opts?.periodOverride ?? period;
      if (!opts?.silent) setRefreshing(true);
      try {
        const [st, topo, provCat, cmbs, cliSt, pxMeta, ipMeta, pqWithAccounts, usg, ks, mdls] =
          await Promise.all([
            api.getStatus(),
            api.getTopology(),
            api.getProviders(),
            api.getCombos(),
            api.getCliToolsStatus(),
            api.getProxiesWithMeta(),
            api.getIpQuotasWithMeta(),
            api.getProviderQuotasWithAccounts(),
            api.getUsage(targetPeriod, 50),
            api.getKeys(),
            api.getModels(),
          ]);
        setStatus(st);
        setTopology(topo);
        setProvidersCatalog(provCat);
        setCombos(cmbs);
        setCliStatus(cliSt);
        setProxies(pxMeta.proxies);
        setProxyTestIntervalSec(pxMeta.test_interval_sec);
        setLastAutoTestAt(pxMeta.last_auto_test_at);
        setIpQuotas(ipMeta.ip_quotas);
        if (ipMeta.probe_status) {
          setQuotaProbeStatus(ipMeta.probe_status);
        }
        if (ipMeta.probe_status?.last_probe_at) {
          setLastQuotaSyncAt(ipMeta.probe_status.last_probe_at);
        } else if (!opts?.silent) {
          setLastQuotaSyncAt(ipMeta.refreshed_at);
        }
        setProviderQuotas(pqWithAccounts.providers);
        setAntigravityAccounts(pqWithAccounts.antigravity_accounts);
        setUsage(usg);
        setKeys(ks);
        setModels(mdls);
      } catch (err) {
        if (err instanceof ApiError && err.status === 401) {
          setAuthModalOpen(true);
        } else if (!opts?.silent) {
          notify(err instanceof Error ? err.message : "Failed to load gateway data", true);
        }
      } finally {
        setLoading(false);
        if (!opts?.silent) setRefreshing(false);
      }
    },
    [notify, period],
  );

  // Core dashboard & TesieCore proxy state polling
  useEffect(() => {
    void loadAll({ silent: true });
    const pollId = window.setInterval(() => {
      if (!authModalOpen) {
        void loadAll({ silent: true });
      }
    }, 5000);
    return () => window.clearInterval(pollId);
  }, [authModalOpen, loadAll]);

  // Dedicated 30-second live per-IP quota check loop
  useEffect(() => {
    const quotaId = window.setInterval(() => {
      if (!authModalOpen) {
        void refreshLiveQuotas();
      }
    }, 30_000);
    return () => window.clearInterval(quotaId);
  }, [authModalOpen, refreshLiveQuotas]);

  useEffect(() => {
    const tickId = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(tickId);
  }, []);

  function handleChangePeriod(next: UsagePeriod) {
    setPeriod(next);
    void (async () => {
      try {
        const nextUsage = await api.getUsage(next, 50);
        setUsage(nextUsage);
      } catch (err) {
        notify(err instanceof Error ? err.message : "Failed to switch period", true);
      }
    })();
  }

  // Proxy pool actions (TesieCore built-in latency test, edit, batch delete & NODE - N naming)
  async function handleCreateProxy(payload: CreateProxyPayload) {
    try {
      const created = await api.createProxy(payload);
      notify(`Added "${created.name}" to TesieCore pool`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to add proxy", true);
      throw err;
    }
  }

  async function handleUpdateProxy(id: string, payload: UpdateProxyPayload) {
    try {
      const updated = await api.updateProxy(id, payload);
      notify(`Updated proxy "${updated.name}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update proxy", true);
      throw err;
    }
  }

  async function handleImportProxies(urlsText: string) {
    try {
      const res = await api.importProxies(urlsText);
      notify(`Imported ${res.imported} proxies with sequential NODE naming`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Bulk import failed", true);
      throw err;
    }
  }

  async function handleDeleteProxy(id: string, name: string) {
    if (!window.confirm(`Remove "${name}" from the global proxy pool?`)) return;
    try {
      await api.deleteProxy(id);
      notify(`Removed "${name}" from pool`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to delete proxy", true);
    }
  }

  async function handleBatchDeleteProxies(ids: string[]) {
    if (ids.length === 0) return;
    try {
      const res = await api.batchDeleteProxies(ids);
      notify(`Deleted ${res.deleted} selected ${res.deleted === 1 ? "proxy" : "proxies"}`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Batch delete failed", true);
      throw err;
    }
  }

  async function handleNormalizeProxyNames() {
    try {
      const res = await api.normalizeProxyNames();
      notify(
        res.renamed > 0
          ? `Standardized ${res.renamed} proxies to NODE - 1..N`
          : "All proxies already follow NODE - 1..N naming",
      );
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to normalize proxy names", true);
    }
  }

  async function handleTestAllProxies() {
    setTestingAllProxies(true);
    try {
      const res = await api.testAllProxies();
      const healthy = res.tested.filter((t) => t.alive).length;
      notify(`TesieCore tested ${res.tested.length} proxies (${healthy} healthy)`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "TesieCore delay test failed", true);
    } finally {
      setTestingAllProxies(false);
    }
  }

  async function handleTestSingleProxy(id: string) {
    try {
      const res = await api.testSingleProxy(id);
      if (res.alive && res.latency_ms !== null) {
        notify(`${res.proxy.name}: ${res.latency_ms}ms (healthy)`);
      } else {
        notify(`${res.proxy.name}: timed out in TesieCore`, true);
      }
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Proxy test failed", true);
      throw err;
    }
  }

  async function handleUpdateTestInterval(seconds: number) {
    try {
      const res = await api.updateProxyTestInterval(seconds);
      setProxyTestIntervalSec(res.test_interval_sec);
      notify(`TesieCore auto-test interval set to ${res.test_interval_sec}s`);
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update test interval", true);
      throw err;
    }
  }

  // Quota actions — live-verified upstream availability check before clearing cooldown
  async function handleResetIpQuota(ipKey: string) {
    try {
      const res = await api.resetIpQuota(ipKey);
      notify(res.message, !res.reset);
      await refreshLiveQuotas();
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to verify & reset IP quota", true);
    }
  }

  async function handleResetAllCoolingQuotas() {
    try {
      const res = await api.resetAllCoolingIpQuotas();
      if (res.reset > 0) {
        notify(
          `Verified live: reset ${res.reset} ready IP${res.reset === 1 ? "" : "s"}${
            res.still_cooling > 0 ? ` (${res.still_cooling} still rate-limited)` : ""
          }`,
        );
      } else {
        notify(
          `Checked ${res.checked} cooling IP${res.checked === 1 ? "" : "s"} — upstream usage not yet available (${res.still_cooling} kept in cooldown)`,
          true,
        );
      }
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to verify cooling IPs", true);
    }
  }

  async function handleProbeQuotasNow() {
    setProbingQuotas(true);
    try {
      const res = await api.probeIpQuotasNow();
      setIpQuotas(res.ip_quotas);
      setQuotaProbeStatus(res.probe_status);
      setLastQuotaSyncAt(res.probe_status.last_probe_at || res.summary.checked_at || Date.now());
      notify(
        `Probed ${res.summary.checked} IPs via minimal OpenCode request (${res.summary.ready} ready, ${res.summary.cooling} cooling)`,
      );
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Live quota probe failed", true);
    } finally {
      setProbingQuotas(false);
    }
  }

  async function handleUpdateIpQuotaLimits(ipKey: string, payload: UpdateIpQuotaLimitsPayload) {
    try {
      await api.updateIpQuotaLimits(ipKey, payload);
      notify(`Saved limits for ${ipKey}`);
      await refreshLiveQuotas();
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to save IP limits", true);
      throw err;
    }
  }

  // Provider config & test actions
  async function handleUpdateProviderConfig(id: string, payload: UpdateProviderConfigPayload) {
    const provLabel =
      providersCatalog.find((p) => p.id === id)?.name ||
      (id === "antigravity" ? "Antigravity" : id === "opencode" ? "OpenCode" : id);
    try {
      await api.updateProviderConfig(id, payload);
      if (payload.thinking_level !== undefined) {
        notify(`OpenCode thinking level set to ${payload.thinking_level}`);
      } else if (payload.routing_strategy !== undefined) {
        notify(`Set ${provLabel} routing strategy to ${payload.routing_strategy}`);
      } else if (payload.probe_interval_sec !== undefined) {
        notify(`Auto-saved ${provLabel} probe interval (${payload.probe_interval_sec}s)`);
      } else if (payload.probe_enabled !== undefined) {
        notify(
          payload.probe_enabled
            ? `Enabled ${provLabel} quota probes`
            : `Disabled ${provLabel} quota probes`,
        );
      } else {
        notify(`Saved ${provLabel} proxy & routing settings`);
      }
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update provider settings", true);
      throw err;
    }
  }

  async function handleCreateCustomProvider(
    payload: CreateCustomProviderPayload,
  ): Promise<string> {
    try {
      const created = await api.createCustomProvider(payload);
      notify(`Created custom provider "${created.name}" (${created.prefix}/*)`);
      await loadAll({ silent: true });
      return created.id;
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to create custom provider", true);
      throw err;
    }
  }

  async function handleUpdateCustomProvider(
    id: string,
    payload: UpdateCustomProviderPayload,
  ) {
    try {
      const updated = await api.updateCustomProvider(id, payload);
      notify(`Saved custom provider "${updated.name}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update custom provider", true);
      throw err;
    }
  }

  async function handleDeleteCustomProvider(id: string, name: string) {
    try {
      await api.deleteCustomProvider(id);
      notify(`Removed custom provider "${name}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to delete custom provider", true);
      throw err;
    }
  }

  async function handleRefreshCustomFavicon(id: string) {
    try {
      const res = await api.refreshCustomProviderFavicon(id);
      notify(
        res.favicon_url
          ? `Refreshed website icon for ${res.custom_provider.name}`
          : `No website icon found on ${res.custom_provider.base_url}`,
        !res.favicon_url,
      );
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to refresh favicon", true);
    }
  }

  async function handleFetchCustomModels(id: string) {
    try {
      const res = await api.fetchCustomProviderModels(id, true);
      notify(
        `Synced ${res.models.length} model(s) from ${res.custom_provider.name}`,
      );
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to fetch upstream /models", true);
    }
  }

  async function handleTestProvider(id: string): Promise<ProviderTestResult> {
    const res = await api.testProvider(id);
    if (res.ok) {
      notify(`${id}: ${res.message}`);
    } else {
      notify(`${id}: ${res.message}`, true);
    }
    return res;
  }

  // Antigravity OAuth & Account Management actions
  async function handleStartAntigravityOAuth(): Promise<StartedOAuthFlow> {
    return api.startAntigravityOAuth();
  }

  const handlePollAntigravityOAuth = useCallback(
    async (state: string): Promise<OAuthSessionPollStatus> => {
      const res = await api.pollAntigravityOAuth(state);
      if (res.status === "completed") {
        notify(`Connected Antigravity Google account ${res.email || ""}`.trim());
        await loadAll({ silent: true });
      }
      return res;
    },
    [loadAll, notify],
  );

  async function handleCompleteAntigravityOAuth(payload: {
    callback_url: string;
    redirect_uri?: string;
  }) {
    const res = await api.completeAntigravityOAuth(payload);
    setAntigravityAccounts(res.accounts);
    notify("Connected Antigravity Google account");
    await loadAll({ silent: true });
  }

  async function handleImportAntigravityAccount(payload: {
    name?: string;
    email?: string;
    refresh_token: string;
    project_id?: string;
  }) {
    const res = await api.importAntigravityAccount(payload);
    setAntigravityAccounts(res.accounts);
    notify("Imported Antigravity Google account & synced Cloud Code quotas");
    await loadAll({ silent: true });
  }

  async function handleRefreshAntigravityQuotas() {
    try {
      const res = await api.refreshAntigravityQuotas();
      setAntigravityAccounts(res.accounts);
      notify(`Refreshed Cloud Code quotas for ${res.refreshed} Antigravity account(s)`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to refresh Antigravity quotas", true);
    }
  }

  async function handleRefreshSingleAntigravityQuota(id: string) {
    try {
      const res = await api.refreshSingleAntigravityQuota(id);
      setAntigravityAccounts(res.accounts);
      notify("Synced Cloud Code model quotas");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to sync account quota", true);
    }
  }

  async function handleUpdateAccountConnection(
    id: string,
    payload: { name?: string; enabled?: boolean; reset_cooldown?: boolean },
  ) {
    try {
      await api.updateConnection(id, payload);
      notify("Updated Antigravity account");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update account", true);
    }
  }

  async function handleDeleteAccountConnection(id: string, label: string) {
    if (!window.confirm(`Disconnect Antigravity account "${label}"?`)) return;
    try {
      await api.deleteConnection(id);
      notify(`Disconnected "${label}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to delete account", true);
    }
  }

  // Combo actions
  async function handleCreateCombo(payload: CreateComboPayload) {
    try {
      const created = await api.createCombo(payload);
      notify(`Created combo "${created.name}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to create combo", true);
      throw err;
    }
  }

  async function handleUpdateCombo(id: string, payload: UpdateComboPayload) {
    try {
      await api.updateCombo(id, payload);
      notify("Updated combo");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update combo", true);
      throw err;
    }
  }

  async function handleDeleteCombo(id: string, name: string) {
    if (!window.confirm(`Delete combo "${name}"?`)) return;
    try {
      await api.deleteCombo(id);
      notify(`Deleted combo "${name}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to delete combo", true);
    }
  }

  // CLI Tools actions
  async function handleApplyOpenCode(payload: ApplyOpenCodeCliPayload) {
    try {
      await api.applyOpenCodeCli(payload);
      notify("Configured OpenCode CLI (~/.config/opencode/opencode.json)");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to configure OpenCode", true);
      throw err;
    }
  }

  async function handleResetOpenCode() {
    try {
      await api.resetOpenCodeCli();
      notify("Removed Tesie from OpenCode config");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to reset OpenCode", true);
    }
  }

  async function handleApplyClaude(payload: ApplyClaudeCliPayload) {
    try {
      await api.applyClaudeCli(payload);
      notify("Configured Claude Code CLI (~/.claude/settings.json)");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to configure Claude Code", true);
      throw err;
    }
  }

  async function handleResetClaude() {
    try {
      await api.resetClaudeCli();
      notify("Reset Claude Code settings");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to reset Claude Code", true);
    }
  }

  async function handleApplyCodex(payload: ApplyCodexCliPayload) {
    try {
      await api.applyCodexCli(payload);
      notify("Configured Codex CLI (~/.codex/config.toml)");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to configure Codex", true);
      throw err;
    }
  }

  async function handleResetCodex() {
    try {
      await api.resetCodexCli();
      notify("Reset Codex CLI config");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to reset Codex", true);
    }
  }

  // API key actions
  async function handleCreateKey(payload: CreateApiKeyPayload): Promise<CreatedApiKeyResponse> {
    try {
      const created = await api.createKey(payload);
      notify(`Created API key "${created.name}"`);
      await loadAll({ silent: true });
      return created;
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to create API key", true);
      throw err;
    }
  }

  async function handleUpdateKey(id: string, payload: UpdateApiKeyPayload) {
    try {
      await api.updateKey(id, payload);
      notify("Updated client API key");
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to update API key", true);
    }
  }

  async function handleDeleteKey(id: string, name: string) {
    if (!window.confirm(`Delete client API key "${name}"?`)) return;
    try {
      await api.deleteKey(id);
      notify(`Deleted API key "${name}"`);
      await loadAll({ silent: true });
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to delete API key", true);
    }
  }

  function handleTestModel(modelId: string) {
    setPlaygroundModel(modelId);
    navigate("/playground");
  }

  async function handleToggleAutostart() {
    setTogglingAutostart(true);
    try {
      const next = !(status?.auto_start_enabled ?? false);
      const res = await api.updateAutostart(next);
      setStatus((prev) =>
        prev
          ? { ...prev, auto_start_enabled: res.auto_start_enabled, data_dir: res.data_dir }
          : prev,
      );
      notify(
        res.auto_start_enabled
          ? "Enabled background auto-start on PC boot"
          : "Disabled background auto-start on PC boot",
      );
    } catch (err) {
      notify(err instanceof Error ? err.message : "Failed to toggle auto-start", true);
    } finally {
      setTogglingAutostart(false);
    }
  }

  function handleSaveAdminToken(e: React.FormEvent) {
    e.preventDefault();
    setStoredAdminToken(adminTokenInput);
    setAuthModalOpen(false);
    void loadAll();
  }

  const allNavItems = NAV_GROUPS.flatMap((g) => g.items);
  const currentMeta = allNavItems.find((i) => i.path === route) ?? allNavItems[0];
  const activeStreamCount = topology?.active_requests.length ?? 0;
  const sampleModel =
    combos.find((c) => c.enabled)?.name || "oc/deepseek-v4.1-flash-free";
  const coreOnline = Boolean(status?.core_healthy ?? status?.mihomo_healthy);
  const externalProxies = proxies.filter((p) => !p.is_direct);
  const healthyProxiesCount = externalProxies.filter((p) => p.alive).length;
  const timeoutProxiesCount = externalProxies.filter((p) => !p.alive).length;
  const validProxyDelays = externalProxies
    .filter((p) => p.alive && p.latency_ms !== null && p.latency_ms > 0)
    .map((p) => p.latency_ms as number);
  const avgProxyLatencyMs =
    validProxyDelays.length > 0
      ? Math.round(validProxyDelays.reduce((a, b) => a + b, 0) / validProxyDelays.length)
      : null;

  const sidebarContent = (
    <div className="flex h-full flex-col justify-between p-4 sm:p-5">
      <div className="space-y-6">
        {/* Brand */}
        <div className="flex items-center justify-between gap-2 px-1">
          <button
            type="button"
            onClick={() => navigate("/")}
            className="flex items-center gap-3 text-left"
          >
            <TesieLogo size={36} />
            <div>
              <div className="font-[family-name:var(--font-display)] text-base font-semibold tracking-tight text-[var(--text-primary)]">
                Tesie
              </div>
              <div className="text-xs text-[var(--text-muted)]">AI Routing Gateway</div>
            </div>
          </button>
          <button
            type="button"
            onClick={() => setMobileNavOpen(false)}
            className="rounded-lg p-1.5 text-[var(--text-muted)] hover:bg-[var(--bg-hover)] lg:hidden"
            aria-label="Close navigation"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Grouped Navigation */}
        <nav className="space-y-5" aria-label="Primary navigation">
          {NAV_GROUPS.map((group) => (
            <div key={group.label} className="space-y-1">
              <div className="px-3 pb-1 text-xs font-semibold uppercase tracking-wider text-[var(--text-muted)]">
                {group.label}
              </div>
              {group.items.map((item) => {
                const Icon = item.icon;
                const active = route === item.path;
                return (
                  <button
                    key={item.path}
                    type="button"
                    onClick={() => navigate(item.path)}
                    className={`flex w-full items-center justify-between gap-2.5 rounded-xl px-3 py-2.5 text-left text-sm font-medium transition-colors ${
                      active
                        ? "border border-[var(--border-subtle)] bg-[var(--bg-surface)] text-[var(--text-primary)]"
                        : "border border-transparent text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-primary)]"
                    }`}
                  >
                    <span className="flex min-w-0 items-center gap-2.5">
                      <Icon
                        className={`h-[18px] w-[18px] shrink-0 ${
                          active ? "text-[var(--accent)]" : "text-[var(--text-muted)]"
                        }`}
                      />
                      <span className="truncate">{item.label}</span>
                    </span>
                    {item.path === "/combos" && combos.length > 0 && (
                      <span className="rounded-full bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-xs text-[var(--text-muted)]">
                        {combos.length}
                      </span>
                    )}
                    {item.path === "/providers" && providersCatalog.length > 0 && (
                      <span className="rounded-full bg-[var(--bg-elevated)] px-2 py-0.5 font-mono text-xs text-[var(--text-muted)]">
                        {providersCatalog.length}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          ))}
        </nav>
      </div>

      {/* Bottom TesieCore Live Proxy Pool Status Only */}
      <button
        type="button"
        onClick={() => navigate("/proxies")}
        title="View and manage global TesieCore proxy pool"
        className="w-full space-y-3 rounded-2xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-3.5 text-left text-[13px] transition-colors hover:border-[var(--border-strong)]"
      >
        <div className="flex items-center justify-between gap-2">
          <span className="font-semibold text-[var(--text-primary)]">Proxy Pool</span>
          <StatusPill tone={coreOnline ? "success" : "danger"} dot>
            {coreOnline ? "Online" : "Offline"}
          </StatusPill>
        </div>

        <div className="grid grid-cols-2 gap-2 text-xs tabular-nums">
          <div className="rounded-lg bg-[var(--bg-elevated)] px-2.5 py-2">
            <div className="text-[var(--text-muted)]">Healthy</div>
            <div className="mt-0.5 font-mono text-base font-semibold text-[var(--success)]">
              {loading && externalProxies.length === 0 ? "—" : healthyProxiesCount}
            </div>
          </div>
          <div className="rounded-lg bg-[var(--bg-elevated)] px-2.5 py-2">
            <div className="text-[var(--text-muted)]">Timed out</div>
            <div
              className={`mt-0.5 font-mono text-base font-semibold ${
                timeoutProxiesCount > 0 ? "text-[var(--danger)]" : "text-[var(--text-muted)]"
              }`}
            >
              {loading && externalProxies.length === 0 ? "—" : timeoutProxiesCount}
            </div>
          </div>
        </div>

        <div className="flex items-center justify-between gap-2 px-0.5 font-mono text-xs text-[var(--text-muted)] tabular-nums">
          <span>
            {externalProxies.length} {externalProxies.length === 1 ? "node" : "nodes"}
            {avgProxyLatencyMs !== null ? ` · ${avgProxyLatencyMs}ms` : ""}
          </span>
          <span>{proxyTestIntervalSec}s</span>
        </div>
      </button>
    </div>
  );

  return (
    <div className="flex min-h-screen bg-[var(--bg-canvas)] text-[var(--text-primary)]">
      {/* Desktop Sidebar */}
      <aside className="hidden w-64 shrink-0 border-r border-[var(--border-subtle)] bg-[var(--bg-sidebar)] lg:block">
        <div className="sticky top-0 h-screen overflow-y-auto">{sidebarContent}</div>
      </aside>

      {/* Mobile Drawer Sidebar */}
      {mobileNavOpen && (
        <div className="fixed inset-0 z-40 flex lg:hidden">
          <div
            className="fixed inset-0 bg-black/55 backdrop-blur-[2px]"
            onClick={() => setMobileNavOpen(false)}
          />
          <aside className="relative z-10 h-full w-72 border-r border-[var(--border-subtle)] bg-[var(--bg-sidebar)]">
            {sidebarContent}
          </aside>
        </div>
      )}

      {/* Main Content */}
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="sticky top-0 z-20 flex h-16 items-center justify-between gap-3 border-b border-[var(--border-subtle)] bg-[var(--bg-surface)]/92 px-4 backdrop-blur-md sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <button
              type="button"
              onClick={() => setMobileNavOpen(true)}
              className="rounded-lg border border-[var(--border-subtle)] p-2 text-[var(--text-secondary)] hover:bg-[var(--bg-hover)] lg:hidden"
              aria-label="Open navigation"
            >
              <Menu className="h-4 w-4" />
            </button>
            <div className="min-w-0">
              <h1 className="truncate text-base font-semibold tracking-tight text-[var(--text-primary)]">
                {currentMeta.label}
              </h1>
              <p className="hidden truncate text-xs text-[var(--text-muted)] sm:block">
                {currentMeta.subtitle}
              </p>
            </div>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {activeStreamCount > 0 && (
              <StatusPill tone="accent" dot pulse>
                <Activity className="h-3.5 w-3.5" />
                {activeStreamCount} streaming
              </StatusPill>
            )}

            <Button
              variant="ghost"
              size="sm"
              className="hidden md:inline-flex"
              disabled={togglingAutostart}
              onClick={() => void handleToggleAutostart()}
              title="Keep Tesie running silently in the background even after PC restart"
            >
              <Power
                className={`h-4 w-4 ${
                  status?.auto_start_enabled ? "text-[var(--success)]" : "text-[var(--text-muted)]"
                }`}
              />
              <span className="text-[var(--text-muted)]">Boot</span>
              <span
                className={`font-mono text-xs font-semibold ${
                  status?.auto_start_enabled ? "text-[var(--success)]" : "text-[var(--text-muted)]"
                }`}
              >
                {status?.auto_start_enabled ? "ON" : "OFF"}
              </span>
            </Button>

            <div className="inline-flex items-center rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)]/60 p-1">
              {status?.data_dir && (
                <button
                  type="button"
                  onClick={() => {
                    if (status.data_dir && navigator.clipboard) {
                      void navigator.clipboard.writeText(status.data_dir);
                      notify("Copied dedicated data path");
                    }
                  }}
                  className="rounded-md p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)]"
                  title={`Copy data path: ${status.data_dir}`}
                  aria-label="Copy data directory"
                >
                  <HardDrive className="h-4 w-4" />
                </button>
              )}

              <button
                type="button"
                onClick={() => navigate("/settings")}
                className="hidden rounded-md p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)] sm:inline-flex"
                title="Tesie Settings, Backup & Security"
                aria-label="Settings"
              >
                <Settings className="h-4 w-4" />
              </button>

              <button
                type="button"
                onClick={() => {
                  setLogsRefreshSignal((n) => n + 1);
                  void loadAll();
                }}
                className="rounded-md p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)]"
                title="Refresh gateway telemetry"
                aria-label="Refresh telemetry"
              >
                <RefreshCw className={`h-4 w-4 ${refreshing ? "animate-spin" : ""}`} />
              </button>

              <button
                type="button"
                onClick={() => setTheme((prev) => (prev === "dark" ? "light" : "dark"))}
                className="inline-flex items-center gap-1 rounded-md px-2 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-surface)] hover:text-[var(--text-primary)]"
                title={`Switch to ${theme === "dark" ? "light" : "dark"} mode`}
                aria-label="Toggle color theme"
              >
                {theme === "dark" ? (
                  <Sun className="h-4 w-4" />
                ) : (
                  <Moon className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>
        </header>

        <main className="mx-auto w-full max-w-[1560px] flex-1 px-4 py-6 sm:px-6">
          <div key={route} className="animate-view-enter">
            {route === "/" && (
              <DashboardView
                period={period}
                onChangePeriod={handleChangePeriod}
                usage={usage}
                topology={topology}
                loading={loading}
                nowMs={nowMs}
                onInspectProviderQuota={(pid) => {
                  setSelectedQuotaProviderId(pid);
                  navigate("/quotas");
                }}
                onOpenAllLogs={() => navigate("/logs")}
              />
            )}

            {route === "/logs" && (
              <LogsView
                proxies={proxies}
                refreshSignal={logsRefreshSignal}
                onNotify={notify}
              />
            )}

            {route === "/providers" && (
              <ProvidersModelsView
                providers={providersCatalog}
                proxies={proxies}
                models={models}
                ipQuotas={ipQuotas}
                antigravityAccounts={antigravityAccounts}
                initialProviderId={selectedProviderId}
                onSelectProvider={(pid) => {
                  setSelectedProviderId(pid);
                  navigate("/providers", pid ? { provider: pid } : undefined);
                }}
                lastQuotaSyncAt={lastQuotaSyncAt}
                quotaProbeStatus={quotaProbeStatus}
                probingQuotas={probingQuotas}
                loading={loading}
                nowMs={nowMs}
                onTestModel={handleTestModel}
                onOpenQuotaTracker={(pid) => {
                  setSelectedQuotaProviderId(pid);
                  navigate("/quotas");
                }}
                onOpenProxyPool={() => navigate("/proxies")}
                onRefreshQuotas={refreshLiveQuotas}
                onProbeQuotasNow={handleProbeQuotasNow}
                onUpdateProviderConfig={handleUpdateProviderConfig}
                onTestProvider={handleTestProvider}
                onStartAntigravityOAuth={handleStartAntigravityOAuth}
                onPollAntigravityOAuth={handlePollAntigravityOAuth}
                onCompleteAntigravityOAuth={handleCompleteAntigravityOAuth}
                onImportAntigravityAccount={handleImportAntigravityAccount}
                onRefreshAntigravityQuotas={handleRefreshAntigravityQuotas}
                onRefreshSingleAntigravityQuota={handleRefreshSingleAntigravityQuota}
                onUpdateAccountConnection={handleUpdateAccountConnection}
                onDeleteAccountConnection={handleDeleteAccountConnection}
                onCreateCustomProvider={handleCreateCustomProvider}
                onUpdateCustomProvider={handleUpdateCustomProvider}
                onDeleteCustomProvider={handleDeleteCustomProvider}
                onRefreshCustomFavicon={handleRefreshCustomFavicon}
                onFetchCustomModels={handleFetchCustomModels}
              />
            )}

            {route === "/combos" && (
              <CombosView
                combos={combos}
                models={models}
                providers={providersCatalog}
                loading={loading}
                onCreateCombo={handleCreateCombo}
                onUpdateCombo={handleUpdateCombo}
                onDeleteCombo={handleDeleteCombo}
                onTestModel={handleTestModel}
              />
            )}

            {route === "/token-saver" && (
              <TokenSaverView nowMs={nowMs} onNotify={notify} />
            )}

            {route === "/bridge" && (
              <MultimodalBridgeView onNotify={notify} />
            )}

            {route === "/quotas" && (
              <QuotaTrackerView
                providers={providersCatalog}
                providerQuotas={providerQuotas}
                ipQuotas={ipQuotas}
                proxies={proxies}
                antigravityAccounts={antigravityAccounts}
                initialProviderId={selectedQuotaProviderId}
                lastQuotaSyncAt={lastQuotaSyncAt}
                quotaProbeStatus={quotaProbeStatus}
                probingQuotas={probingQuotas}
                loading={loading}
                onNotify={notify}
                onOpenProviderSettings={(pid) => {
                  setSelectedProviderId(pid);
                  navigate("/providers", { provider: pid });
                }}
                nowMs={nowMs}
                onRefreshQuotas={refreshLiveQuotas}
                onProbeQuotasNow={handleProbeQuotasNow}
                onResetIpQuota={handleResetIpQuota}
                onResetAllCooling={handleResetAllCoolingQuotas}
                onUpdateIpQuotaLimits={handleUpdateIpQuotaLimits}
                onUpdateProviderConfig={handleUpdateProviderConfig}
                onRefreshAntigravityQuotas={handleRefreshAntigravityQuotas}
                onRefreshSingleAntigravityQuota={handleRefreshSingleAntigravityQuota}
                onUpdateAccountConnection={handleUpdateAccountConnection}
              />
            )}

            {route === "/proxies" && (
              <ProxyPoolView
                proxies={proxies}
                testIntervalSec={proxyTestIntervalSec}
                lastAutoTestAt={lastAutoTestAt}
                loading={loading}
                nowMs={nowMs}
                testingAll={testingAllProxies}
                onCreateProxy={handleCreateProxy}
                onUpdateProxy={handleUpdateProxy}
                onImportProxies={handleImportProxies}
                onDeleteProxy={handleDeleteProxy}
                onBatchDeleteProxies={handleBatchDeleteProxies}
                onNormalizeProxyNames={handleNormalizeProxyNames}
                onTestAll={handleTestAllProxies}
                onTestOne={handleTestSingleProxy}
                onUpdateTestInterval={handleUpdateTestInterval}
                onOpenProviders={() => navigate("/providers")}
              />
            )}

            {route === "/cli-tools" && (
              <CliToolsView
                cliStatus={cliStatus}
                models={models}
                combos={combos}
                keys={keys}
                loading={loading}
                onApplyOpenCode={handleApplyOpenCode}
                onResetOpenCode={handleResetOpenCode}
                onApplyClaude={handleApplyClaude}
                onResetClaude={handleResetClaude}
                onApplyCodex={handleApplyCodex}
                onResetCodex={handleResetCodex}
                onCreateKey={handleCreateKey}
              />
            )}

            {route === "/endpoint" && (
              <EndpointKeysView
                keys={keys}
                sampleModel={sampleModel}
                loading={loading}
                nowMs={nowMs}
                onCreateKey={handleCreateKey}
                onUpdateKey={handleUpdateKey}
                onDeleteKey={handleDeleteKey}
                onOpenCliTools={() => navigate("/cli-tools")}
              />
            )}

            {route === "/playground" && (
              <PlaygroundView
                models={models}
                keys={keys}
                initialModel={playgroundModel}
                onCompleted={() => {
                  setLogsRefreshSignal((n) => n + 1);
                  void loadAll({ silent: true });
                }}
              />
            )}

            {route === "/settings" && (
              <SettingsView
                onNotify={notify}
                onReloadAll={async () => {
                  await loadAll({ silent: true });
                }}
                onAutostartChanged={(enabled) =>
                  setStatus((prev) => (prev ? { ...prev, auto_start_enabled: enabled } : prev))
                }
              />
            )}
          </div>
        </main>
      </div>

      {/* Toast Notification */}
      {toast && (
        <div
          role="status"
          className={`animate-toast-in fixed bottom-5 right-5 z-50 flex max-w-sm items-center gap-2 rounded-xl border px-4 py-2.5 text-xs font-medium shadow-[var(--shadow-float)] ${
            toast.isError
              ? "border-[color-mix(in_srgb,var(--danger)_35%,transparent)] bg-[var(--bg-surface)] text-[var(--danger)]"
              : "border-[color-mix(in_srgb,var(--success)_35%,transparent)] bg-[var(--bg-surface)] text-[var(--text-primary)]"
          }`}
        >
          {toast.isError ? (
            <AlertCircle className="h-4 w-4 shrink-0 text-[var(--danger)]" />
          ) : (
            <CheckCircle2 className="h-4 w-4 shrink-0 text-[var(--success)]" />
          )}
          <span>{toast.text}</span>
        </div>
      )}

      {/* Optional Dashboard Password Prompt (shown automatically if server responds 401) */}
      <Modal
        open={authModalOpen}
        title="Unlock Tesie Dashboard"
        onClose={() => setAuthModalOpen(false)}
      >
        <form onSubmit={handleSaveAdminToken} className="space-y-4">
          <p className="text-xs leading-relaxed text-[var(--text-muted)]">
            This Tesie gateway has Dashboard Password Protection enabled. Enter your Dashboard
            Password (or server <code className="font-mono">ADMIN_TOKEN</code>) to continue. You
            can manage or remove this lock anytime in Settings.
          </p>
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              Dashboard Password
            </label>
            <input
              type="password"
              placeholder="Enter dashboard password…"
              value={adminTokenInput}
              onChange={(e) => setAdminTokenInput(e.target.value)}
              className={`${INPUT_CLASS} font-mono`}
            />
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button variant="secondary" size="md" onClick={() => setAuthModalOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" size="md">
              Unlock &amp; reconnect
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  );
}

export default App;
