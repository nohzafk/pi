//! Project-prompt loading: walks up from `cwd`, gathers `AGENTS.md`,
//! `CLAUDE.md`, and `.pi/instructions.md` files, and concatenates them under
//! visible separators.

use std::path::{Path, PathBuf};

const FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", ".pi/instructions.md"];

/// Load every project prompt fragment we can find, joined with separators.
/// Returns an empty string if there are no fragments.
pub fn load_project_prompt(start: &Path) -> String {
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    for ancestor in start.ancestors() {
        for name in FILES {
            let path = ancestor.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    found.push((path, trimmed.to_string()));
                }
            }
        }
    }
    if found.is_empty() {
        return String::new();
    }
    let mut buf = String::new();
    for (path, content) in found {
        buf.push_str(&format!("\n\n----- {} -----\n", path.display()));
        buf.push_str(&content);
    }
    buf
}
