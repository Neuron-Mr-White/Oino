//! jev questions and state texts (unipi `src/bash-prompt.ts`, `index.ts` state text,
//! `src/extract.ts`).

use std::fmt::Write as _;

use serde_json::{Value, json};

use crate::decide::js_round;
use crate::sample::ProcSample;

/// unipi `STATUS`.
#[must_use]
pub fn status_question() -> Value {
    json!({
        "type": "choice",
        "instructions": "Is this long-running tool call progressing?",
        "criteria": {
            "progressing": "Making forward progress or doing expected long work",
            "waiting": "Legitimately waiting (network, build step, sleep, human-scale timer)",
            "stuck": "Hung, deadlocked, waiting on input that will never come, or frozen",
            "looping": "Repeating the same failure/output without progress",
        },
    })
}

/// unipi `STOP`.
#[must_use]
pub fn stop_question() -> Value {
    json!({
        "type": "noul",
        "instructions": "An AI coding agent ran this shell command and is blocked until it exits. Should it be stopped now because it will not finish on its own in reasonable time (hung, deadlocked, waiting for input or a connection that will not come, retrying forever, or a server/follow/watch that never exits)? Answer no if it is plausibly still doing finite work that will end.",
    })
}

/// unipi `EXPECT`.
#[must_use]
pub fn expect_question() -> Value {
    json!({
        "type": "choice",
        "instructions": "Judging from the command alone, how long should this command normally take before it exits by itself?",
        "criteria": {
            "seconds": "Exits within about 30 seconds (quick lookups, small scripts, status checks)",
            "minutes": "A few minutes (tests, builds, installs, small migrations)",
            "long": "Many minutes to hours (big builds, training, large data jobs, explicit long sleeps/timeouts)",
            "never": "Never exits by design (dev server, follow/watch/tail -f, interactive session, daemon)",
        },
    })
}

/// unipi `KIND`.
#[must_use]
pub fn kind_question() -> Value {
    json!({
        "type": "choice",
        "instructions": "Looking at the command and its output so far, what kind of process is this?",
        "criteria": {
            "finite_job": "A one-shot job that should finish and exit by itself",
            "long_lived": "A service, follow/watch or daemon that never exits by design",
            "needs_input": "Waiting for interactive input (a prompt, password, confirmation, editor)",
            "retry_loop": "Retrying or polling the same failing thing over and over",
        },
    })
}

/// Questions for background tasks and other tools (unipi `tick()` inline block).
#[must_use]
pub fn generic_questions() -> Value {
    json!({
        "status": status_question(),
        "persistent": {
            "type": "noul",
            "instructions": "Is this a long-lived service (dev server, file watcher, daemon, log tail) operating normally, i.e. its recent output shows normal activity rather than repeated errors or failures?",
        },
    })
}

/// Questions for a bash check.
#[must_use]
pub fn bash_questions() -> Value {
    json!({"kind": kind_question(), "status": status_question(), "stop": stop_question()})
}

#[allow(clippy::cast_precision_loss, reason = "display")]
fn fmt_b(n: u64) -> String {
    let f = n as f64;
    if f > 1e6 {
        format!("{:.1}MB", f / 1e6)
    } else if f > 1e3 {
        format!("{}KB", js_round(f / 1e3))
    } else {
        format!("{n}B")
    }
}

fn last_cr(s: &str) -> &str {
    s.rsplit('\r').next().unwrap_or(s)
}

/// unipi `bashState`.
#[must_use]
pub fn bash_state(
    command: &str,
    age: f64,
    since_output: f64,
    bytes: u64,
    output: &str,
    sample: Option<&ProcSample>,
    previous: Option<(f64, bool)>,
) -> String {
    let mut timing = format!(
        "Running for: {}s\nSince last new output: {}s\nTotal output so far: {bytes} bytes",
        js_round(age),
        js_round(since_output)
    );
    if let Some((prev_age, changed)) = previous {
        let _ = write!(
            timing,
            "\nOutput changed since previous check ({}s): {}",
            js_round(prev_age),
            if changed { "yes" } else { "no" }
        );
    }
    let process_block = sample.map_or_else(
        || "Process tree: unavailable".to_owned(),
        |s| {
            let rows: Vec<String> = s
                .processes
                .iter()
                .take(10)
                .map(|p| {
                    let name = if p.comm.is_empty() {
                        format!("pid {}", p.pid)
                    } else {
                        p.comm.clone()
                    };
                    let wait = if p.wchan.is_empty() || p.wchan == "0" {
                        "-"
                    } else {
                        &p.wchan
                    };
                    let children = if p.children > 0 {
                        format!(" children={}", p.children)
                    } else {
                        String::new()
                    };
                    format!(
                        "  {name} [{}] wait={wait} cpu={}% io={}/5s{children}",
                        p.state,
                        js_round(p.cpu_pct),
                        fmt_b(p.io)
                    )
                })
                .collect();
            let rows = if rows.is_empty() {
                "  (none)".to_owned()
            } else {
                rows.join("\n")
            };
            format!(
                "Process tree (sampled over 5s):\n{rows}\nTotal: cpu={}% io={}/5s",
                js_round(s.group_totals.cpu_pct),
                fmt_b(s.group_totals.io_total())
            )
        },
    );
    let mut lines: Vec<&str> = output.split('\n').collect();
    let partial = last_cr(lines.pop().unwrap_or_default());
    let start = lines.len().saturating_sub(20);
    let tail = lines[start..]
        .iter()
        .map(|l| last_cr(l))
        .collect::<Vec<_>>()
        .join("\n");
    let unfinished = if partial.is_empty() {
        String::new()
    } else {
        format!(
            "\n[unfinished line, no newline yet]: {}",
            serde_json::to_string(partial).unwrap_or_default()
        )
    };
    let last = if tail.is_empty() && unfinished.is_empty() {
        "(no output yet)".to_owned()
    } else {
        format!("{tail}{unfinished}")
    };
    format!(
        "Tool: bash\nCommand/args: {command}\n{timing}\n{process_block}\nLast output (tail):\n{last}"
    )
}

