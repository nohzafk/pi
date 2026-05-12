# Roadmap

> **Status:** All milestones from 0.2.0 through 1.0.0 have shipped in
> [1.0.0](./CHANGELOG.md#100--2026-05-12). The list below records what was
> delivered and what remains for a future major. New unchecked items are now
> targeted at 1.x / 2.0.

## Milestone 1 — Provider parity for streaming ✅ (delivered in 1.0.0)

- [x] **SSE parsing for Anthropic Messages** (`stream: true`) — emit
      `text_delta` / `thinking_delta` / `toolcall_delta` as they arrive.
- [x] **SSE parsing for OpenAI Chat Completions** (`stream: true`,
      `stream_options.include_usage: true`).
- [x] Surface `Usage` deltas; aggregate the final usage into the
      `AssistantMessage`.
- [x] Cancellation: thread a `CancellationToken` through the stream so
      callers can cancel mid-response.
- [x] Retry policy with exponential back-off and `Retry-After` honoring.

## Milestone 2 — Coding-agent UX ✅ (delivered in 1.0.0)

- [x] Streaming render in the REPL.
- [x] **Session persistence** under `$XDG_CONFIG_HOME/pi/sessions/<id>.json`;
      `pi --resume <id>` and `pi sessions list / show / delete`.
- [x] **`AGENTS.md` / project-prompt loading**.
- [x] Slash commands: `/clear` (as `/reset`), `/cost`, `/tools`, `/sessions`,
      `/resume`, `/session`, `/help`, `/model`, `/quit`, `/exit`.
- [x] **Print-mode JSON output** (`-p --json`) — emit structured events for
      scripting. (delivered in 1.1.0)
- [ ] **Config file** at `$XDG_CONFIG_HOME/pi/config.toml` (default model,
      thinking level, tool allow-list). (1.x)
- [ ] `/compact` (auto-summarize context to free room). (1.x)

## Milestone 3 — Tool ecosystem ✅ (mostly delivered in 1.0.0)

- [x] **Per-call permission prompts** with allow / allow-session / deny.
- [x] **New tools**: `web_fetch`, `todo`.
- [ ] **`bash` improvements**: streamed stdout/stderr, persisted cwd. (1.x)
- [ ] **`edit` polish**: unified-diff preview before write. (1.x)
- [ ] **`grep` upgrade**: regex mode, context lines. (1.x)
- [ ] **MCP (Model Context Protocol) client**. (1.x — major work)

## Milestone 4 — More providers ✅ (mostly delivered in 1.0.0)

- [x] **Google Generative AI / Vertex AI** (Gemini via
      `streamGenerateContent?alt=sse`).
- [x] **OpenAI-compatible passthrough** — `Model::openai_compat(...)` or
      `StreamOptions::base_url` covers OpenRouter, Together, Groq, Cerebras,
      DeepSeek, Fireworks, xAI, etc.
- [ ] **OpenAI Responses API** (`openai-responses`). (1.x)
- [ ] **AWS Bedrock Converse Stream**. (1.x)
- [ ] **Prompt cache markers** — Anthropic `cache_control`, OpenRouter
      `cache_control`, OpenAI session-id headers. (1.x)
- [ ] **OAuth flows** for Copilot, Codex. (1.x)

## Milestone 5 — Reliability and polish ✅ (delivered in 1.0.0)

- [x] **CI**: GitHub Actions matrix (stable + MSRV, macOS + Linux), `cargo
      fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- [x] **MSRV**: declared as `1.80` in workspace and CI.
- [x] **Release pipeline**: pre-built binaries for macOS (arm64/x86_64) and
      Linux (gnu) per tag via `release.yml`.
- [x] **Structured tracing** with `#[instrument]` on the agent loop.
- [x] **Typed error model** (`pi_agent::AgentError` enum).
- [x] **Crate publishing** of `pi-ai` and `pi-agent` to crates.io.
- [ ] **Documentation site** under `docs/` (mdBook). (1.x)

## Beyond 1.0 — out of scope (no plans)

- Porting `@earendil-works/pi-tui` — terminal renderer. Rust users
  get more leverage from `ratatui` if/when a TUI is built.
- Porting `@earendil-works/pi-web-ui` — browser components.
- Full sandbox parity with `@anthropic-ai/sandbox-runtime`. The pi 1.0
  approach is per-tool permission prompts plus `--yolo` to bypass.

## Non-goals

- One-to-one type compatibility with the TS types (we are idiomatic Rust,
  not a transliteration).
- Bug-for-bug compat with TS provider quirks. We track upstream behavior
  but only port quirks when they affect real-world model output.

## Contributing

Pick any unchecked item, open an issue with the milestone tag, and submit
a PR. See [CHANGELOG.md](./CHANGELOG.md) for what shipped where.
