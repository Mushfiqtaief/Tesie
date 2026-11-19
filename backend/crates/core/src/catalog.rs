use serde::Serialize;

use crate::models::{ProviderKind, WireFormat};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModel {
    pub raw: String,
    pub provider_hint: Option<ProviderKind>,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogModel {
    pub id: &'static str,
    pub name: &'static str,
    pub provider: ProviderKind,
    pub target_format: WireFormat,
    pub supported_formats: &'static [WireFormat],
}

pub fn strip_thinking_suffix(raw: &str) -> (&str, Option<String>) {
    let trimmed = raw.trim();
    if trimmed.ends_with(')')
        && let Some(open) = trimmed.rfind('(')
    {
        let inner = trimmed[open + 1..trimmed.len() - 1].trim();
        let base = trimmed[..open].trim();
        if !base.is_empty() && !inner.is_empty() && !inner.contains('(') {
            return (base, Some(inner.to_ascii_lowercase()));
        }
    }
    (trimmed, None)
}

pub fn parse_model_spec(raw: &str) -> ParsedModel {
    let (without_effort, reasoning_effort) = strip_thinking_suffix(raw);
    if let Some((prefix, rest)) = without_effort.split_once('/')
        && let Some(provider) = ProviderKind::from_prefix(prefix)
    {
        let model_id = rest.trim();
        if !model_id.is_empty() {
            return ParsedModel {
                raw: raw.trim().to_string(),
                provider_hint: Some(provider),
                model_id: model_id.to_string(),
                reasoning_effort,
            };
        }
    }

    ParsedModel {
        raw: raw.trim().to_string(),
        provider_hint: None,
        model_id: without_effort.to_string(),
        reasoning_effort,
    }
}

pub fn base_model_id(raw: &str) -> &str {
    let (without_effort, _) = strip_thinking_suffix(raw);
    if let Some((prefix, rest)) = without_effort.split_once('/')
        && ProviderKind::from_prefix(prefix).is_some()
        && !rest.trim().is_empty()
    {
        return rest.trim();
    }
    without_effort
}

pub fn is_opencode_free_tier_model(model_id: &str) -> bool {
    let base = base_model_id(model_id).to_ascii_lowercase();
    base.ends_with("-free") || base == "big-pickle" || base == "union-alpha"
}

pub fn is_muse_spark_model(model_id: &str) -> bool {
    let base = base_model_id(model_id).to_ascii_lowercase();
    base == "muse-spark"
        || base.starts_with("muse-spark-")
        || base.starts_with("muse_spark")
        || base.starts_with("musespark")
}

/// Returns `true` if an upstream/catalog Antigravity key is an internal IDE model,
/// disabled placeholder, non-chat image/sonar submodel, or retired model that must
/// never be surfaced in the Provider catalog or Quota Tracker.
pub fn is_excluded_antigravity_model_key(raw_id: &str) -> bool {
    let k = base_model_id(raw_id).trim().to_ascii_lowercase();
    if k.is_empty() {
        return true;
    }
    k.starts_with("tab_")
        || k.starts_with("chat_")
        || k.starts_with("rev")
        || k.starts_with("orchestrator-")
        || k.ends_with("-disabled")
        || k.ends_with("-sonar")
        || k.ends_with("-image")
        || k.contains("_preview")
        || k.starts_with("claude-sonnet-5-5")
        || k.starts_with("claude-sonnet-5.5")
        || k.starts_with("claude-opus-5-5")
        || k.starts_with("claude-opus-5.5")
        || k.starts_with("gemini-3.7-")
        || k.starts_with("gemini-3.6-")
        || k.starts_with("gemini-3.1-pro")
        || k.starts_with("gemini-2.5-pro")
        || k == "gemini-pro-agent"
}

pub const THINKING_LEVEL_NAMES: &[&str] = &[
    "auto", "none", "minimal", "low", "medium", "high", "xhigh", "max",
];

/// Returns `(supported_levels, default_level)` for any catalog model.
/// Uses the exact same canonical level vocabulary as the OpenCode Thinking Level control
/// (`auto`, `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`).
pub fn model_level_capabilities(
    provider: ProviderKind,
    model_id: &str,
    live_reasoning: Option<bool>,
    live_variants: Option<&[String]>,
) -> (Vec<String>, Option<String>) {
    let base = base_model_id(model_id).trim().to_ascii_lowercase();
    match provider {
        ProviderKind::Antigravity => {
            let (canon, _) = canonical_antigravity_model(&base);
            match canon.as_str() {
                "gemini-4-carbon" => (
                    vec!["low".into(), "medium".into(), "high".into()],
                    Some("high".into()),
                ),
                "gemini-4-argon" | "gemini-4-hydrogen" | "gemini-3.8-flash" => (
                    vec!["low".into(), "medium".into(), "high".into()],
                    Some("medium".into()),
                ),
                "gemini-2.5-flash" => (
                    vec!["none".into(), "high".into()],
                    Some("none".into()),
                ),
                _ => (Vec::new(), None),
            }
        }
        ProviderKind::OpenCodeFree => {
            if live_reasoning == Some(false) {
                return (Vec::new(), None);
            }
            if let Some(variants) = live_variants
                && !variants.is_empty()
            {
                let mut levels = vec!["auto".to_string()];
                for v in variants {
                    let norm = v.trim().to_ascii_lowercase();
                    if !norm.is_empty() && !levels.contains(&norm) {
                        levels.push(norm);
                    }
                }
                if levels.len() > 1 {
                    return (levels, Some("auto".to_string()));
                }
            }
            (
                THINKING_LEVEL_NAMES.iter().map(|s| (*s).to_string()).collect(),
                Some("auto".to_string()),
            )
        }
        ProviderKind::Ovh => (Vec::new(), None),
    }
}

/// Human-friendly display name for a model ID across OpenCode Free, Antigravity, and OVHcloud.
pub fn display_name_for_model(provider: ProviderKind, model_id: &str) -> String {
    let base = base_model_id(model_id);
    if provider == ProviderKind::Antigravity {
        let (_, name) = canonical_antigravity_model(base);
        return name;
    }
    if provider == ProviderKind::Ovh {
        for m in OVH_MODELS {
            if m.id.eq_ignore_ascii_case(base) {
                return m.name.to_string();
            }
        }
    }
    for m in OPENCODE_FREE_MODELS {
        if m.id.eq_ignore_ascii_case(base) {
            return m.name.to_string();
        }
    }
    base.to_string()
}

/// Merges same-model Antigravity variants (such as `-high`, `-medium`, `-low`, `-tiered`,
/// `-thinking` on base Gemini/Claude/GPT families) into a single canonical `(model_id, display_name)`.
pub fn canonical_antigravity_model(raw_id: &str) -> (String, String) {
    let clean = base_model_id(raw_id).trim().to_ascii_lowercase();
    if clean == "carbon" || clean.starts_with("gemini-4-carbon") {
        return ("gemini-4-carbon".to_string(), "Gemini 4 Carbon".to_string());
    }
    if clean == "argon" || clean.starts_with("gemini-4-argon") {
        return ("gemini-4-argon".to_string(), "Gemini 4 Argon".to_string());
    }
    if clean == "hydrogen" || clean.starts_with("gemini-4-hydrogen") {
        return (
            "gemini-4-hydrogen".to_string(),
            "Gemini 4 Hydrogen".to_string(),
        );
    }
    if clean.starts_with("orchestrator-carbon") {
        return (
            "orchestrator-carbon".to_string(),
            "Orchestrator (Carbon)".to_string(),
        );
    }
    if clean.starts_with("orchestrator-argon") {
        return (
            "orchestrator-argon".to_string(),
            "Orchestrator (Argon)".to_string(),
        );
    }
    if clean.starts_with("gemini-3.8-flash") {
        return (
            "gemini-3.8-flash".to_string(),
            "Gemini 3.8 Flash".to_string(),
        );
    }
    if clean.starts_with("claude-sonnet-4-6") || clean.starts_with("claude-sonnet-4.6") {
        return (
            "claude-sonnet-4-6".to_string(),
            "Claude Sonnet 4.6".to_string(),
        );
    }
    if clean.starts_with("claude-opus-4-6") || clean.starts_with("claude-opus-4.6") {
        return (
            "claude-opus-4-6-thinking".to_string(),
            "Claude Opus 4.6".to_string(),
        );
    }
    if clean.starts_with("gpt-oss-120b") {
        return (
            "gpt-oss-120b-medium".to_string(),
            "GPT-OSS 120B".to_string(),
        );
    }
    if clean.starts_with("gemini-3.5-flash-lite") {
        return (
            "gemini-3.5-flash-lite".to_string(),
            "Gemini 3.5 Flash Lite".to_string(),
        );
    }
    if clean.starts_with("gemini-3.1-flash-lite") {
        return (
            "gemini-3.1-flash-lite".to_string(),
            "Gemini 3.1 Flash Lite".to_string(),
        );
    }
    if clean.starts_with("gemini-3-flash") {
        return ("gemini-3-flash".to_string(), "Gemini 3 Flash".to_string());
    }
    if clean.starts_with("gemini-2.5-pro") {
        return ("gemini-2.5-pro".to_string(), "Gemini 2.5 Pro".to_string());
    }
    if clean.starts_with("gemini-2.5-flash-lite") {
        return (
            "gemini-2.5-flash-lite".to_string(),
            "Gemini 2.5 Flash Lite".to_string(),
        );
    }
    if clean.starts_with("gemini-2.5-flash") {
        return (
            "gemini-2.5-flash".to_string(),
            "Gemini 2.5 Flash".to_string(),
        );
    }

    // Generic fallback: strip trailing effort/tier suffixes
    let mut base = clean.as_str();
    for suffix in [
        "-high",
        "-medium",
        "-low",
        "-minimal",
        "-xhigh",
        "-max",
        "-tiered",
        "-thinking",
    ] {
        if let Some(stripped) = base.strip_suffix(suffix)
            && !stripped.is_empty()
        {
            base = stripped;
            break;
        }
    }
    let title = base
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (base.to_string(), title)
}

/// Human-friendly short model name used for fail-fast quota errors (e.g. `"No quota for Argon."`).
pub fn antigravity_quota_short_name(raw_id: &str) -> String {
    let clean = base_model_id(raw_id).trim().to_ascii_lowercase();
    if clean == "argon" || clean.starts_with("gemini-4-argon") {
        return "Argon".to_string();
    }
    if clean == "carbon" || clean.starts_with("gemini-4-carbon") {
        return "Carbon".to_string();
    }
    if clean == "hydrogen" || clean.starts_with("gemini-4-hydrogen") {
        return "Hydrogen".to_string();
    }
    let (_, display) = canonical_antigravity_model(&clean);
    display
}

pub fn is_antigravity_model(model_id: &str) -> bool {
    let base = base_model_id(model_id).to_ascii_lowercase();
    if is_opencode_free_tier_model(&base) {
        return false;
    }
    let (canon, _) = canonical_antigravity_model(&base);
    ANTIGRAVITY_MODELS.iter().any(|m| m.id == canon || m.id == base)
        || matches!(base.as_str(), "argon" | "carbon" | "hydrogen")
        || base.starts_with("gemini-4-")
        || base.starts_with("gemini-3.")
        || base.starts_with("gemini-3-")
        || base.starts_with("gemini-2.5-")
        || base == "gemini-pro-agent"
        || base.starts_with("orchestrator-")
        || base.starts_with("claude-sonnet-4")
        || base.starts_with("claude-opus-4")
        || base.starts_with("claude-sonnet-5")
        || base.starts_with("claude-opus-5")
        || base.starts_with("gpt-oss-120b")
}

pub fn is_ovh_model(model_id: &str) -> bool {
    let base = base_model_id(model_id).to_ascii_lowercase();
    OVH_MODELS.iter().any(|m| m.id.to_ascii_lowercase() == base)
}

pub fn resolve_provider_for_model(parsed: &ParsedModel) -> ProviderKind {
    if let Some(p) = parsed.provider_hint {
        return p;
    }
    if is_antigravity_model(&parsed.model_id) {
        return ProviderKind::Antigravity;
    }
    if is_ovh_model(&parsed.model_id) {
        return ProviderKind::Ovh;
    }
    ProviderKind::OpenCodeFree
}

pub fn target_format(provider: ProviderKind, model_id: &str) -> WireFormat {
    if provider == ProviderKind::Antigravity || provider == ProviderKind::Ovh {
        return WireFormat::OpenAiChat;
    }
    let base = base_model_id(model_id).to_ascii_lowercase();

    if is_muse_spark_model(&base) || base.starts_with("gpt-") || base.starts_with("grok-") {
        return WireFormat::OpenAiResponses;
    }

    if base == "union-alpha" || base.starts_with("claude-") {
        return WireFormat::AnthropicMessages;
    }

    WireFormat::OpenAiChat
}

pub fn supported_formats(provider: ProviderKind, model_id: &str) -> &'static [WireFormat] {
    match target_format(provider, model_id) {
        WireFormat::OpenAiResponses => &[WireFormat::OpenAiResponses],
        WireFormat::AnthropicMessages => &[WireFormat::AnthropicMessages],
        WireFormat::OpenAiChat => &[WireFormat::OpenAiChat],
    }
}

