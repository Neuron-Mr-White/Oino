//! Harness panels for the known unipi custom message types (unipi `src/render/harness.ts`).
//!
//! unipi drew a violet-railed panel per harness message in three densities. Oino renders the
//! same shapes as an OUS `card` (rail `harness`, `warning` for warnings); body markdown is
//! the message text unchanged. Meta comes from `details.unipiHarness` / `details.oinoProvenance`
//! when present, else a stable per-type fallback (never text detection).

use serde_json::{Value, json};

use crate::settings::RenderStyle;

/// Known harness custom types: `(customType, source, title, warning, delivery)`.
pub const KNOWN: [(&str, &str, &str, bool, &str); 10] = [
    (
        "unipi:lh-continue",
        "Long-horizon",
        "Continuation",
        false,
        "boundary",
    ),
    (
        "unipi:kanboard-continue",
        "Kanboard",
        "Unfinished task",
        false,
        "boundary",
    ),
    (
        "unipi:kanboard-next",
        "Kanboard",
        "Next task",
        false,
        "boundary",
    ),
    (
        "unipi-memory-recall-reminder",
        "Memory",
        "Memory recall",
        false,
        "before_agent_start",
    ),
    (
        "unipi-memory-retro-reminder",
        "Memory",
        "Memory save?",
        false,
        "nextTurn",
    ),
    (
        "unipi-skills-revealed",
        "Skills",
        "Skill reveal",
        false,
        "direct",
    ),
    (
        "unipi:plan-mode-message",
        "Plan mode",
        "Plan mode",
        false,
        "direct",
    ),
    (
        "unipi-watchdog",
        "Watchdog",
        "Warning",
        true,
        "before_agent_start",
    ),
    (
        "compactor-recall",
        "Compactor",
        "Session recall",
        false,
        "direct",
    ),
    ("unipi-response", "Utility", "Response", false, "followUp"),
];

/// Harness meta (unipi `HarnessMessageMeta`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// Stable id.
    pub id: String,
    /// Source module.
    pub source: String,
    /// Title.
    pub title: String,
    /// Synopsis.
    pub synopsis: Option<String>,
    /// Warning severity.
    pub warning: bool,
    /// Delivery.
    pub delivery: String,
    /// Pre-rendered summary rows.
    pub lines: Vec<String>,
}

/// Stable fallback meta for a custom type.
#[must_use]
pub fn fallback(custom_type: &str) -> Meta {
    let known = KNOWN.iter().find(|k| k.0 == custom_type);
    Meta {
        id: format!("hm-fallback-{custom_type}"),
        source: known.map_or(custom_type, |k| k.1).to_owned(),
        title: known.map_or(custom_type, |k| k.2).to_owned(),
        synopsis: None,
        warning: known.is_some_and(|k| k.3),
        delivery: known.map_or("direct", |k| k.4).to_owned(),
        lines: Vec::new(),
    }
}

/// Meta from message details (`unipiHarness` or `oinoProvenance`), else the fallback.
#[must_use]
pub fn read_meta(custom_type: &str, details: Option<&Value>) -> Meta {
    let mut m = fallback(custom_type);
    let Some(d) = details.and_then(|d| d.get("unipiHarness").or_else(|| d.get("oinoProvenance")))
    else {
        return m;
    };
    let s = |k: &str| {
        d.get(k)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
    };
    if let Some(v) = s("id") {
        m.id = v;
    }
    if let Some(v) = s("source") {
        m.source = v;
    }
    if let Some(v) = s("title") {
        m.title = v;
    }
    m.synopsis = s("synopsis");
    if let Some(v) = s("severity") {
        m.warning = v == "warning";
    }
    if let Some(v) = s("delivery") {
        m.delivery = v;
    }
    m.lines = d
        .get("lines")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    m
}

fn dim(text: impl Into<String>) -> Value {
    json!({"t": "text", "value": [{"text": text.into(), "dim": true}]})
}

fn md(text: &str) -> Value {
    json!({"t": "markdown", "value": text})
}

