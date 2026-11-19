use serde_json::{json, Value};
use tesie_core::{MultimodalBridgeConfig, Result, WireFormat};
use tesie_providers::{OpenCodeEngine, ProviderRequest};
use uuid::Uuid;

use crate::aggregator::drain_and_aggregate;

pub fn model_supports_vision(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    // Strip reasoning effort or capability levels: e.g. "ag/gemini-3.8-flash(high)" -> "ag/gemini-3.8-flash"
    let clean = if let Some(idx) = lower.find('(') {
        &lower[..idx]
    } else {
        &lower
    };
    let clean = clean.trim();

    // 1. Google Gemini family: all native Gemini models support vision
    if clean.contains("gemini") {
        return true;
    }

    // 2. Anthropic Claude family: any Claude (Claude 3, 3.5, 3.7, 4), or sonnet, opus, haiku
    if clean.contains("claude")
        || clean.contains("sonnet")
        || clean.contains("opus")
        || clean.contains("haiku")
    {
        return true;
    }

    // 3. OpenAI GPT family with vision: gpt-4o, gpt-4-turbo, gpt-4.5, gpt-5, o1, o3, o4
    if clean.contains("gpt-4o")
        || clean.contains("gpt-4-turbo")
        || clean.contains("gpt-4.5")
        || clean.contains("gpt-5")
        || clean.contains("o1")
        || clean.contains("o3")
        || clean.contains("o4")
    {
        return true;
    }

    // 4. Vision markers: -vl, _vl, vision, -vlm, _vlm
    if clean.contains("-vl")
        || clean.contains("_vl")
        || clean.contains("vision")
        || clean.contains("-vlm")
        || clean.contains("_vlm")
    {
        return true;
    }

    // 5. OpenCode & Open-weight vision models:
    // - DeepSeek vision models: deepseek-v4, deepseek-v4.1, deepseek-vl, deepseek-vl2
    // - GLM vision models: glm-4v, glm-4.5v, glm-5v, glm-5.3
    // - MiMo vision models: mimo-v2.6, mimo-v2.5, mimo-v2-omni
    // - LongCat vision: longcat-2.5
    // - HY4 vision: hy4-flash
    // - Inkling vision: inkling-2
    // - Nemotron Super VL: super-vl
    // - Specialized vision families: pixtral, llava, internvl, kimi-k2, minimax-m3, step-5, crouton, exo-free, pulse-alpha, muse-spark, molmo, cogvlm, florence
    if clean.contains("deepseek-v4")
        || clean.contains("deepseek-vl")
        || clean.contains("glm-4v")
        || clean.contains("glm-5v")
        || clean.contains("glm-5.3")
        || clean.contains("mimo-v2.6")
        || clean.contains("mimo-v2.5")
        || clean.contains("mimo-v2-omni")
        || clean.contains("longcat-2.5")
        || clean.contains("hy4-flash")
        || clean.contains("inkling-2")
        || clean.contains("super-vl")
        || clean.contains("kimi-k2")
        || clean.contains("minimax-m3")
        || clean.contains("step-5")
        || clean.contains("crouton")
        || clean.contains("exo-free")
        || clean.contains("pulse-alpha")
        || clean.contains("muse-spark")
        || clean.contains("pixtral")
        || clean.contains("llava")
        || clean.contains("internvl")
        || clean.contains("molmo")
        || clean.contains("cogvlm")
        || clean.contains("florence")
    {
        return true;
    }

    // 6. Qwen VL specifically (requires 'vl' or 'vision' so text-only Qwen3 models like Qwen3.6-27B are not falsely matched)
    if clean.contains("qwen") && (clean.contains("vl") || clean.contains("vision")) {
        return true;
    }

    false
}

pub fn model_supports_audio(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    let clean = if let Some(idx) = lower.find('(') {
        &lower[..idx]
    } else {
        &lower
    };
    let clean = clean.trim();

    clean.contains("gemini")
        || clean.contains("gpt-4o-audio")
        || clean.contains("whisper")
        || clean.contains("muse")
        || clean.contains("mimo")
}

pub fn request_has_multimodal(body: &Value, _source_format: WireFormat) -> (bool, bool) {
    let mut has_images = false;
    let mut has_audio = false;

    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return (false, false);
    };

    for msg in messages {
        if let Some(parts) = msg.get("content").and_then(Value::as_array) {
            for part in parts {
                let p_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                if p_type == "image_url" || p_type == "image" {
                    has_images = true;
                } else if p_type == "input_audio" || p_type == "audio" {
                    has_audio = true;
                }
            }
        }
    }

    (has_images, has_audio)
}

