use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};

use crate::types::{AgentTool, AgentToolResult};

pub struct BashTool {
    cwd: Mutex<PathBuf>,
}

impl BashTool {
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            cwd: Mutex::new(cwd),
        }
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn requires_permission(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Run a shell command via `bash -lc <cmd>`. Returns combined stdout/stderr and exit code, and `cd <path>` to change persistent cwd."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout_ms": {"type": "integer", "default": 120000}
            },
            "required": ["command"]
        })
    }
    async fn execute(&self, _id: &str, args: Value) -> Result<AgentToolResult, String> {
        let cmd = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("missing 'command'")?;
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000);

        let trimmed = cmd.trim();
        if let Some(rest) = trimmed.strip_prefix("cd ") {
            let target = rest.trim();
            if !target.is_empty() {
                let mut guard = self.cwd.lock().await;
                let candidate = PathBuf::from(target);
                let joined = if candidate.is_absolute() {
                    candidate
                } else {
                    guard.join(&candidate)
                };
                let resolved = joined.canonicalize().unwrap_or(joined);
                *guard = resolved.clone();
                return Ok(AgentToolResult::text(format!(
                    "(cwd → {})",
                    resolved.display()
                )));
            }
        }

        let cwd_snapshot = { self.cwd.lock().await.clone() };

        let mut child = Command::new("bash")
            .arg("-lc")
            .arg(cmd)
            .current_dir(&cwd_snapshot)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to capture stderr".to_string())?;

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let tx_out = tx.clone();
        let tx_err = tx.clone();
        drop(tx);

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_out.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_err.send(format!("[stderr] {line}")).is_err() {
                    break;
                }
            }
        });

        let combined = Arc::new(Mutex::new(String::new()));
        let combined_collector = combined.clone();
        let collector = tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let mut buf = combined_collector.lock().await;
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
        });

        let status = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
            Ok(Ok(s)) => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                let _ = collector.await;
                s
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                let _ = collector.await;
                return Err(format!("wait: {e}"));
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                let _ = collector.await;
                return Err(format!("command timed out after {timeout_ms}ms"));
            }
        };

        let code = status.code().unwrap_or(-1);
        let mut out = combined.lock().await.clone();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("[exit {code}]"));
        Ok(AgentToolResult::text(out))
    }
}
