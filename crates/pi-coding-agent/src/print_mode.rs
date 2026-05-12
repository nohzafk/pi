//! One-shot "print" mode: run a single prompt to completion and print the
//! result. Streams text deltas to stdout as they arrive.

use std::io::Write;
use std::sync::Arc;

use pi_agent::{run_agent, tools::default_tools, AgentConfig, AgentEvent, PermissionPolicy};
use pi_ai::Message;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::system_prompt::build_system_prompt;

pub async fn run_print(
    app: &AppConfig,
    prompt: String,
    permission: Arc<dyn PermissionPolicy>,
) -> anyhow::Result<()> {
    let cfg = AgentConfig::new(app.model.clone(), build_system_prompt(&app.config_dir))
        .with_tools(default_tools())
        .with_max_turns(app.max_turns)
        .with_permission(permission);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let user = Message::user_text(prompt);

    let cfg_cloned = cfg.clone();
    let handle = tokio::spawn(async move { run_agent(&cfg_cloned, user, Some(tx)).await });

    let mut stdout = std::io::stdout();
    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::TextDelta { delta } => {
                let _ = write!(stdout, "{}", delta);
                let _ = stdout.flush();
            }
            AgentEvent::AssistantMessage { .. } => {
                let _ = writeln!(stdout);
            }
            AgentEvent::ToolExecutionStart {
                tool_name, args, ..
            } => {
                eprintln!("→ {}({})", tool_name, args);
            }
            AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                ..
            } => {
                eprintln!("← {} {}", tool_name, if is_error { "error" } else { "ok" });
            }
            AgentEvent::PermissionDenied { tool_name, reason } => {
                eprintln!("✗ permission denied for {tool_name}: {reason}");
            }
            _ => {}
        }
    }

    let res = handle.await??;
    if res.stopped_at_turn_limit {
        eprintln!("(stopped at max turns)");
    }
    Ok(())
}
