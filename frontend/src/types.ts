export type IpQuotaStatus = "active" | "cooldown";
export type UsagePeriod = "today" | "24h" | "7d" | "30d" | "all";
export type QuotaStrategy = "ip_pool" | "account";
export type ThinkingLevel =
  | "auto"
  | "none"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max";

export interface MihomoHealth {
  status: string;
  ja4_cipher_hash: string;
  proxies_loaded: number;
}

export interface StatusResponse {
  status: string;
  app_name?: string;
  core_name?: string;
  core_healthy?: boolean;
  mihomo_healthy: boolean;
  mihomo: MihomoHealth | null;
  data_dir?: string;
  auto_start_enabled?: boolean;
  total_proxies: number;
  active_ips: number;
  cooling_ips: number;
}

export interface ProxyWithQuota {
  id: string;
  name: string;
  proxy_type: string;
  ip: string | null;
  host?: string | null;
  port?: number | null;
  username?: string | null;
  password?: string | null;
  latency_ms: number | null;
  alive: boolean;
  proxy_url: string | null;
  raw_outbound_json: string | null;
  is_direct: boolean;
  enabled: boolean;
  egress_ip: string | null;
  last_delay_ms: number | null;
  last_probed_at: number | null;
  consecutive_failures: number;
  created_at: number;
  updated_at: number;
  ip_key: string;
  quota_status: IpQuotaStatus | null;
  window_started_at: number | null;
  window_duration_ms: number;
  requests_in_window: number;
  max_requests_per_window: number | null;
  tokens_in_window: number;
  max_tokens_per_window: number | null;
  total_requests: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_rate_limits: number;
  cooldown_until: number | null;
  backoff_level: number;
  canonical_session_id: string | null;
  last_error: string | null;
  last_used_at: number | null;
}

export interface ProxiesListResponse {
  proxies: ProxyWithQuota[];
  test_interval_sec: number;
  last_auto_test_at: number | null;
}

export interface IpQuotaWithProxies {
  ip_key: string;
  provider_id: string;
  window_started_at: number;
  window_duration_ms: number;
  requests_in_window: number;
  max_requests_per_window: number | null;
  tokens_in_window: number;
  max_tokens_per_window: number | null;
  total_requests: number;
  total_prompt_tokens: number;
  total_completion_tokens: number;
  total_rate_limits: number;
  status: IpQuotaStatus;
  cooldown_until: number | null;
  backoff_level: number;
  canonical_session_id: string | null;
  session_created_at: number | null;
  last_error: string | null;
  last_used_at: number | null;
  last_quota_checked_at?: number | null;
  updated_at: number;
  proxies: string[];
}

export interface QuotaProbeStatus {
  enabled?: boolean;
  interval_sec: number;
  probe_running: boolean;
  last_probe_at: number | null;
  last_checked: number;
  last_ready: number;
  last_cooling: number;
  last_unhealthy: number;
}

export interface QuotaProbeSummary {
  checked: number;
  ready: number;
  cooling: number;
  unhealthy: number;
  checked_at: number;
}

export interface VerifiedQuotaResetResult {
  ip_key: string;
  reset: boolean;
  available: boolean;
  message: string;
  quota?: IpQuotaWithProxies | null;
}

export interface VerifiedBatchResetResult {
  checked: number;
  reset: number;
  still_cooling: number;
  unhealthy: number;
}

export interface AssignedProxyBrief {
  id: string;
  name: string;
  proxy_type: string;
  ip: string | null;
  alive: boolean;
  latency_ms: number | null;
  is_direct: boolean;
}

export interface AccountProxyPartition {
  connection_id: string;
  account_name: string;
  mode: "fallback" | "direct" | string;
  shared_warning: boolean;
  primary_proxy: AssignedProxyBrief | null;
  assigned_proxies: AssignedProxyBrief[];
}

export interface AntigravityPoolWindow {
  key: string;
  group_name: string;
  window_label: string;
  remaining_percentage: number;
  reset_at: number | null;
  unlocked: boolean;
}

export interface AntigravityModelQuotaEntry {
  model: string;
  display_name?: string;
  family?: string;
  unlocked?: boolean;
  remaining_percentage: number;
  reset_at: number | null;
  weekly_remaining_percentage?: number | null;
  weekly_reset_at?: number | null;
  session_remaining_percentage?: number | null;
  session_reset_at?: number | null;
  failure_count?: number;
  failure_cooldown_until?: number | null;
  failure_reason?: string | null;
}

