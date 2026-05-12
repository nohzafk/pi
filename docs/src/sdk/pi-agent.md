# pi-agent

`pi-agent` is the agent loop. It owns the conversation transcript, calls
`pi-ai::stream_simple` for each turn, dispatches tool calls, and emits
streaming events to a subscriber channel.

```toml
[dependencies]
pi-ai = "1.1"
pi-agent = "1.1"
tokio = { version = "1", features = ["full"] }
```

The example below adds a custom tool alongside the bundled defaults:

```rust
use async_trait::async_trait;
use pi_agent::{run_agent, tools::default_tools, AgentConfig, AgentTool, ToolError};
use pi_ai::{Message, Model};
use serde_json::{json, Value};

struct Echo;

#[async_trait]
impl AgentTool for Echo {
    fn name(&self) -> &str { "echo" }
    fn description(&self) -> &str { "Echo a string back to the agent." }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"],
        })
    }
    async fn execute(&self, args: Value) -> Result<String, ToolError> {
        Ok(args["text"].as_str().unwrap_or("").to_string())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut tools = default_tools();
    tools.push(Box::new(Echo));

    let cfg = AgentConfig::new(
        Model::anthropic_claude_sonnet_4_6(),
        "You are a helpful coding assistant.",
    )
    .with_tools(tools)
    .with_max_turns(8);

    let user = Message::user_text("Use the echo tool to say hello.");
    let run = run_agent(&cfg, user, None).await?;
    println!("{} messages", run.messages.len());
    Ok(())
}
```

Use `run_agent_with_history` to resume an existing transcript. Pass a
`PermissionPolicy` through `AgentConfig` to gate sensitive tools.
