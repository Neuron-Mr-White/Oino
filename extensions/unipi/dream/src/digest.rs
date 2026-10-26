//! Session digests (unipi `src/digest.ts`): failed tool calls with their recovery and user
//! corrections, secret-scrubbed, one JSON file per struggle session plus `INDEX.json`.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One digest event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestEvent {
    /// `tool_error | user_correction`.
    pub kind: String,
    /// Tool name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Call arguments (clipped JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// First line(s) of the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_head: Option<String>,
    /// End of the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_tail: Option<String>,
    /// Next successful call of the same tool (`null` until found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Value>,
    /// Correction text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// One session's digest.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDigest {
    /// File stem.
    pub session: String,
    /// Header cwd.
    pub cwd: String,
    /// First user request (clipped).
    pub first_request: String,
    /// User turns.
    pub user_turns: u32,
    /// Tool calls.
    pub tool_calls: u32,
    /// Errors.
    pub errors: u32,
    /// Events.
    pub events: Vec<DigestEvent>,
}

fn run_len(c: &[char], from: usize, ok: impl Fn(char) -> bool) -> usize {
    c[from..].iter().take_while(|x| ok(**x)).count()
}

fn alnum(x: char) -> bool {
    x.is_ascii_alphanumeric()
}
fn word_dash(x: char) -> bool {
    x.is_ascii_alphanumeric() || x == '_' || x == '-'
}

/// Length of a secret match at `i` (unipi `SECRET_RE`, alternatives in order).
fn secret_at(c: &[char], i: usize) -> Option<usize> {
    let starts = |p: &str| {
        p.chars()
            .enumerate()
            .all(|(k, ch)| c.get(i + k) == Some(&ch))
    };
    if starts("sk-") {
        let n = run_len(c, i + 3, word_dash);
        if n >= 16 {
            return Some(3 + n);
        }
    }
    if starts("ghp_") {
        let n = run_len(c, i + 4, alnum);
        if n >= 20 {
            return Some(4 + n);
        }
    }
    if starts("xoxb-") || starts("xoxp-") {
        let n = run_len(c, i + 5, |x| x.is_ascii_alphanumeric() || x == '-');
        if n >= 10 {
            return Some(5 + n);
        }
    }
    if starts("AKIA") {
        let n = run_len(c, i + 4, |x| x.is_ascii_digit() || x.is_ascii_uppercase());
        if n >= 16 {
            return Some(20);
        }
    }
    if starts("eyJ") {
        let a = run_len(c, i + 3, word_dash);
        if a >= 20 && c.get(i + 3 + a) == Some(&'.') {
            let b = run_len(c, i + 4 + a, word_dash);
            if b >= 10 {
                return Some(4 + a + b);
            }
        }
        // Backtracking: a shorter head followed by `.` cannot exist (the run stops at `.`).
    }
    let n = run_len(c, i, alnum);
    (n >= 40).then_some(n)
}

/// Replaces secrets with `<redacted>`.
#[must_use]
pub fn scrub(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        if let Some(n) = secret_at(&c, i) {
            out.push_str("<redacted>");
            i += n;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out.replace("9801(*)!Coffee", "<redacted>")
}

fn clip(text: &str, max: usize) -> String {
    let s = scrub(text).split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s
    }
}

