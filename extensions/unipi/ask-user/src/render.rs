//! Transcript rendering of `ask_user` results (unipi `renderAskResult`, `askRows`) as OUS.
//!
//! ```text
//!  ● Asked user 3 questions          ● Asked user Coffee or tea?
//!  │ Planet: Jupiter moon            └ Tea
//!  └ Last book: Dune
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::questions::{Answer, AskQuestion, answer_summary};

/// Result outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Answered.
    Answered,
    /// "Not ready".
    Clarify,
    /// Esc.
    Cancelled,
    /// `end_turn` / `new_session` option.
    Action,
    /// Not shown.
    Unavailable,
}

/// Why the dialog was not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reason {
    /// Turned off in settings.
    Disabled,
    /// No interactive UI.
    NoUi,
}

/// An attachment summary in the details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// Token number.
    pub id: u32,
    /// `image` | `file`.
    pub kind: String,
    /// Path (when known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Tool result details (unipi `AskDetails`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Details {
    /// Questions as shown.
    pub questions: Vec<AskQuestion>,
    /// Answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<Vec<Answer>>,
    /// Outcome.
    pub outcome: Outcome,
    /// Why unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    /// Legacy single-question field (compactor, notify).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// Attachments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<AttachmentInfo>>,
}

fn unavailable_text(reason: Option<Reason>) -> &'static str {
    match reason {
        Some(Reason::Disabled) => "not shown — ask_user is turned off in settings",
        Some(Reason::NoUi) => "not shown — no interactive UI",
        None => "not shown (no interactive UI or turned off)",
    }
}

/// Status row for non-answer outcomes: `(text, tone)`.
fn status_row(d: &Details) -> Option<(&'static str, &'static str)> {
    match d.outcome {
        Outcome::Cancelled => Some(("Canceled by the user", "error")),
        Outcome::Clarify => Some(("Not ready to answer — wants to clarify first", "warning")),
        Outcome::Unavailable => Some((unavailable_text(d.reason), "muted")),
        Outcome::Answered | Outcome::Action => None,
    }
}

/// The full tree (regular/advanced density).
#[must_use]
pub fn render_result(d: &Details) -> Value {
    if d.questions.is_empty() {
        return json!({"t": "empty"});
    }
    let single = d.questions.len() == 1;
    let title: Value = if single {
        json!(
            d.questions[0]
                .question
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        json!([{"text": d.questions.len().to_string(), "dim": true}, {"text": " questions"}])
    };
    let failed = matches!(
        d.outcome,
        Outcome::Cancelled | Outcome::Clarify | Outcome::Unavailable
    );
    let bullet = if failed { "error" } else { "success" };
    let mut head = vec![
        json!({"text": " "}),
        json!({"text": "●", "tone": bullet}),
        json!({"text": " Asked user "}),
    ];
    match title {
        Value::Array(spans) => head.extend(spans),
        other => head.push(json!({"text": other})),
    }
    let rows: Vec<(String, &str)> = match status_row(d) {
        Some((text, tone)) => vec![(text.to_owned(), tone)],
        None => d
            .questions
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let a = d.answers.as_ref().and_then(|a| a.get(i));
                let text = answer_summary(q, a);
                (
                    if single {
                        text
                    } else {
                        format!("{}: {text}", q.header)
                    },
                    "muted",
                )
            })
            .collect(),
    };
    let n = rows.len();
    let mut lines = vec![json!({"t": "text", "value": head, "wrap": false})];
    for (i, (text, tone)) in rows.into_iter().enumerate() {
        let rail = if i + 1 == n { "└" } else { "│" };
        lines.push(json!({"t": "text", "wrap": false, "value": [
            {"text": " "}, {"text": rail, "tone": "muted"}, {"text": " "}, {"text": text, "tone": tone},
        ]}));
    }
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": lines})
}

/// Compact rows for the simple density (unipi `askRows`).
#[must_use]
pub fn ask_rows(d: &Details) -> Vec<String> {
    if d.questions.is_empty() {
        return Vec::new();
    }
    if let Some((text, _)) = status_row(d) {
        return vec![format!("   {text}")];
    }
    if d.outcome == Outcome::Action {
        return vec!["   picked an action".to_owned()];
    }
    d.questions
        .iter()
        .enumerate()
        .map(|(i, q)| {
            format!(
                "   {}: {}",
                q.header,
                answer_summary(q, d.answers.as_ref().and_then(|a| a.get(i)))
            )
        })
        .collect()
}

