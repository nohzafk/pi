# pi

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg?logo=rust)](https://www.rust-lang.org/)
[![CI](https://img.shields.io/badge/build-passing-brightgreen.svg)](#tests)

A Rust port of [`earendil-works/pi`](https://github.com/earendil-works/pi) — the
pi agent harness — focused on the core coding-agent loop.

The original is a TypeScript monorepo (~189k LOC) spanning five packages:
`pi-ai`, `pi-agent-core`, `pi-coding-agent`, `pi-tui`, `pi-web-ui`. This port
covers the agent runtime end-to-end and ships a working `pi` CLI that talks to
real Anthropic Messages and OpenAI Chat Completions endpoints.

## Layout

```
pi/
├─ Cargo.toml                       # workspace
└─ crates/
   ├─ pi-ai/                        # ←→ packages/ai
   ├─ pi-agent/                     # ←→ packages/agent
   └─ pi-coding-agent/              # ←→ packages/coding-agent (binary: `pi`)
```

| TS package | Rust crate | Status |
|------------|-----------|--------|
| `@earendil-works/pi-ai` | `pi-ai` | Anthropic Messages + OpenAI Chat Completions. Unified `Message`, `Content`, `Tool`, `AssistantMessageEvent` types; `stream_simple()` entry. Non-streaming requests re-emitted as `Done` events. |
| `@earendil-works/pi-agent-core` | `pi-agent` | `run_agent` loop with sequential tool execution, max-turns limit, event sink. Builtin tools: `read`, `write`, `edit`, `bash`, `ls`, `grep`, `glob`. |
| `@earendil-works/pi-coding-agent` | `pi-coding-agent` | `pi` CLI with one-shot (`-p`) and interactive REPL modes, slash commands `/quit /exit /help /reset /model`. |
| `@earendil-works/pi-tui` | — | Not ported (terminal UI library). |
| `@earendil-works/pi-web-ui` | — | Not ported (browser components). |

## Quick start

```bash
git clone https://github.com/nktkt/pi.git
cd pi
cargo build --release

export ANTHROPIC_API_KEY=sk-ant-...
# or: export OPENAI_API_KEY=sk-...

# One-shot:
./target/release/pi -p "List the files in this directory and summarize them"

# Interactive:
./target/release/pi
```

Pick the model explicitly:

```bash
PI_MODEL=claude-opus-4-7 pi -p "..."
PI_MODEL=gpt-4o-mini     pi -p "..."
```

## Architecture

```
            ┌────────────────────────────┐
            │   pi-coding-agent (bin)    │
            │  print mode | interactive  │
            └──────────────┬─────────────┘
                           │ AgentConfig + tools
                           ▼
            ┌────────────────────────────┐
            │         pi-agent           │
            │  run_agent loop, tools     │
            └──────────────┬─────────────┘
                           │ Context, StreamOptions
                           ▼
            ┌────────────────────────────┐
            │           pi-ai            │
            │ stream_simple → Provider   │
            │  ├─ AnthropicProvider      │
            │  └─ OpenAiProvider         │
            └────────────────────────────┘
```

The agent loop:

1. Push the user prompt onto a `Vec<Message>` transcript.
2. Build a `Context { system_prompt, messages, tools }` and call
   `pi_ai::stream_simple()`.
3. Consume the resulting `AssistantMessageEventStream` until `Done` / `Error`.
4. Append the assistant message; for each `Content::ToolCall`, look up the tool
   in the registry, call `tool.execute()`, and append a `ToolResultMessage`.
5. Repeat until `stop_reason ≠ ToolUse` or `max_turns` is reached.

This is intentionally close to `agentLoop()` in
`packages/agent/src/agent-loop.ts`. Streaming events fire (`Start`, `TextDelta`,
`ToolCallEnd`, etc.) so a future TUI can subscribe to them via the
`mpsc::UnboundedSender<AgentEvent>` already wired into `run_agent`.

## What is *not* ported

Faithful to the goal of getting a working coding agent in Rust, the following
were deliberately left out (they each map to thousands of lines of TS):

- **Provider zoo.** The TS `pi-ai` supports Bedrock, Google Vertex, Azure
  OpenAI, GitHub Copilot, OpenRouter, Vercel AI Gateway, Mistral, Moonshot,
  Groq, Cerebras, Fireworks, Together, xAI, DeepSeek, and more. The Rust port
  starts with Anthropic Messages and OpenAI Chat Completions. Adding a new
  provider is "implement the `Provider` trait + extend `stream_simple`'s
  dispatch."
- **SSE streaming wire-format.** Both providers currently issue a single POST
  and replay the response as a single `Done` event. The public
  `AssistantMessageEventStream` already emits per-content events, so a real SSE
  parser drops in behind the same interface.
- **Prompt caching, OAuth (`pi-ai/oauth.ts`), Bedrock SDK auth, tool-streaming
  beta headers**, OpenAI Responses API, etc.
- **`pi-tui` differential renderer and `pi-web-ui` browser components.**
- **Session persistence, slash-command extensions, package-manager CLI,
  skills/extensions**, telemetry — the TS coding-agent's surface area is large;
  this port keeps a simple line-based interactive REPL.
- **Sandboxing.** The original supports `@anthropic-ai/sandbox-runtime`. The
  Rust `bash` tool runs `bash -lc <cmd>` directly in the host shell.

## Tests

```bash
cargo test
```

Tests cover serialization round-trips for `pi-ai` types and direct execution of
every builtin tool against a tempdir. No network access required.

## License

MIT — same as the upstream project.
