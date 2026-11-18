pub mod chat_claude;
pub mod chat_responses;
pub mod sse;

use bytes::Bytes;
use serde_json::Value;
use tesie_core::WireFormat;

use crate::opencode::fingerprint::{restore_tool_names, ToolRenameMap};
use chat_claude::{
    chat_request_to_claude, claude_request_to_chat, flush_chat_to_claude,
    translate_chat_chunk_to_claude, translate_claude_event_to_chat, ChatToClaudeStreamState,
    ClaudeToChatStreamState,
};
use chat_responses::{
    chat_request_to_responses, flush_chat_to_responses, responses_request_to_chat,
    translate_chat_chunk_to_responses, translate_responses_event_to_chat,
    ChatToResponsesStreamState, ResponsesToChatStreamState,
};
use sse::{format_sse_done, format_sse_frame, SseBuffer, SseEvent, UsageAccumulator};

pub fn translate_request(
    source: WireFormat,
    target: WireFormat,
    model: &str,
    body: &Value,
) -> Value {
    if source == target {
        let mut out = body.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("model".into(), Value::String(model.to_string()));
        }
        return out;
    }

    // Pivot through OpenAI Chat when source != target
    let chat_body = match source {
        WireFormat::OpenAiChat => {
            let mut c = body.clone();
            if let Some(obj) = c.as_object_mut() {
                obj.insert("model".into(), Value::String(model.to_string()));
            }
            c
        }
        WireFormat::OpenAiResponses => responses_request_to_chat(model, body),
        WireFormat::AnthropicMessages => claude_request_to_chat(model, body),
    };

    match target {
        WireFormat::OpenAiChat => chat_body,
        WireFormat::OpenAiResponses => chat_request_to_responses(model, &chat_body),
        WireFormat::AnthropicMessages => chat_request_to_claude(model, &chat_body),
    }
}

pub struct StreamTranslator {
    source: WireFormat,
    target: WireFormat,
    model: String,
    rename_map: ToolRenameMap,
    sse_buf: SseBuffer,
    pub usage: UsageAccumulator,
    resp_to_chat: ResponsesToChatStreamState,
    claude_to_chat: ClaudeToChatStreamState,
    chat_to_resp: ChatToResponsesStreamState,
    chat_to_claude: ChatToClaudeStreamState,
    done_emitted: bool,
}

impl StreamTranslator {
    pub fn new(
        source: WireFormat,
        target: WireFormat,
        model: String,
        rename_map: ToolRenameMap,
    ) -> Self {
        Self {
            source,
            target,
            model,
            rename_map,
            sse_buf: SseBuffer::default(),
            usage: UsageAccumulator::default(),
            resp_to_chat: ResponsesToChatStreamState::default(),
            claude_to_chat: ClaudeToChatStreamState::default(),
            chat_to_resp: ChatToResponsesStreamState::default(),
            chat_to_claude: ChatToClaudeStreamState::default(),
            done_emitted: false,
        }
    }

    pub fn push_bytes(&mut self, chunk: &[u8]) -> Vec<Bytes> {
        let events = self.sse_buf.push(chunk);
        self.process_events(events)
    }

    pub fn finish(&mut self) -> Vec<Bytes> {
        let mut out = Vec::new();
        let tail = self.sse_buf.finish();
        out.extend(self.process_events(tail));

        match self.source {
            WireFormat::OpenAiResponses if self.target != WireFormat::OpenAiResponses => {
                for (ev, data) in flush_chat_to_responses(&mut self.chat_to_resp) {
                    out.push(format_sse_frame(Some(&ev), &data));
                }
            }
            WireFormat::AnthropicMessages if self.target != WireFormat::AnthropicMessages => {
                for (ev, data) in flush_chat_to_claude(&mut self.chat_to_claude) {
                    out.push(format_sse_frame(Some(&ev), &data));
                }
            }
            WireFormat::OpenAiChat if !self.done_emitted => {
                self.done_emitted = true;
                out.push(format_sse_done());
            }
            _ => {}
        }

        out
    }

    fn process_events(&mut self, events: Vec<SseEvent>) -> Vec<Bytes> {
        let mut out = Vec::new();
        for ev in events {
            let trimmed = ev.data.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == "[DONE]" {
                if self.source == WireFormat::OpenAiChat && !self.done_emitted {
                    self.done_emitted = true;
                    out.push(format_sse_done());
                }
                continue;
            }

            let Ok(mut payload) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            self.usage.observe_payload(&payload);
            restore_tool_names(&mut payload, &self.rename_map);

            if self.source == self.target {
                out.push(format_sse_frame(ev.event.as_deref(), &payload));
                continue;
            }

            // Stage 1: target -> OpenAiChat chunks
            let chat_chunks = match self.target {
                WireFormat::OpenAiChat => vec![payload],
                WireFormat::OpenAiResponses => translate_responses_event_to_chat(
                    ev.event.as_deref(),
                    &payload,
                    &mut self.resp_to_chat,
                    &self.model,
                ),
                WireFormat::AnthropicMessages => translate_claude_event_to_chat(
                    ev.event.as_deref(),
                    &payload,
                    &mut self.claude_to_chat,
                    &self.model,
                ),
            };

            // Stage 2: OpenAiChat -> source format
            for chat_chunk in chat_chunks {
                match self.source {
                    WireFormat::OpenAiChat => {
                        out.push(format_sse_frame(None, &chat_chunk));
                    }
                    WireFormat::OpenAiResponses => {
                        for (ev_name, data) in translate_chat_chunk_to_responses(
                            &chat_chunk,
                            &mut self.chat_to_resp,
                            &self.model,
                        ) {
                            out.push(format_sse_frame(Some(&ev_name), &data));
                        }
                    }
                    WireFormat::AnthropicMessages => {
                        for (ev_name, data) in translate_chat_chunk_to_claude(
                            &chat_chunk,
                            &mut self.chat_to_claude,
                            &self.model,
                        ) {
                            out.push(format_sse_frame(Some(&ev_name), &data));
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn translates_chat_to_responses_and_restores_tool_names() {
        let chat_req = json!({
            "model": "muse-spark-1.3-contributor-free",
            "messages": [
                { "role": "system", "content": "be concise" },
                { "role": "user", "content": "check repo" }
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "list_files",
                    "description": "list",
                    "parameters": { "type": "object", "properties": {} }
                }
            }]
        });
        let resp_req = translate_request(
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            "muse-spark-1.3-contributor-free",
            &chat_req,
        );
        assert_eq!(resp_req["instructions"].as_str(), Some("be concise"));
        assert_eq!(resp_req["input"][0]["content"][0]["type"].as_str(), Some("input_text"));
        assert_eq!(resp_req["tools"][0]["name"].as_str(), Some("list_files"));

        let mut rename = ToolRenameMap::new();
        rename.insert("run_command".into(), "playwright_browser_run_code_unsafe".into());

        let mut st = StreamTranslator::new(
            WireFormat::OpenAiChat,
            WireFormat::OpenAiResponses,
            "muse-spark-1.3-contributor-free".into(),
            rename,
        );
        let sse_in = concat!(
            "event: response.output_item.added\n",
            "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"run_command\"}}\n\n",
            "event: response.function_call_arguments.delta\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"x\\\":1}\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":5,\"total_tokens\":15}}}\n\n"
        );
        let frames = st.push_bytes(sse_in.as_bytes());
        let joined: String = frames
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect();
        assert!(joined.contains("playwright_browser_run_code_unsafe"));
        assert_eq!(st.usage.total_tokens, 15);
    }
}
