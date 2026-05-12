//! Interactive (REPL) mode: read user lines from stdin, run agent turns,
//! print streaming output, and dispatch slash commands.

use std::io::{BufRead, Write};
use std::sync::Arc;

use pi_agent::{
    run_agent_with_history, tools::default_tools, AgentConfig, AgentEvent, PermissionPolicy,
};
use pi_ai::Message;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::session::Session;
use crate::system_prompt::build_system_prompt;

pub async fn run_interactive(
    app: &AppConfig,
    permission: Arc<dyn PermissionPolicy>,
    initial: Option<Session>,
) -> anyhow::Result<()> {
    eprintln!(
        "pi — model: {} ({})  •  slash commands: /help",
        app.model.name, app.model.provider
    );

    let mut session = initial.unwrap_or_else(|| Session::new(&app.model));
    if !session.messages.is_empty() {
        eprintln!(
            "(resumed session {}, {} prior messages)",
            session.id,
            session.messages.len()
        );
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let system_prompt = build_system_prompt(&app.config_dir);

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
            if !handle_slash(&prompt, app, &mut session)? {
                break;
            }
            continue;
        }

        let cfg = AgentConfig::new(app.model.clone(), system_prompt.clone())
            .with_tools(default_tools())
            .with_max_turns(app.max_turns)
            .with_thinking(app.thinking_level)
            .with_permission(permission.clone());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let user = Message::user_text(prompt);
        let mut history = session.messages.clone();
        history.push(user);

        let cfg_cloned = cfg.clone();
        let handle =
            tokio::spawn(
                async move { run_agent_with_history(&cfg_cloned, history, Some(tx)).await },
            );

        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::TextDelta { delta } => {
                    let _ = write!(stdout, "{delta}");
                    let _ = stdout.flush();
                }
                AgentEvent::AssistantMessage { .. } => {
                    let _ = writeln!(stdout);
                }
                AgentEvent::ToolExecutionStart {
                    tool_name, args, ..
                } => {
                    eprintln!("  → {}({})", tool_name, args);
                }
                AgentEvent::ToolExecutionEnd {
                    tool_name,
                    is_error,
                    ..
                } => {
                    eprintln!(
                        "  ← {} {}",
                        tool_name,
                        if is_error { "error" } else { "ok" }
                    );
                }
                AgentEvent::PermissionDenied { tool_name, reason } => {
                    eprintln!("  ✗ {tool_name} denied: {reason}");
                }
                _ => {}
            }
        }
        let res = handle.await??;
        session.replace_messages(res.messages);
        if let Err(e) = crate::session::save(&app.config_dir, &session) {
            eprintln!("(warning: session save failed: {e})");
        }
    }
    Ok(())
}

/// Returns `false` if the loop should exit (e.g. `/quit`).
fn handle_slash(line: &str, app: &AppConfig, session: &mut Session) -> anyhow::Result<bool> {
    let (cmd, rest) = match line.split_once(' ') {
        Some((c, r)) => (c, r.trim()),
        None => (line, ""),
    };
    match cmd {
        "/quit" | "/exit" => return Ok(false),
        "/help" => {
            eprintln!("/quit /exit          quit pi");
            eprintln!("/help                show this help");
            eprintln!(
                "/reset               clear in-memory transcript (does not delete session file)"
            );
            eprintln!("/model               print current model");
            eprintln!("/tools               list builtin tools");
            eprintln!("/cost                print accumulated cost/usage so far");
            eprintln!("/sessions            list saved sessions");
            eprintln!("/resume <id>         load a saved session by id");
            eprintln!("/session             print current session id");
        }
        "/reset" => {
            *session = Session::new(&app.model);
            eprintln!("(reset; new session id {})", session.id);
        }
        "/model" => {
            eprintln!("model: {} ({})", app.model.name, app.model.provider);
        }
        "/tools" => {
            for t in default_tools() {
                eprintln!("- {}: {}", t.name(), t.description());
            }
        }
        "/cost" => {
            let mut total_in = 0u64;
            let mut total_out = 0u64;
            for m in &session.messages {
                if let Message::Assistant(a) = m {
                    total_in += a.usage.input;
                    total_out += a.usage.output;
                }
            }
            eprintln!("input tokens: {total_in}  output tokens: {total_out}");
        }
        "/sessions" => {
            let summaries = crate::session::list(&app.config_dir)?;
            if summaries.is_empty() {
                eprintln!("(no saved sessions)");
            }
            for s in summaries.iter().take(20) {
                let first = truncate(&s.first_message, 60);
                eprintln!("{}  ({} msgs, {})  {}", s.id, s.turns, s.model, first);
            }
        }
        "/session" => {
            eprintln!("{}", session.id);
        }
        "/resume" => {
            if rest.is_empty() {
                eprintln!("usage: /resume <id>");
            } else {
                match crate::session::load(&app.config_dir, rest) {
                    Ok(s) => {
                        eprintln!("loaded session {} ({} messages)", s.id, s.messages.len());
                        *session = s;
                    }
                    Err(e) => eprintln!("load failed: {e}"),
                }
            }
        }
        other => {
            eprintln!("unknown command: {other} — try /help");
        }
    }
    Ok(true)
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= n {
        s
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}
