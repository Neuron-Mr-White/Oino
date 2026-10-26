//! "This session" data (unipi `pages/session.ts` `collectSession`, `pages/context.ts`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const CHARS_PER_TOKEN: f64 = 4.0;

/// Context bucket parts in display order: `(id, label, glyph, tone)`.
pub const PARTS: &[(&str, &str, &str, &str)] = &[
    ("system", "system prompt", "◆", "c8"),
    ("tools", "tool schemas", "≡", "c7"),
    ("summary", "summaries", "§", "c6"),
    ("user", "your prompts", "❯", "info"),
    ("assistant", "replies", "◇", "warning"),
    ("results", "tool results", "▪", "c3"),
    ("other", "other", "·", "muted"),
];

/// What fills the context window.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextBreakdown {
    /// Tokens per part.
    pub parts: BTreeMap<String, f64>,
    /// Biggest tool results still in context: `(tool, tokens)`.
    pub heavy: Vec<(String, f64)>,
    /// Used tokens (pi's count when known).
    pub used: f64,
    /// Window.
    pub window: f64,
    /// `used` is an estimate.
    pub estimated: bool,
}

#[allow(clippy::cast_precision_loss, reason = "char counts")]
fn text_len(content: Option<&Value>) -> f64 {
    match content {
        Some(Value::String(s)) => s.encode_utf16().count() as f64,
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|b| {
                let len = |k: &str| {
                    b.get(k)
                        .and_then(Value::as_str)
                        .map_or(0.0, |s| s.encode_utf16().count() as f64)
                };
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => len("text"),
                    Some("thinking") => len("thinking"),
                    Some("toolCall") => {
                        b.get("arguments")
                            .map_or(2, |a| a.to_string().encode_utf16().count())
                            as f64
                            + 32.0
                    }
                    Some("image") => 1600.0 * CHARS_PER_TOKEN,
                    _ => 0.0,
                }
            })
            .sum(),
        _ => 0.0,
    }
}

/// unipi `contextBreakdown`: chars ÷ 4 per part, scaled to pi's token count when known.
#[must_use]
pub fn context_breakdown(
    system_prompt: &str,
    tool_schemas: f64,
    messages: &[Value],
    used: Option<f64>,
    window: f64,
) -> ContextBreakdown {
    let mut chars: BTreeMap<String, f64> = PARTS
        .iter()
        .map(|(id, ..)| ((*id).to_owned(), 0.0))
        .collect();
    #[allow(clippy::cast_precision_loss, reason = "char counts")]
    let sys = system_prompt.encode_utf16().count() as f64;
    chars.insert("system".into(), sys);
    chars.insert("tools".into(), tool_schemas);
    let mut heavy: BTreeMap<String, f64> = BTreeMap::new();
    let mut add = |k: &str, n: f64| *chars.entry(k.to_owned()).or_default() += n;
    for m in messages {
        let s = |k: &str| m.get(k).and_then(Value::as_str);
        #[allow(clippy::cast_precision_loss, reason = "char counts")]
        let slen = |k: &str| s(k).map_or(0.0, |t| t.encode_utf16().count() as f64);
        match s("role") {
            Some("user") => add("user", text_len(m.get("content"))),
            Some("assistant") => add("assistant", text_len(m.get("content"))),
            Some("toolResult") => {
                let n = text_len(m.get("content"));
                add("results", n);
                *heavy
                    .entry(s("toolName").unwrap_or("tool").to_owned())
                    .or_default() += n;
            }
            Some("bashExecution") => {
                add("results", slen("command") + slen("output"));
                *heavy.entry("bash (!)".to_owned()).or_default() += slen("output");
            }
            Some("compactionSummary" | "branchSummary") => add("summary", slen("summary")),
            Some("custom") => add("other", text_len(m.get("content"))),
            _ => {}
        }
    }
    let est_total = chars.values().sum::<f64>() / CHARS_PER_TOKEN;
    let known = used.filter(|u| *u > 0.0);
    let used_tokens = known.unwrap_or(est_total);
    let k = if est_total > 0.0 {
        used_tokens / est_total
    } else {
        0.0
    };
    let parts = chars
        .into_iter()
        .map(|(id, c)| (id, c / CHARS_PER_TOKEN * k))
        .collect();
    let mut heavy: Vec<(String, f64)> = heavy
        .into_iter()
        .map(|(n, c)| (n, c / CHARS_PER_TOKEN * k))
        .collect();
    heavy.sort_by(|a, b| b.1.total_cmp(&a.1));
    heavy.truncate(3);
    ContextBreakdown {
        parts,
        heavy,
        used: used_tokens,
        window,
        estimated: known.is_none(),
    }
}