fn build_vision_system_prompt(cfg: &MultimodalBridgeConfig) -> String {
    if let Some(custom) = &cfg.vision_prompt {
        if !custom.trim().is_empty() {
            return custom.clone();
        }
    }

    match cfg.vision_mode.as_str() {
        "code_ocr" => "You are an expert multimodal transcription engine for a text-only coding assistant. \
The user is providing an image (screenshot, diagram, terminal, or code). \
Extract and transcribe ALL visible text, code snippets, file paths, error messages, and logs EXACTLY as they appear. \
Describe any UI layouts, component hierarchies, or structural flow relevant to a software engineer. \
Be extremely precise, factual, and direct without filler."
            .to_string(),
        "minimal" => "Briefly describe what this image shows in 1-2 concise, factual sentences for a text-only assistant."
            .to_string(),
        _ => "You are an expert multimodal transcription engine. \
The user wants to analyze this image with a text-only model. \
Analyze the image and provide a thorough, high-density description including: \
1. What the image depicts (photo, screenshot, diagram, document, chart, etc.). \
2. All visible text, labels, titles, numbers, and annotations verbatim. \
3. Key visual context, spatial layout, and relationships between elements. \
Be direct, objective, and detailed so the text-only model has complete context."
            .to_string(),
    }
}

pub async fn bridge_request_if_needed(
    engine: &OpenCodeEngine,
    body: &mut Value,
    source_format: WireFormat,
    target_model: &str,
    headers: &[(String, String)],
    api_key_id: &Option<String>,
    cfg: &MultimodalBridgeConfig,
) -> Result<bool> {
    let (has_images, has_audio) = request_has_multimodal(body, source_format);

    let needs_vision_bridge = has_images && cfg.vision_enabled && !model_supports_vision(target_model);
    let needs_audio_bridge = has_audio && cfg.audio_enabled && !model_supports_audio(target_model);

    if !needs_vision_bridge && !needs_audio_bridge {
        return Ok(false);
    }

    let mut transformed = false;
    let vision_model = cfg
        .vision_model
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("ag/gemini-3.8-flash");

    if needs_vision_bridge {
        let system_instruction = build_vision_system_prompt(cfg);

        if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
            for msg in messages.iter_mut() {
                let Some(content_parts) = msg.get_mut("content").and_then(Value::as_array_mut) else {
                    continue;
                };

                let mut new_parts = Vec::new();
                for part in content_parts.drain(..) {
                    let p_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                    if p_type == "image_url" || p_type == "image" {
                        // Transcribe this image part via the vision adapter
                        let bridge_body = json!({
                            "model": vision_model,
                            "messages": [
                                {
                                    "role": "user",
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": system_instruction
                                        },
                                        part.clone()
                                    ]
                                }
                            ],
                            "max_tokens": 1500,
                            "stream": false
                        });

                        let stream_resp = engine
                            .execute_chat(ProviderRequest {
                                request_id: format!("bridge-{}", Uuid::new_v4()),
                                source_format,
                                raw_model: vision_model.to_string(),
                                body: bridge_body,
                                headers: headers.to_vec(),
                                api_key_id: api_key_id.clone(),
                            })
                            .await;

                        match stream_resp {
                            Ok(resp) => {
                                if let Ok(agg) =
                                    drain_and_aggregate(source_format, vision_model, resp.stream).await
                                {
                                    let description = extract_content_from_agg(&agg);
                                    let formatted_text = if !description.trim().is_empty() {
                                        format!(
                                            "\n[Attached Image Analysis (Transcribed by Multimodal Bridge):\n{}\n]\n",
                                            description.trim()
                                        )
                                    } else {
                                        "\n[Attached Image (Empty or could not be transcribed)]\n"
                                            .to_string()
                                    };
                                    new_parts.push(json!({
                                        "type": "text",
                                        "text": formatted_text
                                    }));
                                    transformed = true;
                                } else {
                                    new_parts.push(part);
                                }
                            }
                            Err(_) => {
                                // Fallback: leave original part
                                new_parts.push(part);
                            }
                        }
                    } else {
                        new_parts.push(part);
                    }
                }
                *content_parts = new_parts;
            }
        }
    }

    if transformed {
        let transcriber_provider = if vision_model.starts_with("ag/") {
            "antigravity".to_string()
        } else if vision_model.starts_with("ovh/") {
            "ovh".to_string()
        } else if vision_model.starts_with("lf/") || vision_model.starts_with("cp-") {
            "cp-lf".to_string()
        } else {
            "opencode".to_string()
        };

        let target_provider = if target_model.starts_with("ag/") {
            "antigravity".to_string()
        } else if target_model.starts_with("ovh/") {
            "ovh".to_string()
        } else if target_model.starts_with("lf/") || target_model.starts_with("cp-") {
            "cp-lf".to_string()
        } else {
            "opencode".to_string()
        };

        engine.record_bridge_event(tesie_core::BridgeEvent {
            transcriber_provider,
            target_provider,
            transcriber_model: vision_model.to_string(),
            target_model: target_model.to_string(),
            kind: "image".to_string(),
            timestamp_ms: tesie_store::now_ms(),
        });
    }

    Ok(transformed)
}

