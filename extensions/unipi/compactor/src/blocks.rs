//! Normalized blocks (unipi `normalize.ts`, `sanitize.ts`, `content.ts`, `filter-noise.ts`,
//! `session/recall-blocks.ts`, `extract/files.ts`).

use serde_json::{Map, Value};

use crate::re::{self, regexes};

regexes! {
    XML_WRAPPER = r"<(system-reminder|ide_opened_file|command-message|context-window-usage)[^>]*>[\s\S]*?</\1>";
}

/// One normalized block. `source` = branch entry index.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// User text.
    User {
        /// Text.
        text: String,
        /// Entry index.
        source: Option<usize>,
    },
    /// Assistant text.
    Assistant {
        /// Text.
        text: String,
        /// Entry index.
        source: Option<usize>,
    },
    /// A tool call.
    ToolCall {
        /// Tool name.
        name: String,
        /// Arguments object.
        args: Map<String, Value>,
        /// Entry index.
        source: Option<usize>,
    },
    /// A tool result.
    ToolResult {
        /// Tool name.
        name: String,
        /// Text.
        text: String,
        /// Error result.
        is_error: bool,
        /// Entry index.
        source: Option<usize>,
    },
    /// Thinking.
    Thinking {
        /// Text.
        text: String,
        /// Entry index.
        source: Option<usize>,
    },
}

impl Block {
    /// `kind` string.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::User { .. } => "user",
            Self::Assistant { .. } => "assistant",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolResult { .. } => "tool_result",
            Self::Thinking { .. } => "thinking",
        }
    }

    /// Entry index.
    #[must_use]
    pub fn source(&self) -> Option<usize> {
        match self {
            Self::User { source, .. }
            | Self::Assistant { source, .. }
            | Self::ToolCall { source, .. }
            | Self::ToolResult { source, .. }
            | Self::Thinking { source, .. } => *source,
        }
    }

    fn set_source(&mut self, i: usize) {
        match self {
            Self::User { source, .. }
            | Self::Assistant { source, .. }
            | Self::ToolCall { source, .. }
            | Self::ToolResult { source, .. }
            | Self::Thinking { source, .. } => *source = Some(i),
        }
    }

    /// Text of text-bearing blocks.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::User { text, .. }
            | Self::Assistant { text, .. }
            | Self::ToolResult { text, .. }
            | Self::Thinking { text, .. } => Some(text),
            Self::ToolCall { .. } => None,
        }
    }

    /// Replaces the text of text-bearing blocks.
    pub fn map_text(&mut self, f: impl FnOnce(&str) -> String) {
        match self {
            Self::User { text, .. }
            | Self::Assistant { text, .. }
            | Self::ToolResult { text, .. }
            | Self::Thinking { text, .. } => *text = f(text),
            Self::ToolCall { .. } => {}
        }
    }
}

/// Character count (JS `.length` stand-in).
#[must_use]
pub fn clen(s: &str) -> usize {
    s.chars().count()
}

/// First `n` chars.
#[must_use]
pub fn take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Last `n` chars.
#[must_use]
pub fn take_last(s: &str, n: usize) -> String {
    let c = clen(s);
    s.chars().skip(c.saturating_sub(n)).collect()
}

