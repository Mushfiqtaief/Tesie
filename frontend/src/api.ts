import type {
  AntigravityAccountView,
  AntigravityMitmConfig,
  AntigravityQuotaSnapshot,
  ApiKeyView,
  ApplyClaudeCliPayload,
  ApplyCodexCliPayload,
  ApplyOpenCodeCliPayload,
  AuthStatusResponse,
  BackupImportResponse,
  BridgeEvent,
  BridgeStats,
  ClaudeCliStatus,
  CliToolsStatusMap,
  CodexCliStatus,
  ComboRecord,
  CreateApiKeyPayload,
  CreateComboPayload,
  CreateCustomProviderPayload,
  CreatedApiKeyResponse,
  CreateProxyPayload,
  CustomProvider,
  GatewaySettings,
  IpQuotaWithProxies,
  LogQueryParams,
  MihomoProxyState,
  MultimodalBridgeConfig,
  OAuthSessionPollStatus,
  OpenAiModelEntry,
  OpenCodeCliStatus,
  ProviderCatalogItem,
  ProviderConfig,
  ProviderQuotaSummary,
  ProviderTestResult,
  ProxiesListResponse,
  ProxyWithQuota,
  QuotaProbeStatus,
  QuotaProbeSummary,
  ResetCooldownsResponse,
  SettingsFullResponse,
  StartedOAuthFlow,
  StatusResponse,
  TokenSaverConfig,
  TokenSaverStateResponse,
  TokenSaverTestResponse,
  TopologyResponse,
  UpdateApiKeyPayload,
  UpdateApplyResponse,
  UpdateBridgePayload,
  UpdateCheckResponse,
  UpdateComboPayload,
  UpdateCustomProviderPayload,
  UpdateIpQuotaLimitsPayload,
  UpdateMitmConfigPayload,
  UpdateProviderConfigPayload,
  UpdateProxyPayload,
  UpdateSettingsExtendedPayload,
  UsageLogRecord,
  UsagePeriod,
  UsageSummary,
  VerifiedBatchResetResult,
  VerifiedQuotaResetResult,
} from "./types";

const ADMIN_TOKEN_STORAGE_KEY = "tesie_admin_token";
const LEGACY_ADMIN_TOKEN_KEY = "tesroute_admin_token";

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

export function getStoredAdminToken(): string {
  try {
    return (
      localStorage.getItem(ADMIN_TOKEN_STORAGE_KEY) ||
      localStorage.getItem(LEGACY_ADMIN_TOKEN_KEY) ||
      ""
    );
  } catch {
    return "";
  }
}

export function setStoredAdminToken(token: string): void {
  try {
    const clean = token.trim();
    if (clean) {
      localStorage.setItem(ADMIN_TOKEN_STORAGE_KEY, clean);
    } else {
      localStorage.removeItem(ADMIN_TOKEN_STORAGE_KEY);
    }
  } catch {
    // ignore storage failures
  }
}

async function requestJson<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const token = getStoredAdminToken();
  if (token) {
    headers.set("X-Admin-Token", token);
  }

  const resp = await fetch(path, { ...init, headers });
  if (resp.status === 204) {
    return undefined as T;
  }

  const text = await resp.text();
  let parsed: Record<string, unknown> | null = null;
  try {
    parsed = text ? (JSON.parse(text) as Record<string, unknown>) : null;
  } catch {
    parsed = null;
  }

  if (!resp.ok) {
    const msg =
      (parsed && typeof parsed.error === "string" && parsed.error) ||
      (parsed &&
        typeof parsed.error === "object" &&
        parsed.error !== null &&
        typeof (parsed.error as Record<string, unknown>).message === "string" &&
        ((parsed.error as Record<string, unknown>).message as string)) ||
      text ||
      `HTTP ${resp.status}`;
    throw new ApiError(resp.status, msg);
  }

  return (parsed ?? {}) as T;
}