pub fn resolve_wire_target(
    provider: ProviderKind,
    model_id: &str,
    source_format: WireFormat,
) -> WireFormat {
    let supported = supported_formats(provider, model_id);
    if supported.contains(&source_format) {
        source_format
    } else {
        target_format(provider, model_id)
    }
}

/// Active verified OpenCode Free models matching `https://models.opencode.ai/api.json` (`status != "deprecated"`).
static OPENCODE_FREE_MODELS: &[CatalogModel] = &[
    CatalogModel {
        id: "deepseek-v4.1-flash-free",
        name: "DeepSeek V4.1 Flash Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "mimo-v2.6-flash-free",
        name: "MiMo V2.6 Flash Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "glm-5.3-flash-free",
        name: "GLM 5.3 Flash Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "big-pickle",
        name: "Big Pickle",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "nemotron-3.5-lightning-free",
        name: "Nemotron 3.5 Lightning Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "nemotron-3.5-super-vl-free",
        name: "Nemotron 3.5 Super VL Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "longcat-2.5-free",
        name: "LongCat 2.5 Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "hy4-flash-free",
        name: "HY4 Flash Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "flint-40b-free",
        name: "Flint 40B Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "beam-free",
        name: "Beam Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "iceman-free",
        name: "Iceman Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "inkling-2-small-free",
        name: "Inkling 2 Small Free",
        provider: ProviderKind::OpenCodeFree,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
];

/// Merged canonical Antigravity models (variants like `-high`, `-medium`, `-low` collapse into one entry).
static ANTIGRAVITY_MODELS: &[CatalogModel] = &[
    CatalogModel {
        id: "gemini-4-carbon",
        name: "Gemini 4 Carbon",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-4-argon",
        name: "Gemini 4 Argon",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-4-hydrogen",
        name: "Gemini 4 Hydrogen",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-3.8-flash",
        name: "Gemini 3.8 Flash",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "claude-sonnet-4-6",
        name: "Claude Sonnet 4.6",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "claude-opus-4-6-thinking",
        name: "Claude Opus 4.6",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gpt-oss-120b-medium",
        name: "GPT-OSS 120B",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-3-flash",
        name: "Gemini 3 Flash",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-3.5-flash-lite",
        name: "Gemini 3.5 Flash Lite",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-3.1-flash-lite",
        name: "Gemini 3.1 Flash Lite",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-2.5-flash",
        name: "Gemini 2.5 Flash",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
    CatalogModel {
        id: "gemini-2.5-flash-lite",
        name: "Gemini 2.5 Flash Lite",
        provider: ProviderKind::Antigravity,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            WireFormat::AnthropicMessages,
        ],
    },
];

pub static OVH_MODELS: &[CatalogModel] = &[
    CatalogModel {
        id: "Meta-Llama-3_3-70B-Instruct",
        name: "Llama 3.3 70B Instruct",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "Qwen2.5-VL-72B-Instruct",
        name: "Qwen 2.5 VL 72B (Vision)",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "qwen3-vl-30b-a3b",
        name: "Qwen 3 VL 30B (Vision)",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "gpt-oss-120b",
        name: "GPT-OSS 120B",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "Mistral-Small-3.2-24B-Instruct-2506",
        name: "Mistral Small 3.2 24B",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "Qwen3.6-27B",
        name: "Qwen 3.6 27B",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "Qwen3.5-397B-A17B",
        name: "Qwen 3.5 397B",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "Qwen3.5-9B",
        name: "Qwen 3.5 9B",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large V3 Turbo",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "laguna-xs-2.1",
        name: "Laguna XS 2.1 (Coding)",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
    CatalogModel {
        id: "Olala-v1.0-mini-7A1B",
        name: "Olala V1.0 Mini",
        provider: ProviderKind::Ovh,
        target_format: WireFormat::OpenAiChat,
        supported_formats: &[WireFormat::OpenAiChat],
    },
];

pub fn curated_models_for_provider(provider: ProviderKind) -> &'static [CatalogModel] {
    match provider {
        ProviderKind::OpenCodeFree => OPENCODE_FREE_MODELS,
        ProviderKind::Antigravity => ANTIGRAVITY_MODELS,
        ProviderKind::Ovh => OVH_MODELS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefix_and_reasoning_effort() {
        let p = parse_model_spec("oc/deepseek-v4.1-flash-free(high)");
        assert_eq!(p.provider_hint, Some(ProviderKind::OpenCodeFree));
        assert_eq!(p.model_id, "deepseek-v4.1-flash-free");
        assert_eq!(p.reasoning_effort.as_deref(), Some("high"));

        let unprefixed = parse_model_spec("iceman-free");
        assert_eq!(unprefixed.provider_hint, None);
        assert_eq!(unprefixed.model_id, "iceman-free");
        assert_eq!(unprefixed.reasoning_effort, None);
    }

    #[test]
    fn merges_antigravity_variants_and_formats_fail_fast_names() {
        assert_eq!(
            canonical_antigravity_model("gemini-4-carbon-high"),
            ("gemini-4-carbon".into(), "Gemini 4 Carbon".into())
        );
        assert_eq!(
            canonical_antigravity_model("gemini-4-carbon-low"),
            ("gemini-4-carbon".into(), "Gemini 4 Carbon".into())
        );
        assert_eq!(
            canonical_antigravity_model("gemini-3.8-flash-tiered"),
            ("gemini-3.8-flash".into(), "Gemini 3.8 Flash".into())
        );
        assert_eq!(antigravity_quota_short_name("gemini-4-argon-high"), "Argon");
        assert_eq!(antigravity_quota_short_name("argon"), "Argon");
        assert!(is_excluded_antigravity_model_key("tab_jump_flash_lite_preview"));
        assert!(is_excluded_antigravity_model_key("gemini-4-argon-disabled"));
        assert!(is_excluded_antigravity_model_key("gemini-4-hydrogen-sonar"));
        assert!(is_excluded_antigravity_model_key("orchestrator-carbon"));
        assert!(is_excluded_antigravity_model_key("orchestrator-argon"));

        let (flash_levels, flash_def) =
            model_level_capabilities(ProviderKind::Antigravity, "gemini-3.8-flash", None, None);
        assert_eq!(flash_levels, vec!["low", "medium", "high"]);
        assert_eq!(flash_def.as_deref(), Some("medium"));

        let (carbon_levels, carbon_def) =
            model_level_capabilities(ProviderKind::Antigravity, "gemini-4-carbon", None, None);
        assert_eq!(carbon_levels, vec!["low", "medium", "high"]);
        assert_eq!(carbon_def.as_deref(), Some("high"));

        let (sonnet_levels, sonnet_def) =
            model_level_capabilities(ProviderKind::Antigravity, "claude-sonnet-4-6", None, None);
        assert!(sonnet_levels.is_empty());
        assert_eq!(sonnet_def, None);

        let (oc_levels, oc_def) =
            model_level_capabilities(ProviderKind::OpenCodeFree, "deepseek-v4.1-flash-free", Some(true), None);
        assert_eq!(oc_levels[0], "auto");
        assert!(oc_levels.contains(&"xhigh".to_string()));
        assert_eq!(oc_def.as_deref(), Some("auto"));
    }

    #[test]
    fn resolves_target_wire_formats() {
        assert_eq!(
            target_format(ProviderKind::OpenCodeFree, "deepseek-v4.1-flash-free"),
            WireFormat::OpenAiChat
        );
        assert_eq!(
            target_format(ProviderKind::OpenCodeFree, "oc/muse-spark-1.3-contributor-free"),
            WireFormat::OpenAiResponses
        );
        assert_eq!(
            target_format(ProviderKind::OpenCodeFree, "union-alpha"),
            WireFormat::AnthropicMessages
        );
    }
}
