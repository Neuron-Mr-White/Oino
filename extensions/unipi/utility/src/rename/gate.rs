//! Auto-rename gate (unipi `src/rename/gate.ts`): after a confirmed round, decide whether the
//! session deserves a (new) name. It judges the round (the user's message and what the agent
//! did with it):
//!
//! 1. cheap prefilter: a bare greeting / thanks / "ok" round with no tool work never renames;
//! 2. jev, two local questions — `request` (did the round establish a subject?) and, only when
//!    named, `topic` (did it move to a different task?);
//! 3. jev unavailable → rename only an unnamed session after real work (tool calls) or a
//!    ≥ 4-word prompt.

use serde_json::{Map, Value, json};
use unipi_kit::jev::Answers;

use crate::chatter::is_chatter;

/// A round with this many tool calls did real work, whatever the prompt said.
pub const WORK_TOOL_CALLS: usize = 2;
/// Minimum `request: task` confidence.
pub const REQUEST_CONFIDENCE: f64 = 0.6;
/// Minimum `topic: new` confidence.
pub const TOPIC_CONFIDENCE: f64 = 0.7;

/// What the gate sees of a round.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateInput {
    /// The user's prompt.
    pub prompt: String,
    /// Current session name.
    pub current_name: Option<String>,
    /// Earlier prompts (oldest first, may include `prompt`).
    pub earlier: Vec<String>,
    /// The agent's final reply this round (trimmed).
    pub reply: String,
    /// Tool calls the agent made this round.
    pub tool_calls: usize,
}

/// Gate verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Rename.
    pub rename: bool,
    /// Why (debug trail).
    pub reason: String,
}