export const api = {
  getStatus: () => requestJson<StatusResponse>("/api/admin/status"),

  updateAutostart: (enabled: boolean) =>
    requestJson<{ auto_start_enabled: boolean; daemon_path: string; data_dir: string }>(
      "/api/admin/system/autostart",
      {
        method: "POST",
        body: JSON.stringify({ enabled }),
      },
    ),

  getTopology: () => requestJson<TopologyResponse>("/api/admin/topology"),

  getProxiesWithMeta: async (): Promise<ProxiesListResponse> => {
    const res = await requestJson<ProxiesListResponse>("/api/admin/proxies");
    return {
      proxies: res.proxies || [],
      test_interval_sec: res.test_interval_sec ?? 15,
      last_auto_test_at: res.last_auto_test_at ?? null,
    };
  },

  getProxies: async (): Promise<ProxyWithQuota[]> => {
    const res = await requestJson<{ proxies: ProxyWithQuota[] }>("/api/admin/proxies");
    return res.proxies || [];
  },

  createProxy: async (payload: CreateProxyPayload): Promise<ProxyWithQuota> => {
    const res = await requestJson<{ proxy: ProxyWithQuota }>("/api/admin/proxies", {
      method: "POST",
      body: JSON.stringify(payload),
    });
    return res.proxy;
  },

  importProxies: async (
    urls_text: string,
  ): Promise<{ imported: number; skipped?: string[]; proxies: ProxyWithQuota[] }> => {
    return requestJson<{ imported: number; skipped?: string[]; proxies: ProxyWithQuota[] }>(
      "/api/admin/proxies/import",
      {
        method: "POST",
        body: JSON.stringify({ urls_text }),
      },
    );
  },

  updateProxyTestInterval: async (
    test_interval_sec: number,
  ): Promise<{ test_interval_sec: number; last_auto_test_at: number | null }> => {
    return requestJson("/api/admin/proxies/test-config", {
      method: "PUT",
      body: JSON.stringify({ test_interval_sec }),
    });
  },

  updateProxy: async (id: string, payload: UpdateProxyPayload): Promise<ProxyWithQuota> => {
    const res = await requestJson<{ proxy: ProxyWithQuota }>(
      `/api/admin/proxies/${encodeURIComponent(id)}`,
      {
        method: "PATCH",
        body: JSON.stringify(payload),
      },
    );
    return res.proxy;
  },

  deleteProxy: async (id: string): Promise<void> => {
    await requestJson(`/api/admin/proxies/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  },

  batchDeleteProxies: async (ids: string[]): Promise<{ deleted: number }> => {
    return requestJson("/api/admin/proxies/batch-delete", {
      method: "POST",
      body: JSON.stringify({ ids }),
    });
  },

  normalizeProxyNames: async (): Promise<{ renamed: number }> => {
    return requestJson("/api/admin/proxies/normalize-names", {
      method: "POST",
    });
  },

  testAllProxies: async (): Promise<{
    tested: MihomoProxyState[];
    last_auto_test_at: number | null;
  }> => {
    return requestJson("/api/admin/proxies/test", {
      method: "POST",
    });
  },

  testSingleProxy: async (
    id: string,
  ): Promise<{ proxy: ProxyWithQuota; alive: boolean; latency_ms: number | null }> => {
    return requestJson(`/api/admin/proxies/${encodeURIComponent(id)}/test`, {
      method: "POST",
    });
  },

  getIpQuotasWithMeta: async (): Promise<{
    ip_quotas: IpQuotaWithProxies[];
    refreshed_at: number;
    refresh_interval_sec: number;
    probe_status: QuotaProbeStatus | null;
  }> => {
    const res = await requestJson<{
      ip_quotas: IpQuotaWithProxies[];
      refreshed_at?: number;
      refresh_interval_sec?: number;
      probe_status?: QuotaProbeStatus;
    }>("/api/admin/quotas/ips");
    return {
      ip_quotas: res.ip_quotas || [],
      refreshed_at: res.refreshed_at ?? Date.now(),
      refresh_interval_sec: res.refresh_interval_sec ?? 30,
      probe_status: res.probe_status ?? null,
    };
  },

  probeIpQuotasNow: async (): Promise<{
    summary: QuotaProbeSummary;
    ip_quotas: IpQuotaWithProxies[];
    probe_status: QuotaProbeStatus;
  }> => {
    return requestJson("/api/admin/quotas/ips/check", {
      method: "POST",
    });
  },

  getIpQuotas: async (): Promise<IpQuotaWithProxies[]> => {
    const res = await requestJson<{ ip_quotas: IpQuotaWithProxies[] }>("/api/admin/quotas/ips");
    return res.ip_quotas || [];
  },

  getProviderQuotas: async (): Promise<ProviderQuotaSummary[]> => {
    const res = await requestJson<{ providers: ProviderQuotaSummary[] }>(
      "/api/admin/quotas/providers",
    );
    return res.providers || [];
  },

  getProviderQuotasWithAccounts: async (): Promise<{
    providers: ProviderQuotaSummary[];
    antigravity_accounts: AntigravityAccountView[];
  }> => {
    const res = await requestJson<{
      providers: ProviderQuotaSummary[];
      antigravity_accounts?: AntigravityAccountView[];
    }>("/api/admin/quotas/providers");
    return {
      providers: res.providers || [],
      antigravity_accounts: res.antigravity_accounts || [],
    };
  },

  updateIpQuotaLimits: async (
    ipKey: string,
    payload: UpdateIpQuotaLimitsPayload,
  ): Promise<void> => {
    await requestJson(`/api/admin/quotas/ips/${encodeURIComponent(ipKey)}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },

  resetIpQuota: async (ipKey: string): Promise<VerifiedQuotaResetResult> => {
    return requestJson<VerifiedQuotaResetResult>(
      `/api/admin/quotas/ips/${encodeURIComponent(ipKey)}/reset`,
      {
        method: "POST",
      },
    );
  },

  resetAllCoolingIpQuotas: async (): Promise<VerifiedBatchResetResult> => {
    return requestJson<VerifiedBatchResetResult>("/api/admin/quotas/ips/reset-all", {
      method: "POST",
    });
  },

  getAntigravityAccounts: async (): Promise<AntigravityAccountView[]> => {
    const res = await requestJson<{ accounts: AntigravityAccountView[] }>(
      "/api/admin/antigravity/accounts",
    );
    return res.accounts || [];
  },

  startAntigravityOAuth: async (): Promise<StartedOAuthFlow> => {
    return requestJson<StartedOAuthFlow>("/api/admin/antigravity/oauth/start", {
      method: "POST",
    });
  },

  pollAntigravityOAuth: async (state: string): Promise<OAuthSessionPollStatus> => {
    return requestJson<OAuthSessionPollStatus>(
      `/api/admin/antigravity/oauth/poll/${encodeURIComponent(state)}`,
    );
  },

  completeAntigravityOAuth: async (payload: {
    callback_url: string;
    redirect_uri?: string;
  }): Promise<{ accounts: AntigravityAccountView[] }> => {
    return requestJson("/api/admin/antigravity/oauth/complete", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  importAntigravityAccount: async (payload: {
    name?: string;
    email?: string;
    refresh_token: string;
    project_id?: string;
  }): Promise<{ accounts: AntigravityAccountView[] }> => {
    return requestJson("/api/admin/antigravity/accounts/import", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  refreshAntigravityQuotas: async (): Promise<{
    refreshed: number;
    accounts: AntigravityAccountView[];
  }> => {
    return requestJson("/api/admin/antigravity/quotas/refresh", {
      method: "POST",
    });
  },

  refreshSingleAntigravityQuota: async (
    id: string,
  ): Promise<{ quota: AntigravityQuotaSnapshot; accounts: AntigravityAccountView[] }> => {
    return requestJson(`/api/admin/antigravity/accounts/${encodeURIComponent(id)}/refresh-quota`, {
      method: "POST",
    });
  },

  updateConnection: async (
    id: string,
    payload: { name?: string; enabled?: boolean; reset_cooldown?: boolean },
  ): Promise<void> => {
    await requestJson(`/api/admin/connections/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(payload),
    });
  },

  deleteConnection: async (id: string): Promise<void> => {
    await requestJson(`/api/admin/connections/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  },

  getUsage: async (period: UsagePeriod = "today", limit = 50): Promise<UsageSummary> => {
    return requestJson<UsageSummary>(
      `/api/admin/usage?period=${encodeURIComponent(period)}&limit=${limit}`,
    );
  },

  getLogs: async (params: LogQueryParams = {}): Promise<UsageLogRecord[]> => {
    const sp = new URLSearchParams();
    if (params.limit) sp.set("limit", String(params.limit));
    if (params.status) sp.set("status", params.status);
    if (params.proxy) sp.set("proxy", params.proxy);
    if (params.model) sp.set("model", params.model);
    if (params.q) sp.set("q", params.q);
    const qs = sp.toString();
    const res = await requestJson<{ logs: UsageLogRecord[] }>(
      `/api/admin/logs${qs ? `?${qs}` : ""}`,
    );
    return res.logs || [];
  },

  clearLogs: async (): Promise<{ deleted: number }> => {
    return requestJson<{ deleted: number }>("/api/admin/logs", { method: "DELETE" });
  },

  getModels: async (): Promise<OpenAiModelEntry[]> => {
    const res = await requestJson<{ data: OpenAiModelEntry[] }>("/api/admin/models");
    return res.data || [];
  },

  getKeys: async (): Promise<ApiKeyView[]> => {
    const res = await requestJson<{ keys: ApiKeyView[] }>("/api/admin/keys");
    return res.keys || [];
  },

  createKey: async (payload: CreateApiKeyPayload): Promise<CreatedApiKeyResponse> => {
    return requestJson<CreatedApiKeyResponse>("/api/admin/keys", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  updateKey: async (id: string, payload: UpdateApiKeyPayload): Promise<ApiKeyView> => {
    const res = await requestJson<{ key: ApiKeyView }>(
      `/api/admin/keys/${encodeURIComponent(id)}`,
      {
        method: "PATCH",
        body: JSON.stringify(payload),
      },
    );
    return res.key;
  },

  deleteKey: async (id: string): Promise<void> => {
    await requestJson(`/api/admin/keys/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  },

  getSettings: async (): Promise<GatewaySettings> => {
    const res = await requestJson<{ settings: GatewaySettings }>("/api/admin/settings");
    return res.settings;
  },

  updateSettings: async (payload: Partial<GatewaySettings>): Promise<GatewaySettings> => {
    const res = await requestJson<{ settings: GatewaySettings }>("/api/admin/settings", {
      method: "PUT",
      body: JSON.stringify(payload),
    });
    return res.settings;
  },

  getProviders: async (): Promise<ProviderCatalogItem[]> => {
    const res = await requestJson<{ providers: ProviderCatalogItem[] }>("/api/admin/providers");
    return res.providers || [];
  },

  updateProviderConfig: async (
    id: string,
    payload: UpdateProviderConfigPayload,
  ): Promise<ProviderConfig> => {
    const res = await requestJson<{ config: ProviderConfig }>(
      `/api/admin/providers/${encodeURIComponent(id)}/config`,
      {
        method: "PUT",
        body: JSON.stringify(payload),
      },
    );
    return res.config;
  },

  testProvider: async (id: string): Promise<ProviderTestResult> => {
    return requestJson<ProviderTestResult>(
      `/api/admin/providers/${encodeURIComponent(id)}/test`,
      { method: "POST" },
    );
  },

  getCombos: async (): Promise<ComboRecord[]> => {
    const res = await requestJson<{ combos: ComboRecord[] }>("/api/admin/combos");
    return res.combos || [];
  },

  createCombo: async (payload: CreateComboPayload): Promise<ComboRecord> => {
    const res = await requestJson<{ combo: ComboRecord }>("/api/admin/combos", {
      method: "POST",
      body: JSON.stringify(payload),
    });
    return res.combo;
  },

  updateCombo: async (id: string, payload: UpdateComboPayload): Promise<ComboRecord> => {
    const res = await requestJson<{ combo: ComboRecord }>(
      `/api/admin/combos/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        body: JSON.stringify(payload),
      },
    );
    return res.combo;
  },

  deleteCombo: async (id: string): Promise<void> => {
    await requestJson(`/api/admin/combos/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  },

  getCliToolsStatus: async (): Promise<CliToolsStatusMap> => {
    return requestJson<CliToolsStatusMap>("/api/admin/cli-tools/status");
  },

  applyOpenCodeCli: async (
    payload: ApplyOpenCodeCliPayload,
  ): Promise<{ success: boolean; status: OpenCodeCliStatus }> => {
    return requestJson("/api/admin/cli-tools/opencode", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  resetOpenCodeCli: async (
    model?: string,
  ): Promise<{ success: boolean; status: OpenCodeCliStatus }> => {
    const qs = model ? `?model=${encodeURIComponent(model)}` : "";
    return requestJson(`/api/admin/cli-tools/opencode${qs}`, { method: "DELETE" });
  },

  applyClaudeCli: async (
    payload: ApplyClaudeCliPayload,
  ): Promise<{ success: boolean; status: ClaudeCliStatus }> => {
    return requestJson("/api/admin/cli-tools/claude", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  resetClaudeCli: async (): Promise<{ success: boolean; status: ClaudeCliStatus }> => {
    return requestJson("/api/admin/cli-tools/claude", { method: "DELETE" });
  },

  applyCodexCli: async (
    payload: ApplyCodexCliPayload,
  ): Promise<{ success: boolean; status: CodexCliStatus }> => {
    return requestJson("/api/admin/cli-tools/codex", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  resetCodexCli: async (): Promise<{ success: boolean; status: CodexCliStatus }> => {
    return requestJson("/api/admin/cli-tools/codex", { method: "DELETE" });
  },

  getTokenSaver: async (): Promise<TokenSaverStateResponse> => {
    return requestJson<TokenSaverStateResponse>("/api/admin/token-saver");
  },

  updateTokenSaver: async (
    payload: Partial<
      Omit<TokenSaverConfig, "rtk_categories"> & {
        rtk_categories?: Partial<TokenSaverConfig["rtk_categories"]>;
      }
    >,
  ): Promise<TokenSaverStateResponse> => {
    return requestJson<TokenSaverStateResponse>("/api/admin/token-saver", {
      method: "PUT",
      body: JSON.stringify(payload),
    });
  },

  testTokenSaver: async (payload: {
    text: string;
    tool_name?: string;
    command?: string;
    config?: TokenSaverConfig;
  }): Promise<TokenSaverTestResponse> => {
    return requestJson<TokenSaverTestResponse>("/api/admin/token-saver/test", {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },

  clearTokenSaverStats: async (): Promise<{ deleted: number }> => {
    return requestJson<{ deleted: number }>("/api/admin/token-saver/stats", {
      method: "DELETE",
    });
  },

  getBridgeConfig: async (): Promise<{ config: MultimodalBridgeConfig }> => {
    return requestJson<{ config: MultimodalBridgeConfig }>("/api/admin/bridge");
  },

  updateBridgeConfig: async (
    payload: UpdateBridgePayload,
  ): Promise<{ config: MultimodalBridgeConfig }> => {
    return requestJson<{ config: MultimodalBridgeConfig }>("/api/admin/bridge", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
  },

  getBridgeStats: async (): Promise<{ stats: BridgeStats }> => {
    return requestJson<{ stats: BridgeStats }>("/api/admin/bridge/stats");
  },

  simulateBridgeEvent: async (
    payload: Partial<BridgeEvent>,
  ): Promise<{ ok: boolean; event: BridgeEvent }> => {
    return requestJson<{ ok: boolean; event: BridgeEvent }>("/api/admin/bridge/simulate-event", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
  },

  getMitmConfig: async (): Promise<{ config: AntigravityMitmConfig }> => {
    return requestJson<{ config: AntigravityMitmConfig }>("/api/admin/mitm/config");
  },

  updateMitmConfig: async (
    payload: UpdateMitmConfigPayload,
  ): Promise<{ config: AntigravityMitmConfig }> => {
    return requestJson<{ config: AntigravityMitmConfig }>("/api/admin/mitm/config", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
  },

  getAuthStatus: async (): Promise<AuthStatusResponse> => {
    return requestJson<AuthStatusResponse>("/api/admin/auth/status");
  },

  listCustomProviders: async (): Promise<CustomProvider[]> => {
    const res = await requestJson<{ custom_providers: CustomProvider[] }>(
      "/api/admin/custom-providers",
    );
    return res.custom_providers || [];
  },

  discoverCustomProviderFavicon: async (
    baseUrl: string,
  ): Promise<{ base_url: string; favicon_url: string | null }> => {
    return requestJson<{ base_url: string; favicon_url: string | null }>(
      `/api/admin/custom-providers/favicon?base_url=${encodeURIComponent(baseUrl)}`,
    );
  },

  createCustomProvider: async (
    payload: CreateCustomProviderPayload,
  ): Promise<CustomProvider> => {
    const res = await requestJson<{ custom_provider: CustomProvider }>(
      "/api/admin/custom-providers",
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
    return res.custom_provider;
  },

  updateCustomProvider: async (
    id: string,
    payload: UpdateCustomProviderPayload,
  ): Promise<CustomProvider> => {
    const res = await requestJson<{ custom_provider: CustomProvider }>(
      `/api/admin/custom-providers/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        body: JSON.stringify(payload),
      },
    );
    return res.custom_provider;
  },

  deleteCustomProvider: async (id: string): Promise<void> => {
    await requestJson(`/api/admin/custom-providers/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
  },

  refreshCustomProviderFavicon: async (
    id: string,
  ): Promise<{ custom_provider: CustomProvider; favicon_url: string | null }> => {
    return requestJson(
      `/api/admin/custom-providers/${encodeURIComponent(id)}/refresh-favicon`,
      { method: "POST" },
    );
  },

  fetchCustomProviderModelsPreview: async (payload: {
    base_url: string;
    protocol: string;
    api_type?: string;
    api_keys?: string[];
    custom_headers?: Record<string, string>;
    proxy_mode?: string;
  }): Promise<string[]> => {
    const res = await requestJson<{ models: string[] }>(
      "/api/admin/custom-providers/fetch-models",
      {
        method: "POST",
        body: JSON.stringify(payload),
      },
    );
    return res.models || [];
  },

  fetchCustomProviderModels: async (
    id: string,
    save = false,
  ): Promise<{ models: string[]; custom_provider: CustomProvider }> => {
    return requestJson(
      `/api/admin/custom-providers/${encodeURIComponent(id)}/fetch-models`,
      {
        method: "POST",
        body: JSON.stringify({ save }),
      },
    );
  },

  fetchProviderModels: async (
    id: string,
  ): Promise<{ status: string; provider: string; name: string; count: number; models: unknown[] }> => {
    return requestJson(
      `/api/admin/providers/${encodeURIComponent(id)}/fetch-models`,
      {
        method: "POST",
      },
    );
  },

  getFullSettings: async (): Promise<SettingsFullResponse> => {
    return requestJson<SettingsFullResponse>("/api/admin/settings");
  },

  updateFullSettings: async (
    payload: UpdateSettingsExtendedPayload,
  ): Promise<SettingsFullResponse> => {
    return requestJson<SettingsFullResponse>("/api/admin/settings", {
      method: "PUT",
      body: JSON.stringify(payload),
    });
  },

  checkUpdate: async (): Promise<UpdateCheckResponse> => {
    return requestJson<UpdateCheckResponse>("/api/admin/update/check");
  },

  applyUpdate: async (): Promise<UpdateApplyResponse> => {
    return requestJson<UpdateApplyResponse>("/api/admin/update/apply", {
      method: "POST",
    });
  },

  exportBackup: async (includeSecrets = true): Promise<Record<string, unknown>> => {
    return requestJson<Record<string, unknown>>(
      `/api/admin/backup/export?include_secrets=${includeSecrets ? "true" : "false"}`,
    );
  },

  importBackup: async (
    backup: Record<string, unknown>,
    replace = false,
  ): Promise<BackupImportResponse> => {
    return requestJson<BackupImportResponse>(
      `/api/admin/backup/import?replace=${replace ? "true" : "false"}`,
      {
        method: "POST",
        body: JSON.stringify({ backup, replace }),
      },
    );
  },

  resetAllCooldownsMaintenance: async (): Promise<ResetCooldownsResponse> => {
    return requestJson<ResetCooldownsResponse>("/api/admin/maintenance/reset-cooldowns", {
      method: "POST",
    });
  },
};

export interface PlaygroundChatResult {
  content: string;
  reasoning: string;
  proxyUsed: string | null;
  egressIp: string | null;
  resolvedModel: string | null;
  comboUsed: string | null;
  ttftMs: number | null;
  latencyMs: number;
  completionTokens: number;
  promptTokens: number;
  cachedTokens: number;
  totalTokens: number;
  tokensPerSec: number | null;
}

export interface StreamProgress {
  content: string;
  reasoning: string;
  proxyUsed: string | null;
  egressIp: string | null;
  resolvedModel: string | null;
  comboUsed: string | null;
  ttftMs: number | null;
  elapsedMs: number;
  liveTps: number | null;
  approxTokens: number;
}

export async function runPlaygroundChat(opts: {
  model: string;
  messages: Array<{ role: "system" | "user" | "assistant"; content: string }>;
  stream: boolean;
  apiKey?: string;
  signal?: AbortSignal;
  onProgress?: (p: StreamProgress) => void;
}): Promise<PlaygroundChatResult> {
  const startedAt = performance.now();
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (opts.apiKey?.trim()) {
    headers.Authorization = `Bearer ${opts.apiKey.trim()}`;
  }

  const resp = await fetch("/v1/chat/completions", {
    method: "POST",
    headers,
    signal: opts.signal,
    body: JSON.stringify({
      model: opts.model,
      stream: opts.stream,
      messages: opts.messages,
    }),
  });

  const proxyUsed =
    resp.headers.get("x-tesie-proxy") ?? resp.headers.get("x-tesroute-proxy");
  const egressIp =
    resp.headers.get("x-tesie-egress-ip") ?? resp.headers.get("x-tesroute-egress-ip");
  const resolvedModel =
    resp.headers.get("x-tesie-model") ?? resp.headers.get("x-tesroute-model");
  const comboUsed =
    resp.headers.get("x-tesie-combo") ?? resp.headers.get("x-tesroute-combo");

  if (!resp.ok) {
    const raw = await resp.text();
    let msg = raw;
    try {
      const j = JSON.parse(raw) as { error?: { message?: string } | string };
      if (typeof j.error === "string") msg = j.error;
      else if (j.error?.message) msg = j.error.message;
    } catch {
      // keep raw
    }
    throw new ApiError(resp.status, msg || `HTTP ${resp.status}`);
  }

  if (!opts.stream || !resp.body) {
    const data = (await resp.json()) as {
      choices?: Array<{ message?: { content?: string; reasoning_content?: string } }>;
      usage?: {
        prompt_tokens?: number;
        completion_tokens?: number;
        total_tokens?: number;
        prompt_tokens_details?: { cached_tokens?: number };
      };
    };
    const latencyMs = Math.max(1, Math.round(performance.now() - startedAt));
    const content = data.choices?.[0]?.message?.content || "";
    const reasoning = data.choices?.[0]?.message?.reasoning_content || "";
    const promptTokens = data.usage?.prompt_tokens ?? 0;
    const cachedTokens = data.usage?.prompt_tokens_details?.cached_tokens ?? 0;
    const rawComp = data.usage?.completion_tokens ?? 0;
    const estComp =
      rawComp > 0 ? rawComp : Math.max(1, Math.ceil((content.length + reasoning.length) / 4));
    const totalTokens = data.usage?.total_tokens || promptTokens + estComp;
    const tps = Number(((estComp / Math.max(50, latencyMs)) * 1000).toFixed(2));
    return {
      content,
      reasoning,
      proxyUsed,
      egressIp,
      resolvedModel,
      comboUsed,
      ttftMs: latencyMs,
      latencyMs,
      completionTokens: estComp,
      promptTokens,
      cachedTokens,
      totalTokens,
      tokensPerSec: tps,
    };
  }

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let content = "";
  let reasoning = "";
  let firstChunkAt: number | null = null;
  let promptTokens = 0;
  let cachedTokens = 0;
  let completionTokens = 0;
  let totalTokens = 0;
  let totalDeltaChars = 0;

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    let sepIdx: number;
    while ((sepIdx = buffer.indexOf("\n\n")) !== -1) {
      const rawBlock = buffer.slice(0, sepIdx);
      buffer = buffer.slice(sepIdx + 2);
      const dataLines = rawBlock
        .split(/\r?\n/)
        .filter((l) => l.startsWith("data:"))
        .map((l) => l.slice(5).trimStart());
      if (dataLines.length === 0) continue;
      const payloadStr = dataLines.join("\n").trim();
      if (!payloadStr || payloadStr === "[DONE]") continue;

      let parsed: Record<string, unknown>;
      try {
        parsed = JSON.parse(payloadStr) as Record<string, unknown>;
      } catch {
        continue;
      }

      const usage = parsed.usage as
        | {
            prompt_tokens?: number;
            completion_tokens?: number;
            total_tokens?: number;
            prompt_tokens_details?: { cached_tokens?: number };
          }
        | undefined;
      if (usage) {
        if (typeof usage.prompt_tokens === "number") promptTokens = usage.prompt_tokens;
        if (typeof usage.prompt_tokens_details?.cached_tokens === "number") {
          cachedTokens = usage.prompt_tokens_details.cached_tokens;
        }
        if (typeof usage.completion_tokens === "number") completionTokens = usage.completion_tokens;
        if (typeof usage.total_tokens === "number") totalTokens = usage.total_tokens;
      }

      const choices = parsed.choices as
        | Array<{ delta?: { content?: string; reasoning_content?: string; reasoning?: string } }>
        | undefined;
      const delta = choices?.[0]?.delta;
      if (delta) {
        if (typeof delta.content === "string" && delta.content.length > 0) {
          if (firstChunkAt === null) firstChunkAt = performance.now();
          content += delta.content;
          totalDeltaChars += delta.content.length;
        }
        const r = delta.reasoning_content || delta.reasoning;
        if (typeof r === "string" && r.length > 0) {
          if (firstChunkAt === null) firstChunkAt = performance.now();
          reasoning += r;
          totalDeltaChars += r.length;
        }
      }

      const nowPerf = performance.now();
      const approxTokens =
        completionTokens > 0 ? completionTokens : Math.max(0, Math.ceil(totalDeltaChars / 4));
      const ttftMs = firstChunkAt !== null ? Math.max(1, Math.round(firstChunkAt - startedAt)) : null;
      const elapsedMs = Math.max(1, Math.round(nowPerf - startedAt));
      const decodeMs = firstChunkAt !== null ? Math.max(50, nowPerf - firstChunkAt) : elapsedMs;
      const liveTps =
        approxTokens > 0 ? Number(((approxTokens / decodeMs) * 1000).toFixed(1)) : null;

      opts.onProgress?.({
        content,
        reasoning,
        proxyUsed,
        egressIp,
        resolvedModel,
        comboUsed,
        ttftMs,
        elapsedMs,
        liveTps,
        approxTokens,
      });
    }
  }

  const endAt = performance.now();
  const latencyMs = Math.max(1, Math.round(endAt - startedAt));
  const ttftMs = firstChunkAt !== null ? Math.max(1, Math.round(firstChunkAt - startedAt)) : null;
  const finalComp =
    completionTokens > 0
      ? completionTokens
      : totalDeltaChars > 0
        ? Math.max(1, Math.ceil(totalDeltaChars / 4))
        : 0;
  const finalTotal = Math.max(totalTokens, promptTokens + finalComp);
  const decodeMs =
    firstChunkAt !== null && endAt - firstChunkAt >= 100
      ? endAt - firstChunkAt
      : Math.max(50, latencyMs);
  const tokensPerSec =
    finalComp > 0 ? Number(((finalComp / decodeMs) * 1000).toFixed(2)) : null;

  return {
    content,
    reasoning,
    proxyUsed,
    egressIp,
    resolvedModel,
    comboUsed,
    ttftMs,
    latencyMs,
    completionTokens: finalComp,
    promptTokens,
    cachedTokens,
    totalTokens: finalTotal,
    tokensPerSec,
  };
}