/// State text for background tasks and other tools.
#[must_use]
pub fn generic_state(
    tool: &str,
    command: &str,
    running_sec: i64,
    since_last_output: i64,
    output_changed: bool,
    tail: &str,
) -> String {
    format!(
        "Tool: {tool}\nCommand/args: {command}\nRunning for: {running_sec}s\nSince last new output: {since_last_output}s\nOutput changed since previous check: {}\nLast output (tail):\n{}",
        if output_changed { "yes" } else { "no" },
        if tail.is_empty() {
            "(no output yet)"
        } else {
            tail
        }
    )
}

/// unipi `extractText` (text parts of an `AgentToolResult`, trailing newline per part dropped).
#[must_use]
pub fn extract_text(partial: &Value) -> String {
    match partial {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .map(|t| t.strip_suffix('\n').unwrap_or(t))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn first_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// unipi `summarizeArgs` (≤ 300 chars).
#[must_use]
pub fn summarize_args(args: &Value) -> String {
    match args {
        Value::Null => String::new(),
        Value::Object(o) => ["command", "url", "query", "prompt", "promptText"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
            .map_or_else(
                || first_chars(&args.to_string(), 300),
                |s| first_chars(s, 300),
            ),
        Value::String(s) => first_chars(s, 300),
        other => first_chars(&other.to_string(), 300),
    }
}

/// Last `n` chars of `s` (unipi `slice(-n)`).
#[must_use]
pub fn last_chars(s: &str, n: usize) -> String {
    let len = s.chars().count();
    s.chars().skip(len.saturating_sub(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_state_prompt() {
        let s = ProcSample::default();
        let first = bash_state(
            "work",
            120.0,
            100.0,
            3,
            "a\roverwritten\nPrompt? ",
            Some(&s),
            None,
        );
        assert!(!first.contains("Output changed"));
        assert!(
            first.contains("Process tree (sampled over 5s):\n  (none)\nTotal: cpu=0% io=0B/5s")
        );
        let second = bash_state(
            "work",
            300.0,
            280.0,
            3,
            "a\roverwritten\nPrompt? ",
            Some(&s),
            Some((120.0, false)),
        );
        assert!(second.contains("Output changed since previous check (120s): no"));
        assert!(second.contains("overwritten\n[unfinished line, no newline yet]: \"Prompt? \""));
        assert!(
            bash_state(
                "work",
                300.0,
                0.0,
                3,
                "updated",
                Some(&s),
                Some((120.0, true))
            )
            .contains("(120s): yes")
        );
        assert!(
            bash_state("w", 1.0, 1.0, 0, "", None, None)
                .ends_with("Process tree: unavailable\nLast output (tail):\n(no output yet)")
        );
    }

    #[test]
    fn extracts_text_and_args() {
        let partial = json!({"content": [
            {"type": "text", "text": "Error: connection refused, retrying\n"},
            {"type": "text", "text": "Error: connection refused, retrying\n"},
        ]});
        assert_eq!(
            extract_text(&partial),
            "Error: connection refused, retrying\nError: connection refused, retrying"
        );
        assert_eq!(extract_text(&json!("raw output")), "raw output");
        assert_eq!(extract_text(&Value::Null), "");
        assert_eq!(extract_text(&json!({})), "");
        assert_eq!(summarize_args(&json!({"command": "ls -la"})), "ls -la");
        assert_eq!(summarize_args(&json!({"x": 1})), "{\"x\":1}");
        assert_eq!(last_chars("abcdef", 3), "def");
        assert!(
            generic_state("bg_run", "c", 5, 2, true, "")
                .contains("previous check: yes\nLast output (tail):\n(no output yet)")
        );
    }
}
