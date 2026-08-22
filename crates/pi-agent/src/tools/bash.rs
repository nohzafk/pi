use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration};

use crate::tools::truncate::{
    dump_full_output, truncate_tail, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};
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
        "Run a shell command via `bash -lc <cmd>`. Returns combined stdout/stderr and exit code, and `cd <path>` to change persistent cwd. \
Output keeps the last 2000 lines or 50KB, whichever limit is hit first. When output is truncated the full text is written to a temp file and the path is reported; read or grep that file to see the dropped part."
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
        // 只有整条命令就是一个 cd 时才自己处理。带 && ; | 之类的交给
        // bash —— 我们没资格解析 shell 语法，而猜错的代价是把整条命令
        // 当成路径名，之后每次 spawn 都失败（cwd 是持久状态）。
        if let Some(target) = looks_like_bare_cd(trimmed) {
            let mut guard = self.cwd.lock().await;
            let candidate = PathBuf::from(target);
            let joined = if candidate.is_absolute() {
                candidate
            } else {
                guard.join(&candidate)
            };
            // 不存在就报错。静默接受会把"路径错了"变成"之后全炸"。
            let resolved = joined
                .canonicalize()
                .map_err(|e| format!("cd {}: {e}", joined.display()))?;
            if !resolved.is_dir() {
                return Err(format!("cd {}: not a directory", resolved.display()));
            }
            *guard = resolved.clone();
            return Ok(AgentToolResult::text(format!(
                "(cwd → {})",
                resolved.display()
            )));
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
        let full = combined.lock().await.clone();

        // Truncate what the model sees, but keep the whole thing on disk so
        // the dropped part stays reachable. The footer names the file.
        let t = truncate_tail(&full, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES);
        let dump = if t.truncated() {
            dump_full_output(&full, "bash")
        } else {
            None
        };

        let mut out = t.content.clone();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if t.truncated() {
            let where_ = dump
                .as_ref()
                .map(|p| format!(" Full output: {}", p.display()))
                .unwrap_or_default();
            out.push_str(&format!(
                "[Showing lines {}-{} of {}.{}]\n",
                t.start_line(),
                t.total_lines,
                t.total_lines,
                where_
            ));
        }
        out.push_str(&format!("[exit {code}]"));

        let details = if t.truncated() {
            json!({
                "truncation": {
                    "truncated": true,
                    "truncatedBy": t.truncated_by.map(|b| b.as_str()),
                    "totalLines": t.total_lines,
                    "totalBytes": t.total_bytes,
                    "outputLines": t.output_lines,
                    "outputBytes": t.output_bytes,
                    "maxLines": t.max_lines,
                    "maxBytes": t.max_bytes,
                },
                "fullOutputPath": dump.as_ref().map(|p| p.display().to_string()),
            })
        } else {
            Value::Null
        };

        Ok(AgentToolResult {
            content: vec![pi_ai::Content::text(out)],
            details,
            terminate: false,
        })
    }
}

/// 整条命令是不是一个单纯的 `cd <path>`。
///
/// 只要出现 shell 的控制字符就返回 None —— 交给真正的 bash 去跑。
/// 这个判断宁可保守：漏判只是少一次 cwd 记忆，误判会毁掉整个会话的
/// shell（把整条命令当成目录名，之后每次 spawn 都 ENOENT）。
fn looks_like_bare_cd(cmd: &str) -> Option<&str> {
    let rest = cmd.strip_prefix("cd ")?.trim();
    if rest.is_empty() {
        return None;
    }
    // shell 控制字符：命令不止一个 cd
    const CONTROL: &[&str] = &["&&", "||", ";", "|", "&", "\n", "$(", "`", ">", "<"];
    if CONTROL.iter().any(|c| rest.contains(c)) {
        return None;
    }
    Some(rest)
}

#[cfg(test)]
mod cd_tests {
    use super::looks_like_bare_cd;

    #[test]
    fn bare_cd_is_recognised() {
        assert_eq!(looks_like_bare_cd("cd /tmp"), Some("/tmp"));
        assert_eq!(looks_like_bare_cd("cd  src/lib  "), Some("src/lib"));
    }

    #[test]
    fn compound_commands_go_to_bash() {
        // 这个正是那个 bug 的形状
        assert_eq!(
            looks_like_bare_cd("cd cordis-pi && cargo test render 2>&1 | tail -30"),
            None
        );
        assert_eq!(looks_like_bare_cd("cd /tmp; ls"), None);
        assert_eq!(looks_like_bare_cd("cd $(pwd)"), None);
        assert_eq!(looks_like_bare_cd("cd a > b"), None);
    }

    #[test]
    fn not_a_cd_at_all() {
        assert_eq!(looks_like_bare_cd("ls -la"), None);
        assert_eq!(looks_like_bare_cd("cdk deploy"), None);
        assert_eq!(looks_like_bare_cd("cd"), None);
    }
}
