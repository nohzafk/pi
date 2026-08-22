//! Shared truncation for tool output.
//!
//! Two independent limits apply; whichever is hit first wins:
//!   - a line limit (default 2000 lines)
//!   - a byte limit (default 50 KB)
//!
//! Truncation keeps the tail, because the interesting part of a command's
//! output (errors, test counts, exit status) is at the end. Whole lines
//! only: a partial first line is dropped rather than shown cut in half.
//!
//! When output is truncated the caller writes the full text to a temp file
//! and reports the path, so the model can still reach the dropped part with
//! a follow-up `grep` or `read`. Truncation degrades output; it must not
//! destroy it.

use std::path::PathBuf;

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;

/// Which limit forced the truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncatedBy {
    Lines,
    Bytes,
}

impl TruncatedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            TruncatedBy::Lines => "lines",
            TruncatedBy::Bytes => "bytes",
        }
    }
}

/// The outcome of a truncation pass. `None` in `truncated_by` means the
/// content fit and `content` is the original text.
#[derive(Debug, Clone)]
pub struct Truncation {
    pub content: String,
    pub truncated_by: Option<TruncatedBy>,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub output_lines: usize,
    pub output_bytes: usize,
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Truncation {
    pub fn truncated(&self) -> bool {
        self.truncated_by.is_some()
    }

    /// 1-based line number of the first kept line.
    pub fn start_line(&self) -> usize {
        self.total_lines - self.output_lines + 1
    }
}

fn count_lines(s: &str) -> usize {
    if s.is_empty() {
        return 0;
    }
    let n = s.split('\n').count();
    if s.ends_with('\n') {
        n - 1
    } else {
        n
    }
}

/// Keep the tail of `content` within both limits.
pub fn truncate_tail(content: &str, max_lines: usize, max_bytes: usize) -> Truncation {
    let total_lines = count_lines(content);
    let total_bytes = content.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return Truncation {
            content: content.to_string(),
            truncated_by: None,
            total_lines,
            total_bytes,
            output_lines: total_lines,
            output_bytes: total_bytes,
            max_lines,
            max_bytes,
        };
    }

    let lines: Vec<&str> = content.split('\n').collect();
    // A trailing newline yields a final empty element; it is not a line.
    let lines: &[&str] = if content.ends_with('\n') && !lines.is_empty() {
        &lines[..lines.len() - 1]
    } else {
        &lines[..]
    };

    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0usize;
    let mut hit_bytes = false;

    for line in lines.iter().rev() {
        if kept.len() >= max_lines {
            break;
        }
        // +1 for the newline that rejoins this line to the next one.
        let cost = line.len() + 1;
        if bytes + cost > max_bytes && !kept.is_empty() {
            hit_bytes = true;
            break;
        }
        bytes += cost;
        kept.push(line);
    }
    kept.reverse();

    // Both limits can be exceeded at once. Report the one that actually
    // stopped the scan: the byte limit only if it fired before the line
    // budget ran out.
    let truncated_by = if hit_bytes && kept.len() < max_lines {
        TruncatedBy::Bytes
    } else {
        TruncatedBy::Lines
    };

    let out = kept.join("\n");
    Truncation {
        content: out.clone(),
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        output_lines: kept.len(),
        output_bytes: out.len(),
        max_lines,
        max_bytes,
    }
}

/// Write `body` to a temp file so a truncated result stays reachable.
/// Returns `None` on any IO failure — losing the dump must never fail the
/// tool call itself.
pub fn dump_full_output(body: &str, tag: &str) -> Option<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let path = std::env::temp_dir().join(format!("pi-{tag}-{stamp:x}.log"));
    std::fs::write(&path, body).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_short_output_untouched() {
        let t = truncate_tail("a\nb\nc", 10, 1000);
        assert!(!t.truncated());
        assert_eq!(t.content, "a\nb\nc");
        assert_eq!(t.total_lines, 3);
    }

    #[test]
    fn trailing_newline_is_not_a_line() {
        let t = truncate_tail("a\nb\n", 10, 1000);
        assert_eq!(t.total_lines, 2);
        assert!(!t.truncated());
    }

    #[test]
    fn keeps_the_tail_on_line_limit() {
        let body: String = (1..=100).map(|i| format!("line{i}\n")).collect();
        let t = truncate_tail(&body, 10, 1_000_000);
        assert_eq!(t.truncated_by, Some(TruncatedBy::Lines));
        assert_eq!(t.output_lines, 10);
        assert_eq!(t.total_lines, 100);
        assert_eq!(t.start_line(), 91);
        assert!(t.content.starts_with("line91"));
        assert!(t.content.ends_with("line100"));
    }

    #[test]
    fn byte_limit_wins_when_it_fires_first() {
        let body: String = (1..=100).map(|i| format!("{i}{}\n", "x".repeat(100))).collect();
        let t = truncate_tail(&body, 1000, 500);
        assert_eq!(t.truncated_by, Some(TruncatedBy::Bytes));
        assert!(t.output_bytes <= 500);
        assert!(t.output_lines < 100);
    }

    #[test]
    fn never_yields_a_partial_line() {
        let body = format!("{}\n{}\n", "a".repeat(400), "b".repeat(400));
        let t = truncate_tail(&body, 1000, 500);
        // Only the last line fits; it must be whole.
        assert_eq!(t.content, "b".repeat(400));
    }

    #[test]
    fn keeps_one_line_even_when_it_busts_the_byte_limit() {
        // A single line over the limit still comes back whole rather than
        // as an empty result — an empty tool result tells the model nothing.
        let body = "z".repeat(2000);
        let t = truncate_tail(&body, 1000, 100);
        assert_eq!(t.content, body);
    }
}
