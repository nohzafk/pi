# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] — 2026-05-12

The first stable release of the Rust port. Everything in milestones 0.2.0
through 1.0.0 of the original ROADMAP has shipped.

### Added

- **`pi-ai`**
  - Server-Sent Events streaming for **Anthropic Messages**: per-block
    `text_delta` / `thinking_delta` / `toolcall_delta` events, `Usage`
    aggregation, and stop-reason mapping.
  - SSE streaming for **OpenAI Chat Completions** with `include_usage`,
    tool-call assembly across multiple deltas, and `[DONE]` handling.
  - **Google Generative AI** provider (`google-generative-ai`) targeting
    Gemini's `streamGenerateContent?alt=sse` endpoint.
  - **OpenAI-compatible passthrough**: any base URL (OpenRouter, Groq,
    Together, Cerebras, DeepSeek, Fireworks, xAI, …) works through the
    OpenAI Chat Completions provider via `Model::openai_compat()` or
    `StreamOptions::base_url`.
  - Shared retry helper with exponential back-off and `Retry-After` parsing;
    classifies 429 / 5xx as retry-worthy.
  - Cancellation through the agent loop and SSE reader via
    `StreamOptions::cancel: CancellationToken`.
  - Custom request headers via `StreamOptions::headers`.

- **`pi-agent`**
  - Typed `AgentError` enum (replaces `String` errors) plus
    `#[tracing::instrument]` spans around each turn.
  - **Permission policy hook** — `PermissionPolicy` trait with
    `Allow` / `AllowSession` / `Deny` decisions; tools advertise
    `requires_permission()`. Defaults: `bash` / `write` / `edit` gated;
    read-only tools open.
  - Streaming `AgentEvent::TextDelta` and `ThinkingDelta` events for
    incremental UI rendering.
  - `run_agent_with_history` entry point for resumed sessions.
  - New tools: **`web_fetch`** (HTTP GET → coarse-text extraction) and
    **`todo`** (in-memory checklist).

- **`pi-coding-agent` (binary `pi`)**
  - Streaming REPL render — assistant text prints as it arrives.
  - **Session persistence** under `$XDG_CONFIG_HOME/pi/sessions/<id>.json`.
    Sessions are saved after every turn and listable / loadable.
    Subcommands: `pi sessions list | show <id> | delete <id>`.
    Flag: `pi --resume <id>` resumes interactively.
  - **AGENTS.md / CLAUDE.md / `.pi/instructions.md`** loader walks up from
    `cwd` and concatenates each file into the system prompt.
  - New slash commands: `/help /reset /model /tools /cost /sessions
    /resume <id> /session /quit /exit`.
  - Interactive per-tool permission prompt; `--yolo` to skip.

- **Tooling**
  - GitHub Actions `ci.yml`: matrix of macOS + Linux × {stable, 1.80}, runs
    `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy
    -- -D warnings`.
  - GitHub Actions `release.yml`: builds 4 release binaries
    (`aarch64-apple-darwin`, `x86_64-apple-darwin`,
    `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`) and attaches
    them to a GitHub Release on every `v*` tag.
  - `rust-toolchain.toml` pins the toolchain channel and components.
  - Workspace declares MSRV 1.80.

### Changed

- All HTTP-backed providers now stream by default; the previous
  single-POST/replayed-as-Done behavior is gone.
- Workspace version bumped to **1.0.0**.

### Not in 1.0 (future milestones)

- AWS Bedrock and OpenAI Responses API
- Prompt-caching markers and OAuth flows
- MCP (Model Context Protocol) client
- TUI / browser UIs (out of scope)
