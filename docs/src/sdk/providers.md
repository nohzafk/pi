# Providers

`pi-ai` ships three first-party providers: Anthropic Messages, OpenAI
Chat Completions (plus any OpenAI-compatible base URL), and Google
Generative AI. To add another wire protocol, implement the `Provider`
trait.

The trait converts a `Context` into a provider-shaped request, streams
the response, and emits canonical events that the agent layer
understands:

```rust
use async_trait::async_trait;
use futures::stream::BoxStream;
use pi_ai::{Context, Provider, StreamEvent, StreamOptions, StreamError};

pub struct MyProvider;

#[async_trait]
impl Provider for MyProvider {
    fn name(&self) -> &'static str { "my-provider" }

    async fn stream(
        &self,
        ctx: &Context,
        opts: &StreamOptions,
    ) -> Result<BoxStream<'static, Result<StreamEvent, StreamError>>, StreamError> {
        // 1. Build the HTTP request (use opts.api_key / opts.base_url / opts.headers).
        // 2. POST it, then parse the SSE response into a stream of StreamEvent.
        // 3. Honor opts.cancel: stop reading when the CancellationToken fires.
        // 4. Use the shared retry helper to classify 429 / 5xx.
        todo!()
    }
}
```

If your endpoint already speaks OpenAI Chat Completions, you do not need
a new provider — call `Model::openai_compat(...)` or override
`StreamOptions::base_url` at call time and reuse `OpenAiProvider`.

```rust
use pi_ai::Model;
let m = Model::openai_compat(
    "openrouter",
    "anthropic/claude-3.5-sonnet",
    "https://openrouter.ai/api/v1",
    200_000,
    8_192,
);
```