fn extract_content_from_agg(agg: &Value) -> String {
    // OpenAI format
    if let Some(content) = agg
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
    {
        return content.to_string();
    }

    // Anthropic format
    if let Some(content) = agg.get("content").and_then(Value::as_array) {
        let mut s = String::new();
        for block in content {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                s.push_str(text);
            }
        }
        if !s.is_empty() {
            return s;
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vision_models_detection() {
        // Antigravity models
        assert!(model_supports_vision("ag/gemini-2.5-flash"));
        assert!(model_supports_vision("ag/gemini-3.8-flash(high)"));
        assert!(model_supports_vision("ag/claude-sonnet-4-6"));
        assert!(model_supports_vision("claude-sonnet-4.6"));
        assert!(model_supports_vision("claude-opus-4-6-thinking"));
        assert!(model_supports_vision("claude-3-7-sonnet"));
        assert!(model_supports_vision("sonnet-4"));
        assert!(model_supports_vision("opus-4"));
        assert!(model_supports_vision("claude-3.5-haiku"));

        // OpenCode models
        assert!(model_supports_vision("oc/deepseek-v4.1-flash-free"));
        assert!(model_supports_vision("oc/deepseek-v4.1-flash-free(auto)"));
        assert!(model_supports_vision("mimo-v2.6-flash-free"));
        assert!(model_supports_vision("glm-5.3-flash-free"));
        assert!(model_supports_vision("nemotron-3.5-super-vl-free"));
        assert!(model_supports_vision("longcat-2.5-free"));
        assert!(model_supports_vision("hy4-flash-free"));
        assert!(model_supports_vision("inkling-2-small-free"));

        // OVHcloud models
        assert!(model_supports_vision("ovh/Qwen2.5-VL-72B-Instruct"));
        assert!(model_supports_vision("ovh/qwen3-vl-30b-a3b"));

        // Custom Provider models
        assert!(model_supports_vision("cp-lf/deepseek-v4.1-flash-1017"));
        assert!(model_supports_vision("cp-lf/glm-4v"));
        assert!(model_supports_vision("cp-lf/gpt-4o"));
        assert!(model_supports_vision("pixtral-12b"));
        assert!(model_supports_vision("meta-llama/llama-3.2-11b-vision-instruct"));

        // Text-only models that MUST return false
        assert!(!model_supports_vision("oc/beam-free"));
        assert!(!model_supports_vision("oc/iceman-free"));
        assert!(!model_supports_vision("oc/flint-40b-free"));
        assert!(!model_supports_vision("big-pickle"));
        assert!(!model_supports_vision("oc/deepseek-v3"));
        assert!(!model_supports_vision("Meta-Llama-3_3-70B-Instruct"));
        assert!(!model_supports_vision("Mistral-Small-3.2-24B-Instruct-2506"));
        assert!(!model_supports_vision("Qwen3.6-27B"));
        assert!(!model_supports_vision("Qwen3.5-397B-A17B"));
        assert!(!model_supports_vision("Qwen3.5-9B"));
        assert!(!model_supports_vision("gpt-3.5-turbo"));
    }

    #[test]
    fn test_audio_models_detection() {
        assert!(model_supports_audio("ag/gemini-2.5-flash"));
        assert!(model_supports_audio("ag/gemini-3.8-flash(high)"));
        assert!(model_supports_audio("ovh/whisper-large-v3-turbo"));
        assert!(!model_supports_audio("ag/claude-sonnet-4-6"));
        assert!(!model_supports_audio("oc/beam-free"));
    }
}