export interface AntigravityModelFailureState {
  model: string;
  display_name: string;
  consecutive_failures: number;
  cooldown_until: number;
  last_error: string | null;
  updated_at: number;
}

export interface AntigravityQuotaSnapshot {
  plan?: string | null;
  is_free_tier?: boolean;
  pools?: AntigravityPoolWindow[];
  models: AntigravityModelQuotaEntry[];
  fetched_at: number;
}

export interface AntigravityAccountView {
  id: string;
  name: string;
  email: string | null;
  enabled: boolean;
  status: "ready" | "cooldown" | "disabled" | "banned" | string;
  cooldown_until: number | null;
  last_error: string | null;
  created_at: number;
  updated_at: number;
  quota: AntigravityQuotaSnapshot | null;
  partition: AccountProxyPartition | null;
  model_cooldowns?: AntigravityModelFailureState[];
}

export interface StartedOAuthFlow {
  state: string;
  auth_url: string;
  redirect_uri: string;
}

export interface OAuthSessionPollStatus {
  state: string;
  status: "pending" | "completed" | "error" | string;
  connection_id: string | null;
  email: string | null;
  error: string | null;
}

export interface MihomoProxyState {
  name: string;
  proxy_type: string;
  alive: boolean;
  delay_ms: number | null;
}

export interface AttemptLeg {
  attempt: number;
  proxy: string;
  egress_ip?: string | null;
  status: number;
  latency_ms: number;
  phase: string;
  session_id?: string | null;
  detail?: string | null;
  account_id?: string;
  account_email?: string | null;
  account_name?: string;
  model?: string;
  wire_model?: string;
  reasoning_level?: string | null;
  cooldown_applied?: string | null;
  failure_count?: number | null;
}

export interface UsageLogRecord {
  id: number;
  request_id: string;
  api_key_id: string | null;
  provider_id: string;
  model: string;
  proxy_name: string;
  egress_ip: string | null;
  status_code: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  latency_ms: number;
  ttft_ms: number | null;
  tokens_per_sec: number | null;
  error_kind: string | null;
  error_detail: string | null;
  upstream_url?: string | null;
  attempt_count?: number;
  attempt_trace?: string | null;
  session_id?: string | null;
  wire_format?: string | null;
  created_at: number;
}

export interface ProxyUsageAggregate {
  proxy_name: string;
  last_egress_ip: string | null;
  total_requests: number;
  success_requests: number;
  rate_limited_requests: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  avg_tokens_per_sec: number;
  peak_tokens_per_sec: number;
  avg_ttft_ms: number;
  avg_latency_ms: number;
  last_used_at: number | null;
}

export interface IpUsageAggregate {
  egress_ip: string;
  total_requests: number;
  success_requests: number;
  total_tokens: number;
  rate_limited_requests: number;
  avg_tokens_per_sec: number;
  avg_ttft_ms: number;
  avg_latency_ms: number;
}

export interface ModelUsageAggregate {
  model: string;
  provider_id: string;
  total_requests: number;
  success_requests: number;
  rate_limited_requests: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  avg_tokens_per_sec: number;
  peak_tokens_per_sec: number;
  avg_ttft_ms: number;
  avg_latency_ms: number;
  last_used_at: number | null;
}

export interface ProviderUsageAggregate {
  provider_id: string;
  total_requests: number;
  success_requests: number;
  rate_limited_requests: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  avg_tokens_per_sec: number;
  avg_ttft_ms: number;
  last_used_at: number | null;
}

export interface ApiKeyUsageAggregate {
  key_name: string;
  key_prefix: string | null;
  total_requests: number;
  success_requests: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  last_used_at: number | null;
}

export interface UsageTimeBucket {
  timestamp: number;
  label: string;
  requests: number;
  success_requests: number;
  rate_limited_requests: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  avg_tokens_per_sec: number;
  avg_ttft_ms: number;
}