/// unipi `CORRECTION_RE`.
#[must_use]
pub fn is_correction(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    let word = |w: &str| {
        t.strip_prefix(w).is_some_and(|r| {
            r.chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    };
    word("no")
        || t.starts_with("wrong")
        || t.starts_with("that's not")
        || t.starts_with("thats not")
        || t.starts_with("not what")
        || word("stop")
        || word("don't")
        || word("dont")
        || t.starts_with("why did you")
        || t.starts_with("you should")
        || t.starts_with("you shouldn't")
        || t.starts_with("you shouldnt")
        || word("actually")
}

/// pi/Oino session dir name for a cwd (`/home/oi` → `--home-oi--`).
#[must_use]
pub fn encode_session_dir_name(cwd: &str) -> String {
    format!(
        "--{}--",
        cwd.split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    )
}

fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::Array(a)) => a
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn tail_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

/// Digests one session's JSONL text; `None` without struggle events.
#[must_use]
pub fn digest_session(name: &str, text: &str) -> Option<SessionDigest> {
    let mut calls: HashMap<String, (String, Value)> = HashMap::new();
    let mut d = SessionDigest {
        session: name.to_owned(),
        ..SessionDigest::default()
    };
    for line in text.lines().filter(|l| !l.is_empty()) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let ty = entry.get("type").and_then(Value::as_str);
        if ty == Some("session") {
            entry
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .clone_into(&mut d.cwd);
        }
        if ty != Some("message") {
            continue;
        }
        let Some(m) = entry.get("message") else {
            continue;
        };
        match m.get("role").and_then(Value::as_str) {
            Some("user") => {
                d.user_turns += 1;
                let text = content_text(m.get("content"));
                if d.first_request.is_empty() {
                    d.first_request = clip(&text, 300);
                }
                if is_correction(&text) {
                    d.events.push(DigestEvent {
                        kind: "user_correction".into(),
                        text: Some(clip(&text, 400)),
                        ..DigestEvent::default()
                    });
                }
            }
            Some("assistant") => {
                for c in m
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if c.get("type").and_then(Value::as_str) == Some("toolCall") {
                        d.tool_calls += 1;
                        let id = c.get("id").map(value_str).unwrap_or_default();
                        calls.insert(
                            id,
                            (
                                c.get("name").map(value_str).unwrap_or_default(),
                                c.get("arguments").cloned().unwrap_or(Value::Null),
                            ),
                        );
                    }
                }
            }
            Some("toolResult") => {
                let id = m.get("toolCallId").map(value_str).unwrap_or_default();
                let (name, args) = calls.get(&id).cloned().unwrap_or_else(|| {
                    (
                        m.get("toolName").map(value_str).unwrap_or_default(),
                        json!({}),
                    )
                });
                let text = content_text(m.get("content"));
                let exit_fail =
                    (1..=9).any(|n| text.contains(&format!("Command exited with code {n}")));
                if m.get("isError") == Some(&Value::Bool(true)) || exit_fail {
                    d.errors += 1;
                    d.events.push(DigestEvent {
                        kind: "tool_error".into(),
                        tool: Some(name),
                        call: Some(clip(&args.to_string(), 300)),
                        error_head: Some(clip(&head_chars(&text, 500), 400)),
                        error_tail: Some(clip(&tail_chars(&text, 400), 300)),
                        recovery: Some(Value::Null),
                        text: None,
                    });
                } else if let Some(open) = d.events.iter_mut().rev().find(|e| {
                    e.kind == "tool_error"
                        && e.tool.as_deref() == Some(name.as_str())
                        && matches!(e.recovery, Some(Value::Null))
                }) {
                    open.recovery = Some(Value::String(clip(&args.to_string(), 300)));
                }
            }
            _ => {}
        }
    }
    (!d.events.is_empty()).then_some(d)
}

fn value_str(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), ToOwned::to_owned)
}

