# Roadmap

This roadmap tracks work to bring the Rust port closer to feature parity with
the upstream TypeScript [`earendil-works/pi`](https://github.com/earendil-works/pi)
while staying idiomatic Rust. Items are ordered roughly by priority and value
to the agent loop; mark `[x]` when shipped.

## Milestone 1 — Provider parity for streaming (target: 0.2.0)

The current providers issue one POST and replay the response as a single
`Done` event. The `AssistantMessageEventStream` interface is already in place,
so the work is wire-format only.

- [ ] **SSE parsing for Anthropic Messages** (`stream: true`) — emit
      `text_delta` / `thinking_delta` / `toolcall_delta` as they arrive.
- [ ] **SSE parsing for OpenAI Chat Completions** (`stream: true`,
      `stream_options.include_usage: true`).
- [ ] Surface `Usage` deltas; aggregate the final usage into the
      `AssistantMessage`.
- [ ] Cancellation: thread an `AbortHandle` / `tokio::select!` through the
      stream so callers can cancel mid-response.
- [ ] Retry policy with exponential back-off and `Retry-After` honoring
      (matches `maxRetryDelayMs` in TS).

## Milestone 2 — Coding-agent UX (target: 0.3.0)

Bring the interactive experience closer to the original CLI.

- [ ] **Streaming render in the REPL** — print text deltas as they arrive,
      flush tool-call previews live.
- [ ] **Session persistence** — JSON transcripts under
      `$XDG_CONFIG_HOME/pi/sessions/<id>.json`; `pi --resume <id>` and
      `pi sessions list`.
- [ ] **`AGENTS.md` / project-prompt loading** — concatenate any
      `AGENTS.md` / `CLAUDE.md` in the cwd into the system prompt, mirroring
      `packages/coding-agent/src/core/resource-loader.ts`.
- [ ] **Slash commands**: `/clear`, `/cost`, `/compact`, `/tools`, `/sessions`.
- [ ] **Print-mode JSON output** (`-p --json`) — emit structured events for
      scripting.
- [ ] **Config file** at `$XDG_CONFIG_HOME/pi/config.toml` (default model,
      thinking level, tool allow-list).

## Milestone 3 — Tool ecosystem (target: 0.4.0)

- [ ] **Per-call permission prompts** — interactive confirm for `bash`,
      `write`, `edit` (configurable per-tool).
- [ ] **`bash` improvements**: streamed stdout/stderr, configurable cwd,
      env-var passthrough, working-dir persistence across calls.
- [ ] **`edit` polish**: unified-diff preview before write; respect existing
      file encoding and line endings.
- [ ] **`read` polish**: byte-range mode, image read with auto base64 →
      `Content::Image`.
- [ ] **`grep` upgrade**: regex mode (`fancy-regex`), `--files-with-matches`,
      `--context` lines — parity with `rg` flags.
- [ ] **New tools**: `web_fetch` (HTML→text), `task` (sub-agent dispatch),
      `todo` (in-memory checklist).
- [ ] **MCP (Model Context Protocol) client** — load external tool servers
      configured in `pi/config.toml`.

## Milestone 4 — More providers (target: 0.5.0)

Each provider is a `Provider` impl + a dispatch arm in `stream_simple`.

- [ ] **Google Generative AI / Vertex AI**
- [ ] **OpenAI Responses API** (`openai-responses`) — needed for `o*` and GPT-5
      reasoning-effort knobs.
- [ ] **AWS Bedrock Converse Stream** (Anthropic, Cohere, Mistral on Bedrock).
- [ ] **OpenAI-compatible passthrough** — generic base-URL provider for
      OpenRouter, Together, Groq, Cerebras, Fireworks, DeepSeek, xAI, etc.
      Driven by the existing `Model.base_url` field.
- [ ] **Prompt cache markers** — Anthropic `cache_control`, OpenRouter
      `cache_control`, OpenAI session-id headers.
- [ ] **OAuth flows** for providers that need them (Copilot, Codex).

## Milestone 5 — Reliability and polish (target: 1.0.0)

- [ ] **CI**: GitHub Actions matrix (stable + MSRV, macOS + Linux), `cargo
      fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo
      audit`, `cargo deny`.
- [ ] **MSRV**: declare and document (target: `1.80`).
- [ ] **Release pipeline**: `cargo dist` or similar — pre-built binaries for
      macOS (arm64/x86_64) and Linux (gnu/musl) per tag.
- [ ] **Crate publishing** of `pi-ai` and `pi-agent` to crates.io so the
      runtime can be reused by other agents.
- [ ] **Structured tracing** with span-aware logs (`tracing` is wired up;
      add spans around turns and tool calls).
- [ ] **Error model**: replace `String` errors in `pi-agent` with a typed
      `AgentError` enum so callers can branch.
- [ ] **Documentation site** under `docs/` (mdBook): architecture overview,
      provider-author guide, tool-author guide.

## Out of scope (for now)

- Porting `@earendil-works/pi-tui` — a TS-specific differential renderer.
  Rust users get more leverage from `ratatui` if/when a TUI is built.
- Porting `@earendil-works/pi-web-ui` — browser components are inherently
  TS/DOM; a Rust port would need WASM and is a separate project.
- Full sandbox parity with `@anthropic-ai/sandbox-runtime`. The simplest
  workable replacement is per-tool permission prompts plus an opt-in
  `--sandbox` mode that runs `bash` inside an isolated container/jail.

## Non-goals

- One-to-one type compatibility with the TS types (we are idiomatic Rust,
  not a transliteration).
- Bug-for-bug compat with TS provider quirks. We track upstream behavior
  but only port quirks when they affect real-world model output.

## Contributing

Pick any unchecked item, open an issue with the milestone tag, and submit
a PR. Each milestone is independently shippable — there is no hard ordering
between milestones, only within them.