export interface UsageSummary {
  period: string;
  total_requests: number;
  success_requests: number;
  rate_limited_requests: number;
  error_requests: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  avg_tokens_per_sec: number;
  peak_tokens_per_sec: number;
  avg_ttft_ms: number;
  avg_latency_ms: number;
  timeseries: UsageTimeBucket[];
  by_provider: ProviderUsageAggregate[];
  by_api_key: ApiKeyUsageAggregate[];
  by_proxy: ProxyUsageAggregate[];
  by_ip: IpUsageAggregate[];
  by_model: ModelUsageAggregate[];
  recent: UsageLogRecord[];
}

export interface ActiveRequestInfo {
  request_id: string;
  provider_id: string;
  model: string;
  proxy_name: string;
  egress_ip: string | null;
  started_at: number;
}

export interface TopologyProviderNode {
  id: string;
  name: string;
  color?: string;
  logo?: string;
  proxy_mode?: string;
  enabled: boolean;
  quota_strategy: QuotaStrategy;
  selection_strategy?: string;
  active_units: number;
  cooling_units: number;
  total_requests: number;
  total_tokens: number;
  avg_tokens_per_sec: number;
}

export interface TopologyResponse {
  mihomo_healthy: boolean;
  mihomo: MihomoHealth | null;
  active_requests: ActiveRequestInfo[];
  last_provider: string | null;
  error_provider: string | null;
  proxy_summary: {
    total_proxies: number;
    healthy_proxies?: number;
    unhealthy_proxies?: number;
    probed_ips: number;
    active_ips: number;
    cooling_ips: number;
  };
  providers: TopologyProviderNode[];
  combos?: ComboRecord[];
  antigravity_accounts?: AntigravityAccountView[];
}

export type RoutingStrategy = "fallback" | "round-robin" | "random";

export interface ProviderQuotaSummary {
  id: string;
  name: string;
  description: string;
  strategy: QuotaStrategy;
  selection_strategy?: string;
  routing_strategy?: RoutingStrategy | string;
  unit_label: string;
  active_count: number;
  cooling_count: number;
  probed_count?: number;
  quotas_count?: number;
  proxy_mode?: string;
  window_duration_ms?: number;
  default_max_requests?: number | null;
  default_max_tokens?: number | null;
  probe_enabled?: boolean;
  probe_interval_sec?: number;
  probe_status?: QuotaProbeStatus | null;
  refresh_interval_sec?: number;
  refreshed_at?: number;
}

export interface ProviderConfig {
  provider_id: string;
  enabled: boolean;
  proxy_mode: "all" | "selected" | "direct" | "pool_strict" | "pool_with_direct" | string;
  selected_proxy_ids: string[];
  window_duration_ms: number;
  max_requests_per_window: number | null;
  max_tokens_per_window: number | null;
  session_ttl_ms: number;
  thinking_level?: ThinkingLevel | string;
  probe_enabled?: boolean;
  probe_interval_sec?: number;
  routing_strategy?: RoutingStrategy | string;
  updated_at: number;
}

export interface UpdateProviderConfigPayload {
  enabled?: boolean;
  proxy_mode?: string;
  selected_proxy_ids?: string[];
  window_duration_ms?: number;
  max_requests_per_window?: number | null;
  max_tokens_per_window?: number | null;
  session_ttl_ms?: number;
  thinking_level?: string;
  probe_enabled?: boolean;
  probe_interval_sec?: number;
  routing_strategy?: RoutingStrategy | string;
}

