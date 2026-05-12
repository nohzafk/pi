//! Anthropic Messages API provider.
//!
//! Implements `anthropic-messages` for `streamSimple`. The current Rust port
//! uses the non-streaming `/v1/messages` endpoint and re-emits the final
//! `AssistantMessage` as a single `Done` event, which is sufficient for the
//! agent loop. SSE streaming can be layered on later without changing the
//! public API of `pi-ai`.
//!
//! This mirrors `packages/ai/src/providers/anthropic.ts` from the original TS.

use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::providers::Provider;
use crate::stream::AssistantMessageEventStream;
use crate::types::{
    now_ms, AssistantMessage, AssistantMessageEvent, Content, Context, Message, Model, StopReason,
    StreamOptions, ThinkingLevel, Usage,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct AnthropicTool<'a> {
    name: &'a str,
    description: &'a str,
    input_schema: &'a Value,
}

#[derive(Deserialize, Debug)]
struct ApiResponse {
    #[serde(default)]
    content: Vec<ApiContent>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    model: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiContent {
    Text { text: String },
    Thinking { thinking: String, signature: Option<String> },
    ToolUse { id: String, name: String, input: Value },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

pub struct AnthropicProvider {
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .pool_max_idle_per_host(4)
                .build()
                .expect("reqwest client"),
        }
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn convert_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        match m {
            Message::User { content, .. } => {
                let blocks = content.iter().map(content_to_block).collect::<Vec<_>>();
                out.push(json!({"role": "user", "content": blocks}));
            }
            Message::Assistant(a) => {
                let blocks = a.content.iter().map(content_to_block).collect::<Vec<_>>();
                out.push(json!({"role": "assistant", "content": blocks}));
            }
            Message::ToolResult(tr) => {
                let mut blocks: Vec<Value> = Vec::new();
                let body: Vec<Value> = tr
                    .content
                    .iter()
                    .map(|c| match c {
                        Content::Text { text } => json!({"type": "text", "text": text}),
                        Content::Image { data, mime_type } => json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": mime_type, "data": data}
                        }),
                        _ => json!({"type": "text", "text": ""}),
                    })
                    .collect();
                blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tr.tool_call_id,
                    "content": body,
                    "is_error": tr.is_error,
                }));
                out.push(json!({"role": "user", "content": blocks}));
            }
        }
    }
    out
}

fn content_to_block(c: &Content) -> Value {
    match c {
        Content::Text { text } => json!({"type": "text", "text": text}),
        Content::Thinking { thinking, thinking_signature } => {
            let mut v = json!({"type": "thinking", "thinking": thinking});
            if let Some(sig) = thinking_signature {
                v["signature"] = json!(sig);
            }
            v
        }
        Content::Image { data, mime_type } => json!({
            "type": "image",
            "source": {"type": "base64", "media_type": mime_type, "data": data}
        }),
        Content::ToolCall { id, name, arguments } => json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": arguments,
        }),
    }
}

fn thinking_budget(level: ThinkingLevel) -> Option<u32> {
    match level {
        ThinkingLevel::Off => None,
        ThinkingLevel::Minimal => Some(1024),
        ThinkingLevel::Low => Some(2048),
        ThinkingLevel::Medium => Some(8192),
        ThinkingLevel::High => Some(16384),
        ThinkingLevel::Xhigh => Some(24576),
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn stream(
        &self,
        model: &Model,
        context: &Context,
        options: &StreamOptions,
    ) -> Result<AssistantMessageEventStream> {
        let api_key = options
            .api_key
            .clone()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .ok_or_else(|| Error::MissingApiKey("anthropic".into()))?;

        let mut body = json!({
            "model": model.id,
            "max_tokens": options.max_tokens.unwrap_or(model.max_tokens),
            "messages": convert_messages(&context.messages),
        });
        if let Some(sp) = &context.system_prompt {
            body["system"] = json!(sp);
        }
        if let Some(t) = options.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(level) = options.reasoning {
            if let Some(budget) = thinking_budget(level) {
                body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            }
        }
        if !context.tools.is_empty() {
            let tools: Vec<AnthropicTool> = context
                .tools
                .iter()
                .map(|t| AnthropicTool {
                    name: &t.name,
                    description: &t.description,
                    input_schema: &t.parameters,
                })
                .collect();
            body["tools"] = serde_json::to_value(tools)?;
        }

        let url = format!("{}/v1/messages", model.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::ProviderError {
                status: status.as_u16(),
                body,
            });
        }

        let parsed: ApiResponse = resp.json().await?;
        let api = model.api.clone();
        let provider = model.provider.clone();
        let model_id = model.id.clone();

        let s = stream! {
            yield Ok(AssistantMessageEvent::Start);

            let mut content_index: usize = 0;
            let mut out_content: Vec<Content> = Vec::new();
            for block in parsed.content {
                match block {
                    ApiContent::Text { text } => {
                        yield Ok(AssistantMessageEvent::TextStart { content_index });
                        yield Ok(AssistantMessageEvent::TextDelta {
                            content_index,
                            delta: text.clone(),
                        });
                        yield Ok(AssistantMessageEvent::TextEnd {
                            content_index,
                            content: text.clone(),
                        });
                        out_content.push(Content::Text { text });
                        content_index += 1;
                    }
                    ApiContent::Thinking { thinking, signature } => {
                        yield Ok(AssistantMessageEvent::ThinkingStart { content_index });
                        yield Ok(AssistantMessageEvent::ThinkingDelta {
                            content_index,
                            delta: thinking.clone(),
                        });
                        yield Ok(AssistantMessageEvent::ThinkingEnd {
                            content_index,
                            content: thinking.clone(),
                        });
                        out_content.push(Content::Thinking {
                            thinking,
                            thinking_signature: signature,
                        });
                        content_index += 1;
                    }
                    ApiContent::ToolUse { id, name, input } => {
                        yield Ok(AssistantMessageEvent::ToolCallStart {
                            content_index,
                            id: id.clone(),
                            name: name.clone(),
                        });
                        yield Ok(AssistantMessageEvent::ToolCallEnd {
                            content_index,
                            id: id.clone(),
                            name: name.clone(),
                            arguments: input.clone(),
                        });
                        out_content.push(Content::ToolCall {
                            id,
                            name,
                            arguments: input,
                        });
                        content_index += 1;
                    }
                    ApiContent::Other => {}
                }
            }

            let stop = match parsed.stop_reason.as_deref() {
                Some("tool_use") => StopReason::ToolUse,
                Some("max_tokens") => StopReason::Length,
                Some("end_turn") | Some("stop_sequence") | None => StopReason::Stop,
                _ => StopReason::Stop,
            };

            let usage = parsed.usage.map(|u| Usage {
                input: u.input_tokens,
                output: u.output_tokens,
                cache_read: u.cache_read_input_tokens,
                cache_write: u.cache_creation_input_tokens,
                total_tokens: u.input_tokens + u.output_tokens,
            }).unwrap_or_default();

            let message = AssistantMessage {
                content: out_content,
                api,
                provider,
                model: parsed.model.unwrap_or(model_id),
                usage,
                stop_reason: stop,
                error_message: None,
                timestamp: now_ms(),
            };

            yield Ok(AssistantMessageEvent::Done { reason: stop, message });
        };

        Ok(s.boxed())
    }
}
