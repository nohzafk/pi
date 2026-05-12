//! `pi` — interactive coding agent CLI.

mod config;
mod interactive;
mod permission;
mod print_mode;
mod project;
mod session;
mod system_prompt;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::config::AppConfig;
use crate::permission::{CliPermission, Mode};

#[derive(Parser, Debug)]
#[command(name = "pi", version, about = "Pi coding agent (Rust port)")]
struct Cli {
    /// One-shot prompt — run agent to completion and exit.
    #[arg(short, long)]
    prompt: Option<String>,

    /// Model identifier. Overrides PI_MODEL.
    #[arg(short = 'm', long, env = "PI_MODEL")]
    model: Option<String>,

    /// Maximum agent turns before stopping.
    #[arg(long, default_value_t = 32)]
    max_turns: u32,

    /// Skip permission prompts (DANGEROUS — bash/write/edit run without confirm).
    #[arg(long)]
    yolo: bool,

    /// Resume a saved session by id.
    #[arg(long)]
    resume: Option<String>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Manage saved sessions.
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },
}

#[derive(Subcommand, Debug)]
enum SessionAction {
    /// List saved sessions.
    List,
    /// Show a single session as pretty JSON.
    Show { id: String },
    /// Delete a session by id.
    Delete { id: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    if let Some(m) = &cli.model {
        std::env::set_var("PI_MODEL", m);
    }

    let app = AppConfig {
        max_turns: cli.max_turns,
        ..AppConfig::default()
    };

    if let Some(Cmd::Sessions { action }) = cli.cmd {
        return run_sessions_cmd(&app, action);
    }

    let permission: Arc<dyn pi_agent::PermissionPolicy> = if cli.yolo {
        Arc::new(CliPermission::new(Mode::Yolo))
    } else {
        Arc::new(CliPermission::new(Mode::Interactive))
    };

    match (cli.prompt, cli.resume) {
        (Some(p), _) => print_mode::run_print(&app, p, permission).await,
        (None, resume_id) => {
            let initial = match resume_id {
                Some(id) => match session::load(&app.config_dir, &id) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("warning: failed to load session {id}: {e}");
                        None
                    }
                },
                None => None,
            };
            interactive::run_interactive(&app, permission, initial).await
        }
    }
}

fn run_sessions_cmd(app: &AppConfig, action: SessionAction) -> anyhow::Result<()> {
    match action {
        SessionAction::List => {
            let summaries = session::list(&app.config_dir)?;
            if summaries.is_empty() {
                eprintln!("(no saved sessions)");
                return Ok(());
            }
            for s in summaries {
                let first = s
                    .first_message
                    .replace('\n', " ")
                    .chars()
                    .take(70)
                    .collect::<String>();
                println!("{}\t{}\t{}\t{}", s.id, s.model, s.turns, first);
            }
            Ok(())
        }
        SessionAction::Show { id } => {
            let s = session::load(&app.config_dir, &id)?;
            println!("{}", serde_json::to_string_pretty(&s)?);
            Ok(())
        }
        SessionAction::Delete { id } => {
            let path = session::sessions_dir(&app.config_dir).join(format!("{id}.json"));
            std::fs::remove_file(&path)?;
            eprintln!("deleted {}", path.display());
            Ok(())
        }
    }
}