/// Session page data (unipi `SessionRaw`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRaw {
    /// Model display name.
    pub model: String,
    /// Provider.
    pub provider: String,
    /// Thinking level.
    pub thinking: String,
    /// Context tokens.
    pub ctx_tokens: Option<f64>,
    /// Window.
    pub ctx_window: f64,
    /// Percent.
    pub ctx_pct: Option<f64>,
    /// Input tokens.
    pub input: f64,
    /// Output tokens.
    pub output: f64,
    /// Cache read.
    pub cache_read: f64,
    /// Cache write.
    pub cache_write: f64,
    /// Cost.
    pub cost: f64,
    /// User prompts.
    pub prompts: u64,
    /// Replies.
    pub replies: u64,
    /// Tool calls.
    pub tool_calls: u64,
    /// Failed tool calls.
    pub tool_errors: u64,
    /// `(name, calls, errors)` by calls.
    pub tools: Vec<(String, u64, u64)>,
    /// Files edited/written.
    pub files: u64,
    /// Compactions.
    pub compactions: u64,
    /// First entry time.
    pub started_at: i64,
    /// Tokens per reply (last 160).
    pub per_reply: Vec<f64>,
    /// Cwd.
    pub cwd: String,
    /// Git branch.
    pub branch: Option<String>,
    /// Session name.
    pub name: Option<String>,
    /// Context bucket.
    pub context: Option<ContextBreakdown>,
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// Walks the branch once (unipi `collectSession`, branch part).
#[must_use]
pub fn collect(branch: &[Value], header_ts: Option<i64>, now: i64) -> SessionRaw {
    let mut r = SessionRaw::default();
    let mut tools: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut call_name: BTreeMap<String, String> = BTreeMap::new();
    let mut files: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in branch {
        if r.started_at == 0
            && let Some(t) = e
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(crate::time::parse_iso_ms)
        {
            r.started_at = t;
        }
        if e.get("type").and_then(Value::as_str) == Some("compaction") {
            r.compactions += 1;
        }
        if e.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(m) = e.get("message") else { continue };
        match m.get("role").and_then(Value::as_str) {
            Some("user") => r.prompts += 1,
            Some("assistant") => {
                if matches!(
                    m.get("stopReason").and_then(Value::as_str),
                    Some("error" | "aborted")
                ) {
                    continue;
                }
                r.replies += 1;
                let u = m.get("usage");
                let g = |k: &str| num(u.and_then(|u| u.get(k)));
                r.input += g("input");
                r.output += g("output");
                r.cache_read += g("cacheRead");
                r.cache_write += g("cacheWrite");
                r.cost += num(u.and_then(|u| u.pointer("/cost/total")));
                r.per_reply
                    .push(g("input") + g("output") + g("cacheWrite") + g("cacheRead"));
                for c in m
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let Some(name) = c
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|_| c.get("type").and_then(Value::as_str) == Some("toolCall"))
                    else {
                        continue;
                    };
                    r.tool_calls += 1;
                    if let Some(id) = c.get("id").and_then(Value::as_str) {
                        call_name.insert(id.to_owned(), name.to_owned());
                    }
                    tools.entry(name.to_owned()).or_default().0 += 1;
                    let path = c
                        .pointer("/arguments/path")
                        .or_else(|| c.pointer("/arguments/file_path"))
                        .and_then(Value::as_str);
                    if let Some(p) = path
                        && matches!(
                            name.to_lowercase().as_str(),
                            "edit" | "write" | "multi_edit"
                        )
                    {
                        files.insert(p.to_owned());
                    }
                }
            }
            Some("toolResult") if m.get("isError").and_then(Value::as_bool) == Some(true) => {
                r.tool_errors += 1;
                let name = m
                    .get("toolName")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        m.get("toolCallId")
                            .and_then(Value::as_str)
                            .and_then(|id| call_name.get(id).cloned())
                    });
                if let Some(t) = name.and_then(|n| tools.get_mut(&n)) {
                    t.1 += 1;
                }
            }
            _ => {}
        }
    }
    if r.started_at == 0 {
        r.started_at = header_ts.filter(|t| *t > 0).unwrap_or(now);
    }
    let mut list: Vec<(String, u64, u64)> =
        tools.into_iter().map(|(n, (c, e))| (n, c, e)).collect();
    list.sort_by_key(|t| std::cmp::Reverse(t.1));
    r.tools = list;
    r.files = files.len() as u64;
    let skip = r.per_reply.len().saturating_sub(160);
    r.per_reply.drain(..skip);
    r
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn counts_replies_tools_errors_files_compactions() {
        let b = vec![
            json!({"type": "message", "timestamp": "2026-01-01T10:00:00.000Z", "message": {"role": "user", "content": "go"}}),
            json!({"type": "message", "message": {"role": "assistant", "content": [
                {"type": "toolCall", "id": "1", "name": "bash", "arguments": {"command": "ls"}},
                {"type": "toolCall", "id": "2", "name": "edit", "arguments": {"path": "a.rs"}}],
                "usage": {"input": 1000, "output": 10, "cost": {"total": 0.12}}}}),
            json!({"type": "message", "message": {"role": "toolResult", "toolCallId": "1", "isError": true, "content": []}}),
            json!({"type": "compaction", "summary": "s", "tokensBefore": 10}),
            json!({"type": "message", "message": {"role": "assistant", "content": [], "usage": {"input": 1000, "cost": {"total": 0.2}}}}),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "aborted", "content": [], "usage": {"input": 99}}}),
        ];
        let r = collect(&b, None, 5);
        assert_eq!(
            (
                r.replies,
                r.prompts,
                r.tool_calls,
                r.tool_errors,
                r.files,
                r.compactions
            ),
            (2, 1, 2, 1, 1, 1)
        );
        assert!((r.input - 2000.0).abs() < 1e-9);
        assert!((r.cost - 0.32).abs() < 1e-9);
        assert!(r.tools.contains(&("bash".to_owned(), 1, 1)));
        assert_eq!(r.started_at, 1_767_261_600_000);
        assert_eq!(collect(&[], Some(7), 5).started_at, 7);
    }

    #[test]
    fn context_bucket_scales_to_known_tokens() {
        let msgs = vec![
            json!({"role": "user", "content": "x".repeat(400)}),
            json!({"role": "toolResult", "toolName": "read", "content": [{"type": "text", "text": "y".repeat(4000)}]}),
            json!({"role": "compactionSummary", "summary": "z".repeat(400)}),
        ];
        let b = context_breakdown(&"s".repeat(800), 400.0, &msgs, Some(3000.0), 200_000.0);
        assert!(!b.estimated);
        assert!((b.used - 3000.0).abs() < 1e-9);
        let total: f64 = b.parts.values().sum();
        assert!((total - 3000.0).abs() < 1e-6);
        assert_eq!(b.heavy[0].0, "read");
        let est = context_breakdown("", 0.0, &msgs, None, 0.0);
        assert!(est.estimated);
        assert!((est.used - 1200.0).abs() < 1e-9);
    }
}
