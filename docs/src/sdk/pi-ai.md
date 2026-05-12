# pi-ai

`pi-ai` is the streaming LLM client. It exposes a `stream_simple()`
function that returns an SSE event stream for any supported provider.

```toml
[dependencies]
pi-ai = "1.1"
futures = "0.3"
tokio = { version = "1", features = ["full"] }
```

Minimal example:

```rust
use pi_ai::{stream_simple, Context, Message, Model, StreamOptions};
use futures::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = Model::anthropic_claude_sonnet_4_6();
    let ctx = Context {
        system_prompt: Some("You are a helpful assistant.".into()),
        messages: vec![Message::user_text("Say hi in one word.")],
        tools: vec![],
    };

    let mut events = stream_simple(&model, &ctx, &StreamOptions::default()).await?;
    while let Some(ev) = events.next().await {
        println!("{:?}", ev?);
    }
    Ok(())
}
```

The API key is read from the environment (`ANTHROPIC_API_KEY`,
`OPENAI_API_KEY`, `GOOGLE_API_KEY` / `GEMINI_API_KEY`) unless you set
`StreamOptions::api_key`. Cancellation, retry with `Retry-After`, and
custom headers are all wired through `StreamOptions`.