/// Renderer entry point for a ui-protocol `render` request on the `ask_user` tool.
#[must_use]
pub fn render_request(request: &Value) -> Option<Value> {
    let data = request.get("data")?;
    let Some(details) = data
        .pointer("/result/details")
        .or_else(|| data.pointer("/partialResult/details"))
        .and_then(|d| serde_json::from_value::<Details>(d.clone()).ok())
    else {
        // The call itself renders as nothing (unipi `renderCall` = empty text).
        return Some(json!({"t": "text", "value": ""}));
    };
    if request.get("density").and_then(Value::as_str) == Some("simple") {
        let rows = ask_rows(&details);
        return Some(
            json!({"t": "stack", "direction": "vertical", "gap": 0, "children":
            rows.into_iter().map(|r| json!({"t": "text", "value": [{"text": r, "tone": "muted"}], "wrap": false})).collect::<Vec<_>>()}),
        );
    }
    Some(render_result(&details))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questions::prepare_args;

    fn plain(node: &Value) -> String {
        node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|line| {
                line["value"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s["text"].as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn details(outcome: Outcome, reason: Option<Reason>) -> Details {
        Details {
            questions: prepare_args(
                &json!({"questions": [{"question": "Q1", "header": "Q1", "options": []}]}),
            ),
            answers: None,
            outcome,
            reason,
            question: None,
            attachments: None,
        }
    }

    #[test]
    fn transcript_tree() {
        let qs = prepare_args(&json!({"questions": [
            {"question": "Which planet?", "header": "Planet", "options": [{"label": "Mars"}]},
            {"question": "Foods?", "header": "Foods", "options": [{"label": "Pizza"}]},
            {"question": "Last book?", "header": "Last book", "options": []},
        ]}));
        let d = Details {
            questions: qs,
            answers: Some(vec![
                Answer {
                    selected: vec!["Mars".into()],
                    custom_text: None,
                    skipped: false,
                },
                Answer::skipped(),
                Answer {
                    selected: vec![],
                    custom_text: Some("Dune".into()),
                    skipped: false,
                },
            ]),
            outcome: Outcome::Answered,
            reason: None,
            question: None,
            attachments: None,
        };
        assert_eq!(
            plain(&render_result(&d)),
            " ● Asked user 3 questions\n │ Planet: Mars\n │ Foods: (skipped)\n └ Last book: Dune"
        );
    }

    #[test]
    fn reason_texts() {
        assert_eq!(
            ask_rows(&details(Outcome::Unavailable, Some(Reason::Disabled))),
            ["   not shown — ask_user is turned off in settings"]
        );
        assert_eq!(
            ask_rows(&details(Outcome::Unavailable, Some(Reason::NoUi))),
            ["   not shown — no interactive UI"]
        );
        assert_eq!(
            ask_rows(&details(Outcome::Unavailable, None)),
            ["   not shown (no interactive UI or turned off)"]
        );
        assert_eq!(
            ask_rows(&details(Outcome::Cancelled, None)),
            ["   Canceled by the user"]
        );
        assert_eq!(
            ask_rows(&details(Outcome::Action, None)),
            ["   picked an action"]
        );
        let tree = plain(&render_result(&details(
            Outcome::Unavailable,
            Some(Reason::Disabled),
        )));
        assert!(
            tree.contains("● Asked user Q1\n └ not shown — ask_user is turned off in settings")
        );
        let v = serde_json::to_value(details(Outcome::Unavailable, Some(Reason::NoUi)))
            .unwrap_or_default();
        assert_eq!(v["reason"], "no-ui");
        assert_eq!(v["outcome"], "unavailable");
    }

    #[test]
    fn render_request_densities() {
        let d = serde_json::to_value(details(Outcome::Cancelled, None)).unwrap_or_default();
        let simple = render_request(
            &json!({"density": "simple", "data": {"result": {"details": d.clone()}}}),
        )
        .unwrap_or_default();
        assert_eq!(
            simple["children"][0]["value"][0]["text"],
            "   Canceled by the user"
        );
        let full =
            render_request(&json!({"density": "regular", "data": {"result": {"details": d}}}))
                .unwrap_or_default();
        assert!(plain(&full).contains("Canceled by the user"));
        assert_eq!(
            render_request(&json!({"data": {"toolCall": {}}})),
            Some(json!({"t": "text", "value": ""}))
        );
    }
}
