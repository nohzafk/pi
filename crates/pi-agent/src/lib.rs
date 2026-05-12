//! `pi-agent` — Agent runtime with tool calling.
//!
//! Rust port of `@earendil-works/pi-agent-core`. Provides:
//! - [`AgentTool`] / [`AgentToolResult`] for defining tools
//! - [`AgentConfig`] for configuring a run
//! - [`run_agent`] — the agent loop equivalent to `agentLoop` in TS
//! - Builtin tools under [`tools`]

pub mod agent_loop;
pub mod tools;
pub mod types;

pub use agent_loop::{run_agent, AgentRun};
pub use types::{tool_def, AgentConfig, AgentEvent, AgentTool, AgentToolResult};
