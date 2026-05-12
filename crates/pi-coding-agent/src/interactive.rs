//! Interactive (REPL) mode: read user lines from stdin, run agent turns,
//! print streaming-style output. The TS coding-agent ships a full TUI; this
//! Rust port keeps a simple line-based UI for clarity.

use std::io::{BufRead, Write};

use pi_agent::{run_agent, tools::default_tools, AgentConfig, AgentEvent};
use pi_ai::{Content, Message};
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::system_prompt::SYSTEM_PROMPT;

pub async fn run_interactive(app: &AppConfig) -> anyhow::Result<()> {
    eprintln!(
        "pi (rust) — model: {} ({})",
        app.model.name, app.model.provider
    );
    eprintln!("Type your message and press Enter. Slash commands: /quit /exit /help /model");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut transcript: Vec<Message> = Vec::new();

    loop {
        write!(stdout, "\n> ")?;
        stdout.flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let prompt = line.trim().to_string();
        if prompt.is_empty() {
            continue;
        }
        if prompt.starts_with('/') {
            match prompt.as_str() {
                "/quit" | "/exit" => break,
                "/help" => {
                    eprintln!("Slash commands: /quit /exit /help /model /reset");
                    continue;
                }
                "/reset" => {
                    transcript.clear();
                    eprintln!("(transcript cleared)");
                    continue;
                }
                "/model" => {
                    eprintln!("model: {} ({})", app.model.name, app.model.provider);
                    continue;
                }
                _ => {
                    eprintln!("unknown command: {prompt}");
                    continue;
                }
            }
        }

        let cfg = AgentConfig::new(app.model.clone(), build_system_prompt(&transcript))
            .with_tools(default_tools())
            .with_max_turns(app.max_turns);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let user = Message::user_text(prompt);
        transcript.push(user.clone());

        let cfg_cloned = cfg.clone();
        let handle = tokio::spawn(async move { run_agent(&cfg_cloned, user, Some(tx)).await });

        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::AssistantMessage { message } => {
                    if let Message::Assistant(m) = &message {
                        for c in &m.content {
                            if let Content::Text { text } = c {
                                println!("{}", text);
                            }
                        }
                    }
                    transcript.push(message);
                }
                AgentEvent::ToolExecutionStart { tool_name, args, .. } => {
                    eprintln!("  → {}({})", tool_name, args);
                }
                AgentEvent::ToolExecutionEnd { tool_name, is_error, content, .. } => {
                    let snippet: String = content
                        .iter()
                        .filter_map(|c| match c {
                            Content::Text { text } => Some(text.lines().next().unwrap_or("").to_string()),
                            _ => None,
                        })
                        .next()
                        .unwrap_or_default();
                    let tag = if is_error { "err" } else { "ok" };
                    eprintln!("  ← {} {} {}", tool_name, tag, truncate(&snippet, 80));
                }
                _ => {}
            }
        }
        let res = handle.await?.map_err(anyhow::Error::msg)?;
        // Take final message list (which already includes the user prompt) so we
        // keep tool-result messages in transcript on the next iteration.
        transcript = res.messages;
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

fn build_system_prompt(_history: &[Message]) -> String {
    SYSTEM_PROMPT.to_string()
}