/// unipi `sanitize`: strips control chars, collapses whitespace.
#[must_use]
pub fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(*c as u32, 0x00..=0x08 | 0x0b..=0x0c | 0x0e..=0x1f))
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whitespace collapsed to single spaces, trimmed.
#[must_use]
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// unipi `textOf` (content → plain text).
#[must_use]
pub fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|c| {
                let ty = c.get("type").and_then(Value::as_str);
                match ty {
                    Some("text") => c
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    Some("toolCall") => format!(
                        "[toolCall:{}]",
                        c.get("name").and_then(Value::as_str).unwrap_or("undefined")
                    ),
                    Some("thinking") => "[thinking]".to_owned(),
                    Some("image") => format!(
                        "[image:{}]",
                        c.get("mimeType")
                            .and_then(Value::as_str)
                            .unwrap_or("undefined")
                    ),
                    Some(other) => format!("[{other}]"),
                    None => "[unknown]".to_owned(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Path argument of a tool call (unipi `extractPath`).
#[must_use]
pub fn extract_path(args: &Map<String, Value>) -> Option<String> {
    ["file_path", "path", "filePath", "filepath", "file"]
        .iter()
        .find_map(|k| {
            args.get(*k)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
        })
        .map(ToOwned::to_owned)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// pi-ai roles (`user`, `assistant`, `toolResult`).
fn normalize_message(m: &Value, i: usize) -> Vec<Block> {
    let src = Some(i);
    match m.get("role").and_then(Value::as_str) {
        Some("user") => {
            let mut out = Vec::new();
            let text = sanitize(&text_of(m.get("content")));
            if !text.is_empty() {
                out.push(Block::User { text, source: src });
            }
            if let Some(parts) = m.get("content").and_then(Value::as_array) {
                for p in parts {
                    if p.get("type").and_then(Value::as_str) == Some("image") {
                        out.push(Block::User {
                            text: format!("[image: {}]", s(p, "mimeType")),
                            source: src,
                        });
                    }
                }
            }
            if out.is_empty() {
                out.push(Block::User {
                    text: String::new(),
                    source: src,
                });
            }
            out
        }
        Some("toolResult") => vec![Block::ToolResult {
            name: s(m, "toolName"),
            text: sanitize(&text_of(m.get("content"))),
            is_error: m.get("isError") == Some(&Value::Bool(true)),
            source: src,
        }],
        Some("assistant") => match m.get("content") {
            Some(Value::String(t)) => vec![Block::Assistant {
                text: sanitize(t),
                source: src,
            }],
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                    Some("text") => Some(Block::Assistant {
                        text: sanitize(&s(p, "text")),
                        source: src,
                    }),
                    Some("thinking") => Some(Block::Thinking {
                        text: sanitize(&s(p, "thinking")),
                        source: src,
                    }),
                    Some("toolCall") => Some(Block::ToolCall {
                        name: s(p, "name"),
                        args: p
                            .get("arguments")
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default(),
                        source: src,
                    }),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn text_block(user: bool, text: &str, i: usize) -> Vec<Block> {
    let clean = sanitize(text);
    if clean.is_empty() {
        return Vec::new();
    }
    vec![if user {
        Block::User {
            text: clean,
            source: Some(i),
        }
    } else {
        Block::Assistant {
            text: clean,
            source: Some(i),
        }
    }]
}

/// unipi `normalizeAgentMessage`.
#[must_use]
pub fn normalize_agent_message(m: &Value, i: usize) -> Vec<Block> {
    match m.get("role").and_then(Value::as_str) {
        Some("user" | "assistant" | "toolResult") => {
            let mut v = normalize_message(m, i);
            for b in &mut v {
                b.set_source(i);
            }
            v
        }
        Some("bashExecution") => {
            let cmd = format!("$ {}", s(m, "command"));
            let out = s(m, "output");
            let text = [cmd, out]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let failed = m.get("cancelled") == Some(&Value::Bool(true))
                || m.get("exitCode").and_then(Value::as_i64).unwrap_or(0) != 0;
            vec![Block::ToolResult {
                name: "bash".into(),
                text: sanitize(&text),
                is_error: failed,
                source: Some(i),
            }]
        }
        Some("custom") => text_block(
            true,
            &[s(m, "customType"), text_of(m.get("content"))]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            i,
        ),
        Some("branchSummary" | "compactionSummary") => text_block(false, &s(m, "summary"), i),
        _ => Vec::new(),
    }
}

/// unipi `recallBlocksFromSessionEntries`.
#[must_use]
pub fn recall_blocks(entries: &[Value]) -> Vec<Block> {
    entries
        .iter()
        .enumerate()
        .flat_map(|(i, e)| match e.get("type").and_then(Value::as_str) {
            Some("message") => e
                .get("message")
                .map(|m| normalize_agent_message(m, i))
                .unwrap_or_default(),
            Some("custom_message") => text_block(
                true,
                &[s(e, "customType"), text_of(e.get("content"))]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
                i,
            ),
            Some("branch_summary" | "compaction") => text_block(false, &s(e, "summary"), i),
            _ => Vec::new(),
        })
        .collect()
}

const NOISE_TOOLS: &[&str] = &[
    "TodoWrite",
    "TodoRead",
    "ToolSearch",
    "WebSearch",
    "AskUser",
    "ExitSpecMode",
    "GenerateDroid",
];
const NOISE_STRINGS: &[&str] = &[
    "Continue from where you left off.",
    "No response requested.",
    "IMPORTANT: TodoWrite was not called yet.",
];

/// unipi `filterNoise`: drops thinking, noise tools, noise/wrapper-only user blocks.
#[must_use]
pub fn filter_noise(blocks: Vec<Block>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len());
    for b in blocks {
        match &b {
            Block::Thinking { .. } => continue,
            Block::ToolCall { name, .. } | Block::ToolResult { name, .. }
                if NOISE_TOOLS.contains(&name.as_str()) =>
            {
                continue;
            }
            Block::User { text, source } => {
                let t = text.trim();
                if NOISE_STRINGS.iter().any(|n| t.contains(n)) {
                    continue;
                }
                let cleaned = re::replace_all(&XML_WRAPPER, t, "").trim().to_owned();
                if cleaned.is_empty() {
                    continue;
                }
                out.push(Block::User {
                    text: cleaned,
                    source: *source,
                });
                continue;
            }
            _ => {}
        }
        out.push(b);
    }
    out
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
    reason = "test assertions"
)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn all_compile() {
        for (n, r) in all_patterns() {
            assert!(r.is_some(), "{n}");
        }
    }

    #[test]
    fn normalizing() {
        let u = json!({"role": "user", "content": [{"type": "text", "text": "hi\u{1}  there"}, {"type": "image", "mimeType": "image/png"}]});
        assert_eq!(
            normalize_agent_message(&u, 3),
            vec![
                Block::User {
                    text: "hi there [image:image/png]".into(),
                    source: Some(3)
                },
                Block::User {
                    text: "[image: image/png]".into(),
                    source: Some(3)
                }
            ]
        );
        let a = json!({"role": "assistant", "content": [{"type": "thinking", "thinking": "hm"}, {"type": "toolCall", "name": "read", "arguments": {"path": "a"}}]});
        let b = normalize_agent_message(&a, 1);
        assert_eq!(b[0].kind(), "thinking");
        assert!(matches!(&b[1], Block::ToolCall { name, .. } if name == "read"));
        let bash = json!({"role": "bashExecution", "command": "ls", "output": "x", "exitCode": 1});
        assert!(
            matches!(&normalize_agent_message(&bash, 0)[0], Block::ToolResult { is_error: true, text, .. } if text == "$ ls x")
        );
        let entries = vec![
            json!({"type": "custom_message", "customType": "note", "content": "hello"}),
            json!({"type": "compaction", "summary": "sum"}),
            json!({"type": "custom", "data": {}}),
        ];
        let r = recall_blocks(&entries);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text(), Some("note hello"));
        assert_eq!(r[1].source(), Some(1));
    }

    #[test]
    fn noise() {
        let blocks = vec![
            Block::Thinking {
                text: "x".into(),
                source: None,
            },
            Block::ToolCall {
                name: "TodoWrite".into(),
                args: Map::new(),
                source: None,
            },
            Block::User {
                text: "<system-reminder>a</system-reminder>".into(),
                source: None,
            },
            Block::User {
                text: "No response requested.".into(),
                source: None,
            },
            Block::User {
                text: "keep <system-reminder>a</system-reminder> me".into(),
                source: None,
            },
        ];
        let out = filter_noise(blocks);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text(), Some("keep  me"));
        assert_eq!(sanitize("a\r\n\tb"), "a b");
        assert_eq!(
            text_of(Some(&json!([{"type": "toolCall", "name": "x"}]))),
            "[toolCall:x]"
        );
    }
}
