# Architecture

`pi` is a three-crate workspace. Each layer is independently usable: bring
just `pi-ai` if you want streaming LLM calls, add `pi-agent` for the tool
loop, or install `pi-coding-agent` for the full CLI.

```text
            +----------------------------+
            |   pi-coding-agent (bin)    |
            | print mode | interactive   |
            | session persistence        |
            | permission prompts         |
            | AGENTS.md loader           |
            +--------------+-------------+
                           | AgentConfig + tools + PermissionPolicy
                           v
            +----------------------------+
            |         pi-agent           |
            | run_agent / _with_history  |
            | streaming events           |
            | permission gate            |
            +--------------+-------------+
                           | Context, StreamOptions (incl. CancellationToken)
                           v
            +----------------------------+
            |           pi-ai            |
            | stream_simple -> Provider  |
            |  - AnthropicProvider       |
            |  - OpenAiProvider          |
            |  - GoogleProvider          |
            |  SSE + retry + cancel      |
            +----------------------------+
```

Downward calls only — `pi-ai` knows nothing about agents, and `pi-agent`
knows nothing about the CLI. Each crate is published independently to
crates.io and follows semver.
