use std::net::IpAddr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WireFormat {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
}

impl WireFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            WireFormat::OpenAiChat => "openai",
            WireFormat::OpenAiResponses => "openai-responses",
            WireFormat::AnthropicMessages => "claude",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    #[serde(rename = "opencode")]
    OpenCodeFree,
    #[serde(rename = "antigravity")]
    Antigravity,
    #[serde(rename = "ovh")]
    Ovh,
}

impl ProviderKind {
    pub fn id(&self) -> &'static str {
        match self {
            ProviderKind::OpenCodeFree => "opencode",
            ProviderKind::Antigravity => "antigravity",
            ProviderKind::Ovh => "ovh",
        }
    }

    pub fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix.trim().to_ascii_lowercase().as_str() {
            "oc" | "opencode" => Some(ProviderKind::OpenCodeFree),
            "ag" | "antigravity" => Some(ProviderKind::Antigravity),
            "ovh" => Some(ProviderKind::Ovh),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IpQuotaStatus {
    Active,
    Cooldown,
}

impl IpQuotaStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IpQuotaStatus::Active => "active",
            IpQuotaStatus::Cooldown => "cooldown",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cooldown" => IpQuotaStatus::Cooldown,
            _ => IpQuotaStatus::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub proxy_type: String,
    pub ip: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    pub latency_ms: Option<i64>,
    pub alive: bool,
    pub proxy_url: Option<String>,
    pub raw_outbound_json: Option<String>,
    pub is_direct: bool,
    pub enabled: bool,
    pub egress_ip: Option<String>,
    pub last_delay_ms: Option<i64>,
    pub last_probed_at: Option<i64>,
    pub consecutive_failures: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProxyNode {
    pub fn ip_key(&self) -> String {
        if let Some(ip) = self.egress_ip.as_deref().map(str::trim)
            && !ip.is_empty()
        {
            return ip.to_string();
        }
        if let Some(ip) = self.ip.as_deref().map(str::trim)
            && let Ok(parsed) = ip.parse::<IpAddr>()
            && !parsed.is_loopback()
            && !parsed.is_unspecified()
        {
            return ip.to_string();
        }
        format!("unprobed:{}", self.name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpQuota {
    pub ip_key: String,
    pub provider_id: String,
    pub window_started_at: i64,
    pub window_duration_ms: i64,
    pub requests_in_window: i64,
    pub max_requests_per_window: Option<i64>,
    pub tokens_in_window: i64,
    pub max_tokens_per_window: Option<i64>,
    pub total_requests: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_rate_limits: i64,
    pub status: IpQuotaStatus,
    pub cooldown_until: Option<i64>,
    pub backoff_level: i64,
    pub canonical_session_id: Option<String>,
    pub session_created_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_used_at: Option<i64>,
    pub last_quota_checked_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpCandidate {
    pub proxy: ProxyNode,
    pub quota: IpQuota,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConnection {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub auth_data_json: Option<String>,
    #[serde(default)]
    pub quota_json: Option<String>,
    pub proxy_node_id: Option<String>,
    #[serde(default)]
    pub sticky_proxy_id: Option<String>,
    pub enabled: bool,
    pub cooldown_until: Option<i64>,
    pub backoff_level: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityPoolWindow {
    /// Stable pool key: `"gemini_weekly"`, `"gemini_session"`, `"gemini_argon_weekly"`,
    /// `"gemini_argon_session"`, `"claude_gpt_weekly"`, `"claude_gpt_session"`
    pub key: String,
    /// Group heading matching official Antigravity client: `"Gemini Models"`, `"Gemini 4 Argon"`, `"Claude and GPT models"`
    pub group_name: String,
    /// `"Weekly limit"` | `"5h limit"`
    pub window_label: String,
    pub remaining_percentage: f64,
    pub reset_at: Option<i64>,
    #[serde(default = "default_true")]
    pub unlocked: bool,
}

fn default_true() -> bool {
    true
}

fn default_thinking_level() -> String {
    "auto".to_string()
}

fn default_probe_interval_sec() -> i64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityModelQuotaEntry {
    pub model: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub family: String,
    #[serde(default = "default_true")]
    pub unlocked: bool,
    pub remaining_percentage: f64,
    pub reset_at: Option<i64>,
    #[serde(default)]
    pub weekly_remaining_percentage: Option<f64>,
    #[serde(default)]
    pub weekly_reset_at: Option<i64>,
    #[serde(default)]
    pub session_remaining_percentage: Option<f64>,
    #[serde(default)]
    pub session_reset_at: Option<i64>,
    #[serde(default)]
    pub failure_count: u32,
    #[serde(default)]
    pub failure_cooldown_until: Option<i64>,
    #[serde(default)]
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AntigravityQuotaSnapshot {
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub is_free_tier: bool,
    #[serde(default)]
    pub pools: Vec<AntigravityPoolWindow>,
    pub models: Vec<AntigravityModelQuotaEntry>,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub name: String,
    pub key_hash: String,
    pub key_prefix: String,
    pub raw_key: Option<String>,
    pub quota_tokens_limit: Option<i64>,
    pub quota_tokens_used: i64,
    pub quota_requests_limit: Option<i64>,
    pub quota_requests_used: i64,
    pub enabled: bool,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub enabled: bool,
    /// Proxy selection mode across the global TesieCore proxy pool:
    /// - `"all"`: use all external proxies in the global pool (auto-falls back to direct if pool is empty)
    /// - `"selected"`: use only proxies listed in `selected_proxy_ids` (0 selected => direct)
    /// - `"none"` / `"direct"`: direct host egress only (no proxy)
    pub proxy_mode: String,
    #[serde(default)]
    pub selected_proxy_ids: Vec<String>,
    pub window_duration_ms: i64,
    pub max_requests_per_window: Option<i64>,
    pub max_tokens_per_window: Option<i64>,
    pub session_ttl_ms: i64,
    /// Default thinking level when caller does not specify one:
    /// `"auto"` | `"none"` | `"minimal"` | `"low"` | `"medium"` | `"high"` | `"xhigh"` | `"max"`
    #[serde(default = "default_thinking_level")]
    pub thinking_level: String,
    /// Whether background quota probes are enabled for this provider.
    #[serde(default = "default_true")]
    pub probe_enabled: bool,
    /// Background probe interval in seconds (default `30`, clamped `5..=86400`).
    #[serde(default = "default_probe_interval_sec")]
    pub probe_interval_sec: i64,
    /// Multi-IP / multi-account / multi-key routing order (`"fallback"` | `"round-robin"` | `"random"`).
    #[serde(default)]
    pub routing_strategy: String,
    pub updated_at: i64,
}

pub fn default_routing_strategy_for(provider_id: &str) -> &'static str {
    if provider_id.eq_ignore_ascii_case("opencode") || provider_id.eq_ignore_ascii_case("ovh") {
        "round-robin"
    } else {
        "fallback"
    }
}

pub fn normalize_routing_strategy(raw: &str, default_for_provider: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "round-robin" | "round_robin" | "rr" => "round-robin".to_string(),
        "random" | "rand" => "random".to_string(),
        "fallback" | "smart_fallback" | "smart-fallback" => "fallback".to_string(),
        _ => match default_for_provider.trim().to_ascii_lowercase().as_str() {
            "round-robin" | "round_robin" | "rr" => "round-robin".to_string(),
            "random" | "rand" => "random".to_string(),
            _ => "fallback".to_string(),
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub prefix: String,
    /// `"openai"` | `"anthropic"`
    pub protocol: String,
    /// `"chat"` (`/chat/completions`) | `"responses"` (`/responses`) | `"messages"` (`/messages`)
    pub api_type: String,
    pub base_url: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub custom_headers: std::collections::BTreeMap<String, String>,
    pub favicon_url: Option<String>,
    pub color: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComboRecord {
    pub id: String,
    pub name: String,
    /// `"fallback"` | `"round-robin"` | `"random"`
    pub strategy: String,
    pub models: Vec<String>,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageLogInput {
    pub request_id: String,
    pub api_key_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub proxy_name: String,
    pub egress_ip: Option<String>,
    pub status_code: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms: i64,
    pub ttft_ms: Option<i64>,
    pub tokens_per_sec: Option<f64>,
    pub error_kind: Option<String>,
    pub error_detail: Option<String>,
    pub upstream_url: Option<String>,
    pub attempt_count: i64,
    pub attempt_trace: Option<String>,
    pub session_id: Option<String>,
    pub wire_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageLogRecord {
    pub id: i64,
    pub request_id: String,
    pub api_key_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub proxy_name: String,
    pub egress_ip: Option<String>,
    pub status_code: i64,
    pub prompt_tokens: i64,
    pub cached_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub latency_ms: i64,
    pub ttft_ms: Option<i64>,
    pub tokens_per_sec: Option<f64>,
    pub error_kind: Option<String>,
    pub error_detail: Option<String>,
    pub upstream_url: Option<String>,
    pub attempt_count: i64,
    pub attempt_trace: Option<String>,
    pub session_id: Option<String>,
    pub wire_format: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewaySettings {
    pub strict_proxy_mode: bool,
    pub default_free_ip_window_ms: i64,
    pub default_free_ip_max_requests: Option<i64>,
    pub default_free_ip_max_tokens: Option<i64>,
    pub session_ttl_ms: i64,
    pub proxy_test_interval_sec: u64,
    pub exhaustive_failover: bool,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            strict_proxy_mode: false,
            default_free_ip_window_ms: 3_600_000,
            default_free_ip_max_requests: None,
            default_free_ip_max_tokens: None,
            session_ttl_ms: 14_400_000,
            proxy_test_interval_sec: 15,
            exhaustive_failover: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalBridgeConfig {
    pub vision_enabled: bool,
    pub vision_model: Option<String>,
    pub vision_mode: String,
    pub vision_prompt: Option<String>,
    pub audio_enabled: bool,
    pub audio_model: Option<String>,
    pub audio_prompt: Option<String>,
}

impl Default for MultimodalBridgeConfig {
    fn default() -> Self {
        Self {
            vision_enabled: false,
            vision_model: None,
            vision_mode: "standard".to_string(),
            vision_prompt: None,
            audio_enabled: false,
            audio_model: None,
            audio_prompt: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeEvent {
    pub transcriber_provider: String,
    pub target_provider: String,
    pub transcriber_model: String,
    pub target_model: String,
    pub kind: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgeStats {
    pub total_transcriptions: u64,
    pub images_transcribed: u64,
    pub audio_transcribed: u64,
    pub last_event: Option<BridgeEvent>,
}

