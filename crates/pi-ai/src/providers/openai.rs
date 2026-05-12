//! OpenAI Chat Completions provider (`openai-completions`).
//!
//! Like the Anthropic provider in this Rust port, this implementation issues a
//! single non-streaming request and re-emits the result as a `Done` event.

use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::providers::Provider;
use crate::stream::AssistantMessageEventStream;
use crate::types::{
    now_ms, AssistantMessage, AssistantMessageEvent, Content, Context, Message, Model, StopReason,
    StreamOptions, Usage,
};

#[derive(Deserialize, Debug)]
struct ApiResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Choice {
    message: ChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize, Debug)]
struct ToolCall {
    id: String,
    #[serde(default)]
    function: Function,
}

#[derive(Deserialize, Debug, Default)]
struct Function {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Deserialize, Debug, Default)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

pub struct OpenAiProvider {
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn convert_messages(system_prompt: Option<&str>, messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if let Some(sp) = system_prompt {
        out.push(json!({"role": "system", "content": sp}));
    }
    for m in messages {
        match m {
            Message::User { content, .. } => {
                let text = content
                    .iter()
                    .filter_map(|c| c.as_text().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
                    .join("");
                out.push(json!({"role": "user", "content": text}));
            }
            Message::Assistant(a) => {
                let mut text = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for c in &a.content {
                    match c {
                        Content::Text { text: t } => text.push_str(t),
                        Content::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments.to_string(),
                                }
                            }));
                        }
                        _ => {}
                    }
                }
                let mut msg = json!({"role": "assistant", "content": text});
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = json!(tool_calls);
                }
                out.push(msg);
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|c| c.as_text().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
                    .join("");
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": tr.tool_call_id,
                    "content": text,
                }));
            }
        }
    }
    out
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn stream(
        &self,
        model: &Model,
        context: &Context,
        options: &StreamOptions,
    ) -> Result<AssistantMessageEventStream> {
        let api_key = options
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| Error::MissingApiKey("openai".into()))?;

        let mut body = json!({
            "model": model.id,
            "messages": convert_messages(context.system_prompt.as_deref(), &context.messages),
        });
        if let Some(t) = options.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = options.max_tokens {
            body["max_tokens"] = json!(m);
        }
        if !context.tools.is_empty() {
            let tools: Vec<Value> = context
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools);
        }

        let url = format!(
            "{}/chat/completions",
            model.base_url.trim_end_matches('/')
        );

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&api_key)
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

            let choice = parsed
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| Error::InvalidResponse("no choices".into()));
            let choice = match choice {
                Ok(c) => c,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };

            let mut content_index: usize = 0;
            let mut out_content: Vec<Content> = Vec::new();
            if let Some(t) = choice.message.content {
                if !t.is_empty() {
                    yield Ok(AssistantMessageEvent::TextStart { content_index });
                    yield Ok(AssistantMessageEvent::TextDelta { content_index, delta: t.clone() });
                    yield Ok(AssistantMessageEvent::TextEnd { content_index, content: t.clone() });
                    out_content.push(Content::Text { text: t });
                    content_index += 1;
                }
            }
            for tc in choice.message.tool_calls {
                let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Object(Default::default()));
                yield Ok(AssistantMessageEvent::ToolCallStart {
                    content_index,
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                });
                yield Ok(AssistantMessageEvent::ToolCallEnd {
                    content_index,
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: args.clone(),
                });
                out_content.push(Content::ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: args,
                });
                content_index += 1;
            }

            let stop = match choice.finish_reason.as_deref() {
                Some("tool_calls") => StopReason::ToolUse,
                Some("length") => StopReason::Length,
                _ => StopReason::Stop,
            };
            let usage = parsed.usage.map(|u| Usage {
                input: u.prompt_tokens,
                output: u.completion_tokens,
                cache_read: 0,
                cache_write: 0,
                total_tokens: u.total_tokens,
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
