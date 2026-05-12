//! Builtin tools: read, write, edit, bash, ls, grep, glob.
//!
//! Mirrors the core toolset shipped in `packages/coding-agent/src/core/...`.

pub mod read;
pub mod write;
pub mod edit;
pub mod bash;
pub mod ls;
pub mod grep;
pub mod glob_tool;

use std::sync::Arc;

use crate::types::AgentTool;

/// Returns the default suite of builtin tools used by the coding agent.
pub fn default_tools() -> Vec<Arc<dyn AgentTool>> {
    vec![
        Arc::new(read::ReadTool),
        Arc::new(write::WriteTool),
        Arc::new(edit::EditTool),
        Arc::new(bash::BashTool),
        Arc::new(ls::LsTool),
        Arc::new(grep::GrepTool),
        Arc::new(glob_tool::GlobTool),
    ]
}
