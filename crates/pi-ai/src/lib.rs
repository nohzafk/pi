//! `pi-ai` — Unified multi-provider LLM API.
//!
//! Rust port of the TypeScript package `@earendil-works/pi-ai`. The original
//! project supports many providers; this port focuses on the two most common
//! agent backends: Anthropic Messages and OpenAI Chat Completions. Both share
//! the same `Message`/`Tool`/`AssistantMessageEvent` types so the agent loop
//! is provider-agnostic.

pub mod error;
pub mod providers;
pub mod stream;
pub mod types;

pub use error::{Error, Result};
pub use providers::{anthropic::AnthropicProvider, openai::OpenAiProvider, Provider};
pub use stream::AssistantMessageEventStream;
pub use types::{
    now_ms, AssistantMessage, AssistantMessageEvent, Content, Context, Message, Model, StopReason,
    StreamOptions, ThinkingLevel, Tool, ToolResultMessage, Usage,
};

/// Entry point that mirrors `streamSimple()` in pi-ai TS: pick the provider
/// implementation from `model.api` and return a stream of message events.
pub async fn stream_simple(
    model: &Model,
    context: &Context,
    options: &StreamOptions,
) -> Result<AssistantMessageEventStream> {
    match model.api.as_str() {
        "anthropic-messages" => {
            let p = AnthropicProvider::new();
            p.stream(model, context, options).await
        }
        "openai-completions" => {
            let p = OpenAiProvider::new();
            p.stream(model, context, options).await
        }
        other => Err(Error::UnsupportedProvider(other.into())),
    }
}