export interface CustomProvider {
  id: string;
  name: string;
  prefix: string;
  protocol: "openai" | "anthropic" | string;
  api_type: "chat" | "responses" | string;
  base_url: string;
  api_keys: string[];
  models: string[];
  custom_headers: Record<string, string>;
  favicon_url?: string | null;
  color: string;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface CreateCustomProviderPayload {
  name: string;
  prefix: string;
  protocol: "openai" | "anthropic" | string;
  api_type?: "chat" | "responses" | string;
  base_url: string;
  api_keys?: string[];
  models?: string[];
  custom_headers?: Record<string, string>;
  favicon_url?: string | null;
  color?: string;
  enabled?: boolean;
}

export interface UpdateCustomProviderPayload {
  name?: string;
  prefix?: string;
  protocol?: "openai" | "anthropic" | string;
  api_type?: "chat" | "responses" | string;
  base_url?: string;
  api_keys?: string[];
  models?: string[];
  custom_headers?: Record<string, string>;
  favicon_url?: string | null;
  color?: string;
  enabled?: boolean;
}

export interface ProviderCatalogItem {
  id: string;
  name: string;
  prefix: string;
  category: "free" | "oauth" | "custom" | string;
  is_custom?: boolean;
  auth_type: "none" | "oauth" | "api_key" | string;
  protocol?: string;
  api_type?: string;
  base_url?: string;
  logo?: string | null;
  favicon_url?: string | null;
  color: string;
  description: string;
  config: ProviderConfig;
  selection_strategy: RoutingStrategy | string;
  routing_strategy?: RoutingStrategy | string;
  quota_tracking: "per_ip" | "per_account" | "api_key" | string;
  quota_refresh_interval_sec: number;
  proxy_test_interval_sec: number;
  health_source: "tesie-core" | string;
  healthy_proxies: number;
  unhealthy_proxies: number;
  active_units: number;
  cooling_units: number;
  total_units: number;
  accounts?: AntigravityAccountView[];
  custom_provider?: CustomProvider;
  refreshed_at: number;
  models: string[];
}

export interface ProviderTestResult {
  ok: boolean;
  provider_id: string;
  proxy?: string;
  egress_ip?: string | null;
  latency_ms: number;
  message: string;
}

export type ComboStrategy = "fallback" | "round-robin" | "random";

export interface ComboRecord {
  id: string;
  name: string;
  strategy: ComboStrategy;
  models: string[];
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface CreateComboPayload {
  name: string;
  strategy?: ComboStrategy;
  models: string[];
  enabled?: boolean;
}

export interface UpdateComboPayload {
  name?: string;
  strategy?: ComboStrategy;
  models?: string[];
  enabled?: boolean;
}

export interface OpenCodeCliStatus {
  installed: boolean;
  configured: boolean;
  config_path: string;
  base_url: string | null;
  models: string[];
  active_model: string | null;
  subagent_model: string | null;
}

export interface ClaudeCliStatus {
  installed: boolean;
  configured: boolean;
  config_path: string;
  base_url: string | null;
  default_model: string | null;
  opus_model: string | null;
  sonnet_model: string | null;
  haiku_model: string | null;
  subagent_model: string | null;
}

export interface CodexCliStatus {
  installed: boolean;
  configured: boolean;
  config_path: string;
  base_url: string | null;
  active_model: string | null;
}

export interface CliToolsStatusMap {
  opencode: OpenCodeCliStatus;
  claude: ClaudeCliStatus;
  codex: CodexCliStatus;
}

export interface ApplyOpenCodeCliPayload {
  base_url: string;
  api_key?: string;
  models: string[];
  active_model?: string;
  subagent_model?: string;
}

export interface ApplyClaudeCliPayload {
  base_url: string;
  api_key?: string;
  default_model?: string;
  opus_model?: string;
  sonnet_model?: string;
  haiku_model?: string;
  subagent_model?: string;
}

export interface ApplyCodexCliPayload {
  base_url: string;
  api_key?: string;
  model: string;
  subagent_model?: string;
}

export interface ModelLimitInfo {
  context?: number;
  input?: number;
  output?: number;
}

export interface OpenAiModelEntry {
  id: string;
  object: string;
  created: number;
  owned_by: string;
  display_name?: string;
  supported_levels?: string[];
  default_level?: string | null;
  attachment?: boolean;
  reasoning?: boolean;
  tool_call?: boolean;
  modalities?: string[];
  limit?: ModelLimitInfo;
  context_length?: number;
  max_output_tokens?: number;
}

export interface ApiKeyView {
  id: string;
  name: string;
  key_prefix: string;
  raw_key?: string | null;
  quota_tokens_limit: number | null;
  quota_tokens_used: number;
  quota_requests_limit: number | null;
  quota_requests_used: number;
  enabled: boolean;
  created_at: number;
  last_used_at: number | null;
}

export interface CreatedApiKeyResponse {
  id: string;
  name: string;
  key_prefix: string;
  raw_key: string;
  quota_tokens_limit: number | null;
  quota_requests_limit: number | null;
  created_at: number;
}

export interface GatewaySettings {
  strict_proxy_mode: boolean;
  default_free_ip_window_ms: number;
  default_free_ip_max_requests: number | null;
  default_free_ip_max_tokens: number | null;
  session_ttl_ms: number;
  proxy_test_interval_sec: number;
  exhaustive_failover: boolean;
}

export interface SettingsSystemInfo {
  data_dir: string;
  db_path: string;
  db_size_bytes: number;
  auto_start_enabled: boolean;
  daemon_path: string;
  tray_active: boolean;
  core_healthy: boolean;
  proxy_test_interval_sec: number;
}

export interface SettingsSecurityInfo {
  has_admin_password: boolean;
  env_admin_token_active: boolean;
  protection_enabled: boolean;
}

export interface SettingsFullResponse {
  settings: GatewaySettings;
  system: SettingsSystemInfo;
  security: SettingsSecurityInfo;
}

export interface UpdateSettingsExtendedPayload extends Partial<GatewaySettings> {
  admin_password?: string | null;
  clear_admin_password?: boolean;
}

export interface AuthStatusResponse {
  protected: boolean;
  authenticated: boolean;
  env_token_active: boolean;
  password_set: boolean;
}

export interface BackupImportSummary {
  settings_updated: boolean;
  token_saver_updated: boolean;
  provider_configs: number;
  custom_providers: number;
  combos: number;
  proxies: number;
  connections: number;
  api_keys: number;
}

export interface BackupImportResponse {
  ok: boolean;
  replace: boolean;
  summary: BackupImportSummary;
}

export interface ResetCooldownsResponse {
  ok: boolean;
  cleared_ips: number;
  cleared_connections: number;
  cleared_model_locks: number;
  cleared_model_failures: number;
}

export interface CreateProxyPayload {
  name?: string;
  proxy_type?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  proxy_url?: string;
  raw_outbound_json?: string;
  enabled?: boolean;
}

export interface UpdateProxyPayload {
  name?: string;
  proxy_type?: string;
  host?: string;
  port?: number;
  username?: string | null;
  password?: string | null;
  proxy_url?: string | null;
  raw_outbound_json?: string | null;
  enabled?: boolean;
}

export interface UpdateIpQuotaLimitsPayload {
  window_duration_ms?: number;
  max_requests_per_window?: number | null;
  max_tokens_per_window?: number | null;
}

export interface CreateApiKeyPayload {
  name: string;
  quota_tokens_limit?: number | null;
  quota_requests_limit?: number | null;
}

export interface UpdateApiKeyPayload {
  name?: string;
  enabled?: boolean;
  quota_tokens_limit?: number | null;
  quota_requests_limit?: number | null;
}

export interface LogQueryParams {
  limit?: number;
  status?: string;
  proxy?: string;
  model?: string;
  q?: string;
}

export interface TokenSaverCategories {
  git: boolean;
  build: boolean;
  test: boolean;
  package: boolean;
  docker: boolean;
  system: boolean;
}

export interface TokenSaverConfig {
  rtk_enabled: boolean;
  rtk_intensity: "minimal" | "standard" | "aggressive";
  rtk_max_lines: number;
  rtk_deduplicate: boolean;
  rtk_categories: TokenSaverCategories;
  style_mode: "off" | "caveman" | "ponytail" | "adhd" | "custom";
  style_level: "lite" | "full" | "ultra";
  custom_prompt: string;
}

export interface TokenSaverCategoryStat {
  category: string;
  requests: number;
  saved_tokens: number;
}

export interface TokenSaverEvent {
  id: number;
  created_at: number;
  provider_id: string;
  model: string;
  category: string;
  original_tokens: number;
  compressed_tokens: number;
  saved_tokens: number;
  style_mode: string;
}

export interface TokenSaverStats {
  total_saved_tokens: number;
  total_original_tokens: number;
  compressed_requests: number;
  style_applied_requests: number;
  avg_saved_pct: number;
  by_category: TokenSaverCategoryStat[];
  recent: TokenSaverEvent[];
}

export interface TokenSaverStateResponse {
  config: TokenSaverConfig;
  stats: TokenSaverStats;
}

export interface TokenSaverTestResponse {
  result: {
    original_tokens: number;
    compressed_tokens: number;
    saved_tokens: number;
    saved_pct: number;
    text: string;
    detected_category: string;
    techniques_used: string[];
  };
  style_prompt_preview: string | null;
}