fn verdict(rename: bool, reason: impl Into<String>) -> Decision {
    Decision {
        rename,
        reason: reason.into(),
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Whitespace-separated word count.
#[must_use]
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// jev `(state, questions)` for a round.
#[must_use]
pub fn gate_request(input: &GateInput) -> (String, Value) {
    let earlier: Vec<&String> = input
        .earlier
        .iter()
        .filter(|e| **e != input.prompt)
        .collect();
    let earlier = &earlier[earlier.len().saturating_sub(3)..];
    let mut parts = vec![format!(
        "Latest user message:\n{}",
        clip(&input.prompt, 1500)
    )];
    if !earlier.is_empty() {
        let list: Vec<String> = earlier
            .iter()
            .map(|e| format!("- {}", clip(e, 200)))
            .collect();
        parts.push(format!("Earlier user messages:\n{}", list.join("\n")));
    }
    if !input.reply.is_empty() {
        parts.push(format!(
            "Assistant's reply this round ({} tool calls):\n{}",
            input.tool_calls,
            clip(&input.reply, 1200)
        ));
    }
    parts.push(match &input.current_name {
        Some(n) => format!("Current session title: \"{n}\""),
        None => "The session has no title yet.".to_owned(),
    });
    let mut q = Map::new();
    q.insert(
        "request".into(),
        json!({
            "type": "choice",
            "instructions": "Did this round (the latest user message together with the assistant's reply) establish a task or subject that a session title could describe?",
            "criteria": {
                "task": "The user asked for work, a change, an answer, a review or an investigation — or opened loosely and the assistant's reply shows real work on a definite subject.",
                "chatter": "Only small talk: a greeting, thanks, acknowledgement, approval (yes / ok / go on) or a bare 'continue', and the reply adds no subject of its own.",
            },
        }),
    );
    if let Some(name) = &input.current_name {
        q.insert(
            "topic".into(),
            json!({
                "type": "choice",
                "instructions": format!("Does this round move to a different task than the current title \"{name}\"?"),
                "criteria": {
                    "same": "Same task or area: a follow-up, refinement, fix or next step of what the title describes.",
                    "new": "A different task, feature or subject that the current title no longer describes.",
                },
            }),
        );
    }
    (parts.join("\n\n"), Value::Object(q))
}

/// Rounds that can never name a session, decided without jev.
#[must_use]
pub fn is_idle_round(input: &GateInput) -> bool {
    is_chatter(&input.prompt) && input.tool_calls < WORK_TOOL_CALLS
}

/// The decision from the jev answers (`None` = jev unavailable).
#[must_use]
pub fn decide(input: &GateInput, answers: Option<&Answers>) -> Decision {
    if is_idle_round(input) {
        return verdict(false, "chatter");
    }
    let Some(answers) = answers else {
        if input.current_name.is_some() {
            return verdict(false, "jev unavailable; keeping current name");
        }
        return if word_count(&input.prompt) >= 4 || input.tool_calls >= WORK_TOOL_CALLS {
            verdict(true, "jev unavailable; first substantive round")
        } else {
            verdict(false, "jev unavailable; prompt too short")
        };
    };
    let pick = |id: &str| {
        let a = answers.get(id);
        (
            a.and_then(|a| a.choice.clone()),
            a.and_then(|a| a.confidence).unwrap_or(0.0),
        )
    };
    let (req, req_conf) = pick("request");
    if req.as_deref() != Some("task") || req_conf < REQUEST_CONFIDENCE {
        return verdict(
            false,
            format!(
                "not a request ({} {req_conf:.2})",
                req.as_deref().unwrap_or("?")
            ),
        );
    }
    if input.current_name.is_none() {
        return verdict(true, "first request");
    }
    let (topic, topic_conf) = pick("topic");
    if topic.as_deref() == Some("new") && topic_conf >= TOPIC_CONFIDENCE {
        return verdict(true, format!("topic changed ({topic_conf:.2})"));
    }
    verdict(
        false,
        format!(
            "same topic ({} {topic_conf:.2})",
            topic.as_deref().unwrap_or("?")
        ),
    )
}

/// Cleans a model-proposed title: one line, no quotes, ≤ 6 words / 60 chars.
#[must_use]
pub fn sanitize_name(raw: &str) -> String {
    let line = raw.split('\n').next().unwrap_or_default();
    let line = line
        .trim_start_matches(|c: char| {
            matches!(c, '"' | '\'' | '`' | '*' | '#') || c.is_whitespace()
        })
        .trim_end_matches(|c: char| matches!(c, '"' | '\'' | '`' | '*' | '.') || c.is_whitespace());
    let words: Vec<&str> = line.split_whitespace().take(6).collect();
    clip(&words.join(" "), 60).trim().to_owned()
}

/// Text of the last assistant message of a round (`<summary>` tags dropped, ≤ 1500 chars).
#[must_use]
pub fn last_reply_text(messages: &[Value]) -> String {
    for m in messages.iter().rev() {
        if m.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let text = crate::answer::extract::message_text(m.get("content").unwrap_or(&Value::Null));
        if !text.trim().is_empty() {
            let cleaned = text.replace("<summary>", "").replace("</summary>", "");
            return clip(cleaned.trim(), 1500);
        }
    }
    String::new()
}

/// Tool calls in a round's assistant messages.
#[must_use]
pub fn count_tool_calls(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|m| m.get("content").and_then(Value::as_array))
        .map(|c| {
            c.iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("toolCall"))
                .count()
        })
        .sum()
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
    use unipi_kit::jev::Answer;

    use super::*;

    fn input(prompt: &str, name: Option<&str>) -> GateInput {
        GateInput {
            prompt: prompt.into(),
            current_name: name.map(Into::into),
            ..GateInput::default()
        }
    }

    fn answers(pairs: &[(&str, &str, f64)]) -> Answers {
        pairs
            .iter()
            .map(|(id, c, conf)| {
                (
                    (*id).to_owned(),
                    Answer {
                        choice: Some((*c).into()),
                        confidence: Some(*conf),
                        ..Answer::default()
                    },
                )
            })
            .collect()
    }

    #[test]
    fn never_renames_on_greetings_acks_or_commands() {
        for t in [
            "hi",
            "Hello there!",
            "thanks",
            "ok",
            "go ahead",
            "continue",
            "/unipi:plan",
            "yes.",
        ] {
            assert!(!decide(&input(t, None), None).rename, "{t}");
        }
    }

    #[test]
    fn topic_question_only_when_named() {
        let (_, q) = gate_request(&input("fix lanes", None));
        assert_eq!(
            q.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
            Some(vec!["request".to_owned()])
        );
        let mut named = input("fix lanes", Some("Kanboard Lanes"));
        named.earlier = vec!["earlier ask".into()];
        let (state, q) = gate_request(&named);
        assert!(q.get("topic").is_some() && q.get("request").is_some());
        assert!(state.contains("Current session title: \"Kanboard Lanes\""));
        assert!(state.contains("earlier ask"));
    }

    #[test]
    fn first_request_then_only_confident_topic_change() {
        let task = ("request", "task", 0.9);
        assert!(decide(&input("fix the lane order", None), Some(&answers(&[task]))).rename);
        let named = input("now the image module", Some("Kanboard Lanes"));
        assert!(decide(&named, Some(&answers(&[task, ("topic", "new", 0.85)]))).rename);
        assert!(!decide(&named, Some(&answers(&[task, ("topic", "new", 0.5)]))).rename);
        assert!(!decide(&named, Some(&answers(&[task, ("topic", "same", 0.95)]))).rename);
        assert!(
            !decide(
                &named,
                Some(&answers(&[
                    ("request", "chatter", 0.9),
                    ("topic", "new", 0.9)
                ]))
            )
            .rename
        );
    }

    #[test]
    fn falls_back_without_jev() {
        assert!(decide(&input("fix the lane order please", None), None).rename);
        assert!(!decide(&input("fix lanes", None), None).rename);
        assert!(!decide(&input("a totally different task now", Some("Old")), None).rename);
    }

    #[test]
    fn sanitizes_model_titles() {
        assert_eq!(
            sanitize_name("\"Kanboard Lane Ordering.\"\nextra"),
            "Kanboard Lane Ordering"
        );
        assert_eq!(
            sanitize_name("one two three four five six seven eight"),
            "one two three four five six"
        );
        assert_eq!(sanitize_name("  ## `Title`  "), "Title");
        assert_eq!(sanitize_name(""), "");
    }

    #[test]
    fn vague_opener_with_real_work_can_name() {
        let i = GateInput {
            prompt: "Hi, lets discuss about what we have".into(),
            reply: "Here's the full picture of pi-test: a sandbox project…".into(),
            tool_calls: 6,
            ..GateInput::default()
        };
        assert!(!is_idle_round(&i));
        assert!(
            gate_request(&i)
                .0
                .contains("Assistant's reply this round (6 tool calls)")
        );
        assert!(decide(&i, Some(&answers(&[("request", "task", 0.8)]))).rename);
        assert!(is_idle_round(&input("hi", None)));
        let hi = GateInput {
            prompt: "hi".into(),
            ..i
        };
        assert!(decide(&hi, None).rename, "jev down: tool work still counts");
    }

    #[test]
    fn last_reply_and_tool_calls() {
        let msgs = vec![
            json!({"role": "assistant", "content": [{"type": "toolCall"}, {"type": "toolCall"}]}),
            json!({"role": "toolResult", "content": []}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "<summary>\nDone: overview\n</summary>"}]}),
        ];
        assert_eq!(last_reply_text(&msgs), "Done: overview");
        assert_eq!(count_tool_calls(&msgs), 2);
        assert_eq!(last_reply_text(&[]), "");
    }
}
