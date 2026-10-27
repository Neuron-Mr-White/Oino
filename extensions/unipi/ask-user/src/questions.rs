//! Question shapes and answer texts (unipi `questions.ts`, pure).
//!
//! One `ask_user` call carries 1–4 questions (Devin's shape), read by [`prepare_args`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Questions per call (hard cap).
pub const MAX_QUESTIONS: usize = 4;
/// Header chip length.
pub const HEADER_MAX: usize = 16;

/// Option action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptionAction {
    /// Stop the agent's turn.
    EndTurn,
    /// Hand off to `prefill`.
    NewSession,
}

/// One option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskOption {
    /// Label (1–5 words).
    pub label: String,
    /// What it means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Returned instead of the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Handoff action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<OptionAction>,
    /// Message or /command for `new_session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill: Option<String>,
}

impl AskOption {
    /// Returned value (`value` or `label`).
    #[must_use]
    pub fn value(&self) -> &str {
        self.value.as_deref().unwrap_or(&self.label)
    }
}

/// One question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskQuestion {
    /// Full question.
    pub question: String,
    /// Chip label.
    pub header: String,
    /// Choices.
    pub options: Vec<AskOption>,
    /// Several answers allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_select: Option<bool>,
    /// `Some(false)` removes the "Other" row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<bool>,
}

impl AskQuestion {
    /// Multi-select question.
    #[must_use]
    pub fn multi(&self) -> bool {
        self.multi_select == Some(true)
    }

    /// Offers "Other (type your own)".
    #[must_use]
    pub fn has_other(&self) -> bool {
        self.other != Some(false)
    }
}

/// What the user did with one question.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// Chosen option values.
    pub selected: Vec<String>,
    /// "Other" text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_text: Option<String>,
    /// Not answered.
    pub skipped: bool,
}

impl Answer {
    /// A skipped answer.
    #[must_use]
    pub fn skipped() -> Self {
        Self {
            selected: Vec::new(),
            custom_text: None,
            skipped: true,
        }
    }

    /// Something was selected or typed.
    #[must_use]
    pub fn is_answered(&self) -> bool {
        !self.selected.is_empty()
            || self
                .custom_text
                .as_deref()
                .is_some_and(|t| !t.trim().is_empty())
    }
}

/// Answered check for an optional answer.
#[must_use]
pub fn is_answered(a: Option<&Answer>) -> bool {
    a.is_some_and(Answer::is_answered)
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(other) => other.to_string(),
    }
}

const STOPWORDS: &[&str] = &[
    "what", "which", "who", "how", "should", "would", "could", "does", "do", "you", "your", "the",
    "and", "for", "want", "like", "prefer",
];

/// A short chip label for a question without one (unipi `deriveHeader`).
#[must_use]
pub fn derive_header(question: &str, index: usize) -> String {
    let cleaned: String = question
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() || c == '-' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let pick: Vec<String> = cleaned
        .split_whitespace()
        .filter(|w| w.encode_utf16().count() > 2 && !STOPWORDS.contains(&w.to_lowercase().as_str()))
        .take(2)
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        })
        .collect();
    let header = if pick.is_empty() {
        format!("Question {}", index + 1)
    } else {
        pick.join(" ")
    };
    header.chars().take(HEADER_MAX).collect()
}

fn normalize_option(raw: &Value) -> Option<AskOption> {
    if let Some(s) = raw.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| AskOption {
            label: s.to_owned(),
            description: None,
            value: None,
            action: None,
            prefill: None,
        });
    }
    let o = raw.as_object()?;
    let label = text(
        o.get("label")
            .filter(|v| !v.is_null())
            .or_else(|| o.get("value")),
    );
    let label = label.trim().to_owned();
    if label.is_empty() {
        return None;
    }
    let nonempty = |k: &str| {
        let t = text(o.get(k));
        let t = t.trim();
        (!t.is_empty()).then(|| t.to_owned())
    };
    Some(AskOption {
        description: nonempty("description"),
        value: nonempty("value").filter(|v| *v != label),
        action: match o.get("action").and_then(Value::as_str) {
            Some("end_turn") => Some(OptionAction::EndTurn),
            Some("new_session") => Some(OptionAction::NewSession),
            _ => None,
        },
        prefill: nonempty("prefill"),
        label,
    })
}

fn normalize_question(raw: &Map<String, Value>, index: usize) -> Option<AskQuestion> {
    let question = text(raw.get("question")).trim().to_owned();
    if question.is_empty() {
        return None;
    }
    let options: Vec<AskOption> = raw
        .get("options")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(normalize_option).collect())
        .unwrap_or_default();
    let header = {
        let h = text(raw.get("header")).trim().to_owned();
        let h = if h.is_empty() {
            derive_header(&question, index)
        } else {
            h
        };
        h.chars().take(HEADER_MAX).collect()
    };
    let multi = raw.get("multi_select") == Some(&Value::Bool(true));
    let other = raw.get("other") == Some(&Value::Bool(false));
    Some(AskQuestion {
        other: (other && !options.is_empty()).then_some(false),
        multi_select: multi.then_some(true),
        question,
        header,
        options,
    })
}

/// Tool-call arguments (`{questions: [...]}`) → questions, capped at [`MAX_QUESTIONS`].
#[must_use]
pub fn prepare_args(args: &Value) -> Vec<AskQuestion> {
    let raw: Vec<Map<String, Value>> = args
        .get("questions")
        .and_then(Value::as_array)
        .map(|qs| qs.iter().filter_map(|q| q.as_object().cloned()).collect())
        .unwrap_or_default();
    raw.iter()
        .enumerate()
        .filter_map(|(i, q)| normalize_question(q, i))
        .take(MAX_QUESTIONS)
        .collect()
}

