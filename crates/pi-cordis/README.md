# pi-cordis

Cordis plugin governance for the pi agent harness.

What the agent can do — its tools, its permissions, part of its system
prompt — comes from a plugin configuration that can be changed while it
runs. When a plugin is unloaded, everything it installed is undone,
without the plugin having written any cleanup for it.

Built on [Cordis on Elle](https://github.com/nohzafk/cordis-elle), which
implements the spatiotemporal composability paradigm from Shi, Zhang &
Cui (PKU + DeepSeek-AI, 2026).

## Status

**Stage A.** Cordis supplies the contents of `AgentConfig`: tools,
permission policy, system prompt. The agent loop itself is still pi's
`run_agent` and cannot be replaced — that is stage B.

## Build requirements

This crate depends on Elle by path:

```toml
elle = { path = "../../../elle-cordis-probe/elle" }
```

Adjust it to wherever you have the Elle source. There is no published
crate to depend on yet.

At runtime it needs the cordis-elle repository, passed as a directory:

```rust
pi_cordis::boot("/path/to/cordis-elle", default_tools(), plugin_config)?;
```

Elle's `include` resolves relative to the process working directory, so
the runtime thread changes into that directory on startup.

## Trying it

```sh
cargo run -p pi-cordis --example stage_a --release -- /path/to/cordis-elle
```

Expected output:

```
[test] tools from cordis: 1
[test]   - greet : Greet from the Cordis plugin
[test] plugin returned: hello from plugin, args={"who":"world"}
[test] bash denied: capability 'exec' is withheld by the harness configuration
[test] after unload: :gone
[test] tools after unload: 0
```

## How a plugin looks

A plugin is a Cordis component — a factory that takes config and returns
a function receiving its own fiber:

```lisp
@{:id "toolkit" :url "toolkit.lisp"
  :factory (fn [config]
             (fn [fib]
               ## a tool the agent can call
               ((C :set) fib :greet-handler
                 (fn [id args] (string "hello, args=" args)))
               ## register it in the list the agent reads
               ((C :set) fib :plugin-tools
                 @[@{:name "greet" :class "read" :handler "greet-handler"
                     :description "Greet from the plugin"}])
               ## withhold a capability at the harness level
               ((C :set) fib :denied-classes @["exec"])))
  :config @{}
  :provide @[:greet-handler :plugin-tools :denied-classes]}
```

Every `(C :set)` is a revertible effect. Unloading the plugin removes
all of it — the tool disappears from the agent's toolset and the
capability restriction lifts, with no cleanup code in the plugin.

## The two directions

A plugin system needs both, and only one of them is not enough:

**plugin → agent** (`bridge.rs`) — pi's async tools exposed as Elle
primitives, so a plugin can read files or run commands.

**agent → plugin** (`runtime.rs`) — a tool call dispatched back into
Elle, so the agent can invoke what a plugin registered.

## Why the bridge is three primitives

`pi/read-tool`, `pi/write-tool`, `pi/exec-tool` rather than one generic
`pi/tool`.

Elle infers signals statically, while a tool name is a runtime argument.
One primitive cannot be both `:io` and `:exec`, so `:deny |:exec|` would
have to block every tool call or none of them. Splitting by capability
class lets exec-class tools carry `SIG_EXEC`, and a plugin denied that
capability is stopped at the bridge with the primitive name and
arguments visible to the orchestrator.

The first version declared the bridge `Signal::silent()`. A plugin
denied `:io` could still call bash through it — the capability model was
decorative. This is load-bearing, not an optimisation.

## Threading

The Elle runtime lives on its own thread and is reached over a channel.
`Runtime` is not `Send` or `Sync`, and that follows from the memory
model rather than being an oversight: a fiber's arena is freed as a
whole when the fiber ends, which assumes single-threaded ownership.

Consequence: governance operations are serialised through the channel.
Acceptable here, since loading and unloading plugins is not a hot path.

## License

MIT, same as pi.
