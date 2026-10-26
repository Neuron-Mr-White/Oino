//! Transcript rendering (unipi `src/render/*`) as render specs (ui-protocol §6.4).
//!
//! | unipi | Oino |
//! |---|---|
//! | `render.style` simple/regular/advanced re-registering read/bash/edit/write | tool renderers for the built-ins ([`tools`]); `regular` defers to the frontend |
//! | harness panels for 10 known custom types | message renderers ([`harness`]) |
//! | delegated-step panel imported by fusion/subagents | service `utility.render-delegated` ([`delegated`]) |
//! | `unipi:attachments` entry renderer | [`crate::attach`] |
//! | user-card patch, transcript spacing, reply background, image collapse, markdown transformer | frontend-owned (host provenance cards, OUS layout) |

pub mod harness;
pub mod parse;
pub mod tools;

use serde_json::{Value, json};

use crate::settings::RenderStyle;

/// Service rendering delegated steps.
pub const DELEGATED_SERVICE: &str = "utility.render-delegated";
/// Service reporting the active style.
pub const STYLE_SERVICE: &str = "utility.render-style";

/// A render request (`{target, key, density, expanded, width, data}`).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// `customMessage` | `customEntry` | `tool`.
    pub target: String,
    /// Custom type or tool name.
    pub key: String,
    /// Expanded.
    pub expanded: bool,
    /// Data.
    pub data: Value,
}

impl Request {
    /// Parses the request JSON.
    #[must_use]
    pub fn parse(v: &Value) -> Self {
        Self {
            target: v
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            key: v
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            expanded: v.get("expanded").and_then(Value::as_bool).unwrap_or(false),
            data: v.get("data").cloned().unwrap_or(Value::Null),
        }
    }
}

/// Delegated-work steps (fusion sidekick, subagent transcripts) as one panel.
pub mod delegated {
    use super::{RenderStyle, Value, json, tools};

    /// One step: `{kind: "tool", name, arg, output, isError, args?}` or `{kind: "text", text}`.
    #[must_use]
    pub fn render(
        steps: &[Value],
        label: Option<&str>,
        style: RenderStyle,
        expanded: bool,
    ) -> Value {
        let mut children = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            match step.get("kind").and_then(Value::as_str) {
                Some("text") => {
                    let text = step.get("text").and_then(Value::as_str).unwrap_or_default();
                    let text = if style == RenderStyle::Simple && !text.starts_with("● ") {
                        format!("● {text}")
                    } else {
                        text.to_owned()
                    };
                    children.push(json!({"t": "markdown", "value": text}));
                }
                Some("tool") => {
                    let name = step.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let arg = step.get("arg").and_then(Value::as_str).unwrap_or_default();
                    let args = step.get("args").cloned().unwrap_or_else(|| {
                        if name == "bash" {
                            json!({"command": arg})
                        } else {
                            json!({"path": arg})
                        }
                    });
                    let output = step
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let failed = step
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let view = tools::ToolView {
                        name: name.to_owned(),
                        args,
                        result: Some(
                            json!({"content": [{"type": "text", "text": output}], "isError": failed}),
                        ),
                        running: false,
                        failed,
                        cwd: String::new(),
                        home: None,
                    };
                    let mut node = if style == RenderStyle::Advanced || expanded {
                        tools::advanced(&view, expanded)
                    } else {
                        tools::simple_row(&view)
                    };
                    let next_is_tool = steps
                        .get(i + 1)
                        .and_then(|s| s.get("kind"))
                        .and_then(Value::as_str)
                        == Some("tool");
                    if node["t"] == "text" && next_is_tool {
                        node["value"][0]["text"] = json!("├ ");
                    }
                    children.push(node);
                }
                _ => {}
            }
        }
        let mut card = json!({"t": "card", "rail": "info", "body": {"t": "stack", "direction": "vertical", "children": children}});
        if let Some(l) = label {
            card["title"] = json!([{"text": format!("◆ {l}"), "tone": "info", "bold": true}]);
        }
        card
    }
}

/// Dispatches a render request (`style` = `utility.render.style`).
#[must_use]
pub fn render(
    req: &Request,
    style: RenderStyle,
    cwd: &str,
    home: Option<&str>,
    preview: bool,
) -> Option<Value> {
    match req.target.as_str() {
        "tool" if tools::BUILTINS.contains(&req.key.as_str()) => tools::render(
            style,
            &tools::ToolView::from_data(&req.key, &req.data, cwd, home),
            req.expanded,
        ),
        "customMessage" if harness::KNOWN.iter().any(|k| k.0 == req.key) => {
            Some(harness::render(&req.key, &req.data, style, req.expanded))
        }
        "customEntry" if req.key == crate::attach::ENTRY => {
            crate::attach::Attachments::render_entry(
                req.data.get("data").unwrap_or(&req.data),
                preview,
            )
        }
        _ => None,
    }
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

    #[test]
    fn dispatch() {
        let tool = Request::parse(
            &json!({"target": "tool", "key": "bash", "expanded": false, "data": {"args": {"command": "ls"}}}),
        );
        assert!(render(&tool, RenderStyle::Simple, "/w", None, true).is_some());
        assert!(render(&tool, RenderStyle::Regular, "/w", None, true).is_none());
        let other = Request {
            key: "web_search".into(),
            ..tool.clone()
        };
        assert!(render(&other, RenderStyle::Simple, "/w", None, true).is_none());
        let msg = Request::parse(
            &json!({"target": "customMessage", "key": "unipi-response", "data": {"content": "x"}}),
        );
        assert_eq!(
            render(&msg, RenderStyle::Simple, "/w", None, true).map(|v| v["t"].clone()),
            Some(json!("card"))
        );
        let entry = Request::parse(
            &json!({"target": "customEntry", "key": "unipi:attachments", "data": {"data": {"items": [{"id": 1, "kind": "file", "name": "a.pdf"}]}}}),
        );
        assert!(render(&entry, RenderStyle::Simple, "/w", None, true).is_some());
    }

    #[test]
    fn delegated_panel() {
        let steps = vec![
            json!({"kind": "tool", "name": "bash", "arg": "npm test", "output": "ok", "isError": false}),
            json!({"kind": "tool", "name": "read", "arg": "a.rs", "output": "x", "isError": false}),
            json!({"kind": "text", "text": "Done."}),
        ];
        let v = delegated::render(&steps, Some("sidekick"), RenderStyle::Simple, false);
        let kids = v["body"]["children"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(kids[0]["value"][0]["text"], "├ ");
        assert_eq!(kids[1]["value"][0]["text"], "└ ");
        assert_eq!(kids[2]["value"], "● Done.");
        assert_eq!(v["title"][0]["text"], "◆ sidekick");
        let adv = delegated::render(&steps, None, RenderStyle::Advanced, false);
        assert_eq!(adv["body"]["children"][0]["t"], "stack");
        assert!(adv.get("title").is_none());
    }
}