/// Digests every `*.jsonl` in `sessions_dir` into `out_dir`; returns `(sessions, events)`.
#[must_use]
pub fn digest_sessions(sessions_dir: &Path, out_dir: &Path) -> (u32, u32) {
    let _ = std::fs::create_dir_all(out_dir);
    let mut files: Vec<String> = std::fs::read_dir(sessions_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|f| {
                    Path::new(f)
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("jsonl"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let (mut count, mut events) = (0u32, 0u32);
    let mut index = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(sessions_dir.join(&f)) else {
            continue;
        };
        let stem = f.trim_end_matches(".jsonl");
        let Some(d) = digest_session(stem, &text) else {
            continue;
        };
        count += 1;
        events += u32::try_from(d.events.len()).unwrap_or(u32::MAX);
        let _ = std::fs::write(
            out_dir.join(format!("{stem}.json")),
            serde_json::to_string_pretty(&d).unwrap_or_default(),
        );
        index.push(json!({
            "session": d.session,
            "firstRequest": head_chars(&d.first_request, 120),
            "toolCalls": d.tool_calls,
            "errors": d.errors,
            "corrections": d.events.iter().filter(|e| e.kind == "user_correction").count(),
        }));
    }
    let _ = std::fs::write(
        out_dir.join("INDEX.json"),
        serde_json::to_string_pretty(&index).unwrap_or_default(),
    );
    (count, events)
}

/// Session files modified after `since_ms`.
#[must_use]
pub fn count_new_sessions(sessions_dir: &Path, since_ms: i64) -> u32 {
    let Ok(rd) = std::fs::read_dir(sessions_dir) else {
        return 0;
    };
    let n = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
        .filter(|e| {
            e.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .is_some_and(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX) > since_ms)
        })
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    clippy::format_collect,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    fn lines(v: &[Value]) -> String {
        v.iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn sid() -> Value {
        json!({"type": "session", "version": 3, "id": "x", "timestamp": "t", "cwd": "/w/p"})
    }
    fn user(t: &str) -> Value {
        json!({"type": "message", "message": {"role": "user", "content": t}})
    }
    fn call(id: &str, name: &str, args: Value) -> Value {
        json!({"type": "message", "message": {"role": "assistant", "content": [{"type": "toolCall", "id": id, "name": name, "arguments": args}]}})
    }
    fn result(id: &str, name: &str, text: &str, err: bool) -> Value {
        json!({"type": "message", "message": {"role": "toolResult", "toolCallId": id, "toolName": name, "isError": err, "content": [{"type": "text", "text": text}]}})
    }

    #[test]
    fn pairs_error_with_recovery() {
        let d = digest_session(
            "s",
            &lines(&[
                sid(),
                user("fix the build"),
                call("1", "bash", json!({"command": "sed -n \"$(grep -n x f)\""})),
                result(
                    "1",
                    "bash",
                    "sed: unknown command: ',' Command exited with code 1",
                    true,
                ),
                call("2", "bash", json!({"command": "grep -n x f"})),
                result("2", "bash", "3:x", false),
            ]),
        )
        .unwrap_or_default();
        assert_eq!(d.events.len(), 1);
        let e = &d.events[0];
        assert_eq!(e.kind, "tool_error");
        assert!(
            e.error_head
                .as_deref()
                .unwrap_or_default()
                .contains("sed: unknown command")
        );
        assert!(
            e.recovery
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("grep -n x f")
        );
        assert_eq!((d.errors, d.tool_calls, d.cwd.as_str()), (1, 2, "/w/p"));
        let v = serde_json::to_value(&d).unwrap_or_default();
        assert!(v["events"][0].get("errorHead").is_some());
    }

    #[test]
    fn stack_head_and_corrections() {
        let stack: String = (0..60)
            .map(|i| format!("at frame{i} (/deep/node_modules/x/y.js:1:1)\n"))
            .collect();
        let d = digest_session(
            "s",
            &lines(&[
                sid(),
                call("1", "bash", json!({"command": "npx tsx /tmp/s.mts"})),
                result(
                    "1",
                    "bash",
                    &format!("Error: Cannot find module '@earendil-works/pi-tui'\n{stack}")
                        .repeat(3),
                    true,
                ),
            ]),
        )
        .unwrap_or_default();
        let head = d.events[0].error_head.clone().unwrap_or_default();
        assert!(head.contains("Cannot find module '@earendil-works/pi-tui'"));
        assert!(head.chars().count() <= 401);
        let d = digest_session(
            "s",
            &lines(&[
                sid(),
                user("no, wrong — use the token sk-abcdefghij0123456789 for this"),
                call("1", "bash", json!({"command": "deploy"})),
                result("1", "bash", "deployed", false),
            ]),
        )
        .unwrap_or_default();
        let c = d
            .events
            .iter()
            .find(|e| e.kind == "user_correction")
            .and_then(|e| e.text.clone())
            .unwrap_or_default();
        assert!(!c.contains("sk-abcdefghij") && c.contains("<redacted>"));
        assert!(
            digest_session(
                "s",
                &lines(&[
                    sid(),
                    user("hi"),
                    call("1", "read", json!({"path": "x"})),
                    result("1", "read", "ok", false)
                ])
            )
            .is_none()
        );
    }

    #[test]
    fn digest_dir_and_names() {
        let base = std::env::temp_dir().join(format!("dream-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (sessions, out) = (base.join("s"), base.join("o"));
        std::fs::create_dir_all(&sessions).unwrap_or_default();
        let name = "2026-10-09T01-00-00-000Z_01234567";
        std::fs::write(
            sessions.join(format!("{name}.jsonl")),
            lines(&[
                sid(),
                call("1", "bash", json!({"command": "x"})),
                result("1", "bash", "9801(*)!Coffee exploded", true),
            ]),
        )
        .unwrap_or_default();
        assert_eq!(digest_sessions(&sessions, &out), (1, 1));
        let stored: Value = serde_json::from_str(
            &std::fs::read_to_string(out.join(format!("{name}.json"))).unwrap_or_default(),
        )
        .unwrap_or_default();
        assert!(
            stored["events"][0]["errorHead"]
                .as_str()
                .unwrap_or_default()
                .contains("<redacted> exploded")
        );
        assert!(
            std::fs::read_to_string(out.join("INDEX.json"))
                .unwrap_or_default()
                .contains("firstRequest")
        );
        assert_eq!(count_new_sessions(&sessions, 0), 1);
        assert_eq!(count_new_sessions(&sessions, i64::MAX), 0);
        assert_eq!(
            encode_session_dir_name("/home/oi/Projects/Personal/archived/unipi"),
            "--home-oi-Projects-Personal-archived-unipi--"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn scrubbing() {
        assert_eq!(
            scrub("key ghp_abcdefghijklmnopqrstuvwxyz ok"),
            "key <redacted> ok"
        );
        assert_eq!(scrub("AKIAABCDEFGHIJKLMNOP!"), "<redacted>!");
        assert_eq!(scrub(&"a".repeat(45)), "<redacted>");
        assert_eq!(scrub("short abc"), "short abc");
        assert!(is_correction("Actually, use X"));
        assert!(!is_correction("nothing wrong"));
        assert!(!is_correction("notable"));
    }
}
