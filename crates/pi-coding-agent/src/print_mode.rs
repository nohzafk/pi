//! One-shot "print" mode: run a single prompt to completion and print the result.
//!
//! Equivalent to `packages/coding-agent/src/modes/print-mode.ts`.

use pi_agent::{run_agent, tools::default_tools, AgentConfig, AgentEvent};
use pi_ai::{Content, Message};
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::system_prompt::SYSTEM_PROMPT;

pub async fn run_print(app: &AppConfig, prompt: String) -> anyhow::Result<()> {
    let cfg = AgentConfig::new(app.model.clone(), SYSTEM_PROMPT)
        .with_tools(default_tools())
        .with_max_turns(app.max_turns);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let user = Message::user_text(prompt);

    let cfg_cloned = cfg.clone();
    let handle = tokio::spawn(async move { run_agent(&cfg_cloned, user, Some(tx)).await });

    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::AssistantMessage { message: Message::Assistant(m) } => {
                for c in &m.content {
                    if let Content::Text { text } = c {
                        println!("{}", text);
                    }
                }
            }
            AgentEvent::ToolExecutionStart { tool_name, args, .. } => {
                eprintln!("→ {}({})", tool_name, args);
            }
            AgentEvent::ToolExecutionEnd { tool_name, is_error, .. } => {
                if is_error {
                    eprintln!("← {} (error)", tool_name);
                } else {
                    eprintln!("← {} ok", tool_name);
                }
            }
            _ => {}
        }
    }

    let res = handle.await?.map_err(anyhow::Error::msg)?;
    if res.stopped_at_turn_limit {
        eprintln!("(stopped at max turns)");
    }
    Ok(())
}
