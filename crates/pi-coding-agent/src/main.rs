//! `pi` — interactive coding agent CLI.
//!
//! Rust port of `packages/coding-agent`. Supports two modes:
//!   * one-shot `pi -p "prompt"` (print mode)
//!   * interactive REPL when no `-p` is given.

mod config;
mod interactive;
mod print_mode;
mod system_prompt;

use clap::Parser;

use crate::config::AppConfig;

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

    let mut app = AppConfig::default();
    app.max_turns = cli.max_turns;

    match cli.prompt {
        Some(p) => print_mode::run_print(&app, p).await,
        None => interactive::run_interactive(&app).await,
    }
}