/// `"Mars, Jupiter moon"` — the one-line summary of an answer.
#[must_use]
pub fn answer_summary(q: &AskQuestion, a: Option<&Answer>) -> String {
    let Some(a) = a.filter(|a| a.is_answered()) else {
        return "(skipped)".to_owned();
    };
    let mut parts: Vec<String> = a
        .selected
        .iter()
        .map(|v| {
            q.options
                .iter()
                .find(|o| o.value() == v)
                .map_or_else(|| v.clone(), |o| o.label.clone())
        })
        .collect();
    if let Some(t) = a
        .custom_text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        parts.push(t.to_owned());
    }
    parts.join(", ")
}

/// The tool result the agent reads (unipi `answersText`, Devin's shape).
#[must_use]
pub fn answers_text(questions: &[AskQuestion], answers: &[Answer], images: &[String]) -> String {
    let mut body = Map::new();
    for (i, q) in questions.iter().enumerate() {
        let a = answers.get(i);
        let answered = is_answered(a);
        let mut entry = Map::new();
        entry.insert(
            "selected".into(),
            json!(if answered {
                a.map(|a| a.selected.clone()).unwrap_or_default()
            } else {
                Vec::new()
            }),
        );
        if answered
            && let Some(t) = a
                .and_then(|a| a.custom_text.as_deref())
                .map(str::trim)
                .filter(|t| !t.is_empty())
        {
            entry.insert("custom_text".into(), json!(t));
        }
        entry.insert("skipped".into(), json!(!answered));
        body.insert(q.question.clone(), Value::Object(entry));
    }
    let mut lines = vec![
        "User answered your questions:".to_owned(),
        js_pretty(&Value::Object(body)),
    ];
    if !images.is_empty() {
        lines.push(format!("Attached: {} (shown below).", images.join(", ")));
    }
    if (0..questions.len()).any(|i| !is_answered(answers.get(i))) {
        lines.push(
            "Skipped questions were left unanswered on purpose — don't ask them again unless you must."
                .to_owned(),
        );
    }
    lines.join("\n")
}

/// `JSON.stringify(v, null, 2)`.
#[must_use]
pub fn js_pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// The "help me out" result (unipi `clarifyText`).
#[must_use]
pub fn clarify_text(questions: &[AskQuestion], answers: &[Answer]) -> String {
    let mut lines = vec![
        "The user is not ready to answer and wants to clarify these questions first.".to_owned(),
        "They may have more information, context or questions for you. Start by asking what they would like to clarify, then reformulate the questions if needed.".to_owned(),
        "Questions asked:".to_owned(),
    ];
    for (i, q) in questions.iter().enumerate() {
        lines.push(format!("- \"{}\"", q.question));
        lines.push(if is_answered(answers.get(i)) {
            format!("  Answer so far: {}", answer_summary(q, answers.get(i)))
        } else {
            "  (No answer provided)".to_owned()
        });
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_derives_headers() {
        let raw: Vec<Value> = (0..6)
            .map(|i| json!({"question": format!("Question number {i}?"), "options": []}))
            .collect();
        assert_eq!(prepare_args(&json!({"questions": raw})).len(), 4);
        assert_eq!(
            derive_header("What is your preferred time of day?", 0),
            "Preferred Time"
        );
        assert_eq!(derive_header("Do it?", 2), "Question 3");
        assert_eq!(prepare_args(&json!({"nope": 1})), Vec::<AskQuestion>::new());
        let q = &prepare_args(
            &json!({"questions": [{"question": "x?", "header": "A very very long header", "options": [{"label": "a", "value": "a"}, {"label": " ", "value": ""}, {"value": "v"}], "other": false}]}),
        )[0];
        assert_eq!(q.header.chars().count(), 16);
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.options[0].value, None, "value equal to label is dropped");
        assert_eq!(q.options[1].label, "v");
        let no_opts =
            &prepare_args(&json!({"questions": [{"question": "free?", "other": false}]}))[0];
        assert_eq!(
            no_opts.other, None,
            "a question without options keeps Other"
        );
    }

    #[test]
    fn answer_texts() {
        let q = AskQuestion {
            question: "Which planet?".into(),
            header: "Planet".into(),
            options: vec![AskOption {
                label: "Mars".into(),
                description: None,
                value: Some("mars".into()),
                action: None,
                prefill: None,
            }],
            multi_select: None,
            other: None,
        };
        let a = Answer {
            selected: vec!["mars".into()],
            custom_text: Some(" moon ".into()),
            skipped: false,
        };
        assert_eq!(answer_summary(&q, Some(&a)), "Mars, moon");
        assert_eq!(answer_summary(&q, None), "(skipped)");
        let mut q2 = q.clone();
        q2.question = "Second?".into();
        let text = answers_text(
            &[q.clone(), q2],
            std::slice::from_ref(&a),
            &["[Image #1]".into()],
        );
        assert!(text.starts_with("User answered your questions:\n{\n  \"Which planet?\": {\n    \"selected\": [\n      \"mars\"\n    ],"));
        assert!(text.contains("\"custom_text\": \"moon\""));
        assert!(text.contains("Attached: [Image #1] (shown below)."));
        assert!(text.ends_with("don't ask them again unless you must."));
        let c = clarify_text(&[q], &[a]);
        assert!(c.contains("- \"Which planet?\"\n  Answer so far: Mars, moon"));
    }
}