/// The panel for a harness message.
#[must_use]
pub fn panel(content: &str, meta: &Meta, style: RenderStyle, expanded: bool) -> Value {
    let rows = content.lines().count();
    let hint = |n: usize| dim(format!("Expand: full message ({n} lines)"));
    let label = format!(
        "{}UniPi · {} · {}",
        if meta.warning { "⚠ " } else { "" },
        meta.source,
        meta.title
    );
    let synopsis = meta.synopsis.clone().unwrap_or_else(|| meta.title.clone());
    let mut body: Vec<Value> = Vec::new();
    if meta.warning {
        if style != RenderStyle::Simple || expanded {
            body.push(json!({"t": "text", "value": [{"text": synopsis, "tone": "warning"}]}));
        }
        if expanded {
            body.push(md(content));
        } else if style != RenderStyle::Simple {
            body.push(hint(rows));
        }
    } else {
        match style {
            RenderStyle::Simple => {
                body.push(dim(synopsis));
                body.push(if expanded { md(content) } else { hint(rows) });
            }
            RenderStyle::Regular => {
                if expanded {
                    let mut detail = vec![
                        format!("id: {}", meta.id),
                        format!("origin: harness · transport: {}", meta.delivery),
                    ];
                    if let Some(s) = &meta.synopsis {
                        detail.push(format!("synopsis: {s}"));
                    }
                    body.push(dim(detail.join("\n")));
                }
                body.push(md(content));
            }
            RenderStyle::Advanced if !meta.lines.is_empty() => {
                body.extend(meta.lines.iter().map(|l| dim(l.as_str())));
                body.push(if expanded { md(content) } else { hint(rows) });
            }
            RenderStyle::Advanced => {
                if expanded || rows <= 4 {
                    body.push(md(content));
                } else {
                    body.push(md(&content.lines().take(4).collect::<Vec<_>>().join("\n")));
                    body.push(dim(format!(
                        "Expand: full message ({} more lines)",
                        rows - 4
                    )));
                }
                body.push(dim(format!(
                    "origin: harness | model role: user | delivery: {}",
                    meta.delivery
                )));
            }
        }
    }
    json!({
        "t": "card",
        "title": [{"text": label, "tone": if meta.warning { "warning" } else { "harness" }}],
        "rail": if meta.warning { "warning" } else { "harness" },
        "source": format!("UniPi · {}", meta.source),
        "body": {"t": "stack", "direction": "vertical", "children": body},
    })
}

/// Text of message content.
#[must_use]
pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

/// Renders a known custom message.
#[must_use]
pub fn render(custom_type: &str, message: &Value, style: RenderStyle, expanded: bool) -> Value {
    let meta = read_meta(custom_type, message.get("details"));
    panel(
        &content_text(message.get("content").unwrap_or(&Value::Null)),
        &meta,
        style,
        expanded,
    )
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    fn kids(v: &Value) -> Vec<Value> {
        v["body"]["children"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn fallback_meta_is_stable_per_type() {
        let m = fallback("unipi-watchdog");
        assert!(m.warning);
        assert_eq!(
            (m.source.as_str(), m.title.as_str(), m.delivery.as_str()),
            ("Watchdog", "Warning", "before_agent_start")
        );
        assert_eq!(fallback("x").id, "hm-fallback-x");
        assert_eq!(fallback("x").source, "x");
        assert_eq!(KNOWN.len(), 10);
    }

    #[test]
    fn meta_from_details() {
        let d = json!({"unipiHarness": {"id": "hm1", "source": "Goal", "title": "Check", "synopsis": "2 of 5", "severity": "warning", "delivery": "followUp", "lines": ["a"]}});
        let m = read_meta("unipi-response", Some(&d));
        assert_eq!(
            (
                m.id.as_str(),
                m.source.as_str(),
                m.synopsis.as_deref(),
                m.warning,
                m.lines.len()
            ),
            ("hm1", "Goal", Some("2 of 5"), true, 1)
        );
        let p = read_meta(
            "unipi-response",
            Some(&json!({"oinoProvenance": {"source": "Answer"}})),
        );
        assert_eq!(p.source, "Answer");
        assert_eq!(p.title, "Response");
    }

    #[test]
    fn densities() {
        let body = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6";
        let m = fallback("unipi-response");
        let simple = panel(body, &m, RenderStyle::Simple, false);
        assert_eq!(simple["rail"], "harness");
        assert_eq!(kids(&simple).len(), 2);
        assert!(
            kids(&simple)[1]["value"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("6 lines"))
        );
        assert_eq!(
            kids(&panel(body, &m, RenderStyle::Simple, true))[1]["t"],
            "markdown"
        );
        assert_eq!(kids(&panel(body, &m, RenderStyle::Regular, false)).len(), 1);
        assert_eq!(kids(&panel(body, &m, RenderStyle::Regular, true)).len(), 2);
        let adv = kids(&panel(body, &m, RenderStyle::Advanced, false));
        assert!(
            adv[1]["value"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("2 more lines"))
        );
        assert!(
            adv[2]["value"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("delivery: followUp"))
        );
        let w = fallback("unipi-watchdog");
        assert_eq!(
            kids(&panel(body, &w, RenderStyle::Simple, false)).len(),
            0,
            "simple warnings collapse to the label"
        );
        assert_eq!(
            panel(body, &w, RenderStyle::Regular, false)["rail"],
            "warning"
        );
        assert_eq!(kids(&panel(body, &w, RenderStyle::Advanced, true)).len(), 2);
        let mut lines = m.clone();
        lines.lines = vec!["skill-a".into(), "skill-b".into()];
        assert_eq!(
            kids(&panel(body, &lines, RenderStyle::Advanced, false)).len(),
            3
        );
    }

    #[test]
    fn renders_messages() {
        let msg = json!({"customType": "unipi-response", "content": [{"type": "text", "text": "## Cleanup"}]});
        let v = render("unipi-response", &msg, RenderStyle::Regular, false);
        assert_eq!(kids(&v)[0]["value"], "## Cleanup");
        assert_eq!(content_text(&json!("x")), "x");
    }
}
