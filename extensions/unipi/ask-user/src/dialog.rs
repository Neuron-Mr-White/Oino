//! The question dialog as a ui-protocol `form` (unipi `ask-ui.ts` → frontend-rendered), the
//! settings that shape it, and the answer decoding (unipi `phoneAnswer` semantics: unknown
//! values dropped, single-select keeps one, missing answers are skipped).

use serde_json::{Value, json};

use crate::questions::{Answer, AskOption, AskQuestion};

/// Esc behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    /// Esc stops the agent's turn.
    Stop,
    /// Esc sends what's answered, the rest skipped.
    Send,
}

/// "Other" row policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherPolicy {
    /// The agent decides (on unless it sets `other: false`).
    Agent,
    /// Always offered.
    Always,
    /// Never offered.
    Never,
}

/// Namespace `ask-user` values (unipi `getAskUserSettings`).
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the unipi settings object"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Tool enabled.
    pub enabled: bool,
    /// Announce prompts (notify).
    pub notify_on_ask: bool,
    /// Esc behaviour.
    pub escape: Escape,
    /// Digits pick and advance.
    pub digit_advance: bool,
    /// "Not ready" line.
    pub help_line: bool,
    /// Other row policy.
    pub other: OtherPolicy,
    /// Questions per call (1–4).
    pub max_questions: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            notify_on_ask: true,
            escape: Escape::Stop,
            digit_advance: true,
            help_line: true,
            other: OtherPolicy::Agent,
            max_questions: 4,
        }
    }
}

impl Settings {
    /// From the resolved namespace object (invalid values → defaults, `maxQuestions` clamped).
    #[must_use]
    pub fn from_json(raw: &Value) -> Self {
        let d = Self::default();
        let b = |k: &str, def: bool| raw.get(k).and_then(Value::as_bool).unwrap_or(def);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 1..=4"
        )]
        let max = raw
            .get("maxQuestions")
            .and_then(Value::as_f64)
            .map_or(d.max_questions, |m| m.floor().clamp(1.0, 4.0) as usize);
        Self {
            enabled: b("enabled", d.enabled),
            notify_on_ask: b("notifyOnAsk", d.notify_on_ask),
            escape: if raw.get("escape").and_then(Value::as_str) == Some("send") {
                Escape::Send
            } else {
                Escape::Stop
            },
            digit_advance: b("digitAdvance", d.digit_advance),
            help_line: b("helpLine", d.help_line),
            other: match raw.get("other").and_then(Value::as_str) {
                Some("always") => OtherPolicy::Always,
                Some("never") => OtherPolicy::Never,
                _ => OtherPolicy::Agent,
            },
            max_questions: max,
        }
    }
}

/// The user's settings win over what the agent asked for (unipi `applySettings`).
#[must_use]
pub fn apply_settings(questions: Vec<AskQuestion>, s: &Settings) -> Vec<AskQuestion> {
    questions
        .into_iter()
        .take(s.max_questions)
        .map(|mut q| {
            if s.other == OtherPolicy::Always || q.options.is_empty() {
                q.other = None;
            } else if s.other == OtherPolicy::Never {
                q.other = Some(false);
            }
            q
        })
        .collect()
}

/// The `form` dialog request (ui-protocol §4.3) plus Oino presentation hints (`escape`,
/// `digitAdvance`, `helpLine`, `clarify`, see deviations).
#[must_use]
pub fn form_request(questions: &[AskQuestion], s: &Settings) -> Value {
    let title: Vec<&str> = questions
        .iter()
        .map(|q| {
            if q.header.is_empty() {
                q.question.as_str()
            } else {
                q.header.as_str()
            }
        })
        .collect();
    json!({
        "method": "form",
        "title": title.join(" · "),
        "questions": questions.iter().enumerate().map(|(i, q)| json!({
            "id": format!("q{i}"),
            "question": q.question,
            "header": q.header,
            "options": q.options.iter().map(|o| {
                let mut v = json!({"label": o.label});
                if let Some(d) = &o.description { v["description"] = json!(d); }
                v["value"] = json!(o.value());
                if let Some(a) = o.action { v["action"] = serde_json::to_value(a).unwrap_or(Value::Null); }
                if let Some(p) = &o.prefill { v["prefill"] = json!(p); }
                v
            }).collect::<Vec<_>>(),
            "multiSelect": q.multi(),
            "other": q.has_other(),
            "attachments": q.has_other(),
        })).collect::<Vec<_>>(),
        "escape": match s.escape { Escape::Stop => "stop", Escape::Send => "send" },
        "digitAdvance": s.digit_advance,
        "helpLine": s.help_line,
        "clarify": true,
    })
}

/// What the dialog produced (unipi `PanelResult`).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Answers (some may be skipped) and attached images (`ImageContent` JSON).
    Answered {
        /// Per question.
        answers: Vec<Answer>,
        /// Images from "Other".
        images: Vec<Value>,
    },
    /// "Not ready — help me out".
    Clarify(Vec<Answer>),
    /// An option with an action was picked.
    Action {
        /// Question index.
        question: usize,
        /// The option.
        option: AskOption,
        /// Answers so far.
        answers: Vec<Answer>,
    },
    /// Esc / dismissed.
    Cancel,
}

/// Decodes a dialog response (`{value: FormAnswers}` or `{cancelled: true}`).
#[must_use]
pub fn decode(response: &Value, questions: &[AskQuestion]) -> Outcome {
    let Some(v) = response.get("value").filter(|v| v.is_object()) else {
        return Outcome::Cancel;
    };
    let raw = v
        .get("answers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut images = Vec::new();
    let answers: Vec<Answer> = questions
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let id = format!("q{i}");
            let a = raw
                .iter()
                .find(|a| a.get("id").and_then(Value::as_str) == Some(id.as_str()))
                .or_else(|| raw.get(i).filter(|a| a.get("id").is_none()));
            let Some(a) = a else { return Answer::skipped() };
            if let Some(imgs) = a.get("attachments").and_then(Value::as_array) {
                images.extend(imgs.iter().cloned());
            }
            let mut selected: Vec<String> = a
                .get("selected")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|s| q.options.iter().any(|o| o.value() == *s))
                .map(ToOwned::to_owned)
                .collect();
            if !q.multi() {
                selected.truncate(1);
            }
            let custom = a
                .get("other")
                .or_else(|| a.get("custom_text"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty() && q.has_other())
                .map(ToOwned::to_owned);
            let mut answer = Answer {
                selected,
                custom_text: custom,
                skipped: false,
            };
            answer.skipped = !answer.is_answered();
            if answer.skipped {
                Answer::skipped()
            } else {
                answer
            }
        })
        .collect();
    if v.get("clarify").and_then(Value::as_bool) == Some(true) {
        return Outcome::Clarify(answers);
    }
    for (i, q) in questions.iter().enumerate() {
        if q.multi() {
            continue;
        }
        let picked = answers.get(i).and_then(|a| a.selected.first());
        if let Some(opt) = q
            .options
            .iter()
            .find(|o| o.action.is_some() && Some(&o.value().to_owned()) == picked)
        {
            return Outcome::Action {
                question: i,
                option: opt.clone(),
                answers,
            };
        }
    }
    Outcome::Answered { answers, images }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::questions::{OptionAction, prepare_args};

    fn questions() -> Vec<AskQuestion> {
        prepare_args(&json!({"questions": [
            {"question": "Which DB?", "header": "DB", "options": [{"label": "Postgres"}, {"label": "SQLite", "value": "sqlite"}]},
            {"question": "Extras?", "header": "Extras", "multi_select": true, "options": [{"label": "Auth"}, {"label": "Cache"}]},
            {"question": "Stop?", "header": "Stop", "other": false, "options": [{"label": "Go on"}, {"label": "End turn", "action": "end_turn"}]},
        ]}))
    }

    fn answer(selected: &[&str]) -> Answer {
        Answer {
            selected: selected.iter().map(|s| (*s).to_owned()).collect(),
            custom_text: None,
            skipped: false,
        }
    }

    #[test]
    fn keeps_known_options_and_single_vs_multi() {
        let r = decode(
            &json!({"value": {"answers": [
                {"id": "q0", "selected": ["sqlite", "Postgres"]},
                {"id": "q1", "selected": ["Auth", "nope", "Cache"]},
            ]}}),
            &questions(),
        );
        assert_eq!(
            r,
            Outcome::Answered {
                answers: vec![
                    answer(&["sqlite"]),
                    answer(&["Auth", "Cache"]),
                    Answer::skipped()
                ],
                images: vec![]
            }
        );
    }

    #[test]
    fn free_text_skips_cancel_and_garbage() {
        let r = decode(
            &json!({"value": {"answers": [{"id": "q0", "selected": [], "other": " MySQL "}, {"id": "q1"}, {"id": "q2", "other": "x"}]}}),
            &questions(),
        );
        let Outcome::Answered { answers, .. } = r else {
            unreachable!("answered")
        };
        assert_eq!(
            answers[0],
            Answer {
                selected: vec![],
                custom_text: Some("MySQL".into()),
                skipped: false
            }
        );
        assert_eq!(answers[1], Answer::skipped());
        assert_eq!(
            answers[2],
            Answer::skipped(),
            "other:false ignores free text"
        );
        assert_eq!(
            decode(&json!({"cancelled": true}), &questions()),
            Outcome::Cancel
        );
        assert_eq!(decode(&json!(null), &questions()), Outcome::Cancel);
        assert_eq!(
            decode(&json!({"value": "??"}), &questions()),
            Outcome::Cancel
        );
    }

    #[test]
    fn action_option_and_clarify() {
        let r = decode(
            &json!({"value": {"answers": [{"id": "q0", "selected": ["Postgres"]}, {"id": "q2", "selected": ["End turn"]}], "action": "end_turn"}}),
            &questions(),
        );
        let Outcome::Action {
            question, option, ..
        } = r
        else {
            unreachable!("action")
        };
        assert_eq!(question, 2);
        assert_eq!(option.action, Some(OptionAction::EndTurn));
        let r = decode(
            &json!({"value": {"answers": [], "clarify": true}}),
            &questions(),
        );
        assert!(matches!(r, Outcome::Clarify(a) if a.iter().all(|a| a.skipped)));
    }

    #[test]
    fn images_are_collected() {
        let img = json!({"type": "image", "data": "AAAA", "mimeType": "image/png"});
        let r = decode(
            &json!({"value": {"answers": [{"id": "q0", "other": "see [Image #1]", "attachments": [img]}]}}),
            &questions(),
        );
        let Outcome::Answered { images, answers } = r else {
            unreachable!("answered")
        };
        assert_eq!(images.len(), 1);
        assert_eq!(answers[0].custom_text.as_deref(), Some("see [Image #1]"));
    }

    #[test]
    fn settings_win_over_the_agent() {
        let qs = prepare_args(&json!({"questions": [
            {"question": "a?", "header": "A", "options": [{"label": "x"}], "other": false},
            {"question": "b?", "header": "B", "options": [{"label": "y"}]},
            {"question": "c?", "header": "C", "options": []},
        ]}));
        let s = |o: OtherPolicy, m: usize| Settings {
            other: o,
            max_questions: m,
            ..Settings::default()
        };
        assert_eq!(
            apply_settings(qs.clone(), &s(OtherPolicy::Always, 4))[0].other,
            None
        );
        assert_eq!(
            apply_settings(qs.clone(), &s(OtherPolicy::Never, 4))[1].other,
            Some(false)
        );
        assert_eq!(
            apply_settings(qs.clone(), &s(OtherPolicy::Never, 4))[2].other,
            None
        );
        assert_eq!(apply_settings(qs, &s(OtherPolicy::Agent, 2)).len(), 2);
    }

    #[test]
    fn settings_parse_and_clamp() {
        let s = Settings::from_json(
            &json!({"enabled": true, "escape": "send", "other": "never", "maxQuestions": 9.7}),
        );
        assert!(s.enabled);
        assert_eq!(s.escape, Escape::Send);
        assert_eq!(s.other, OtherPolicy::Never);
        assert_eq!(s.max_questions, 4);
        assert_eq!(
            Settings::from_json(&json!({"maxQuestions": 0})).max_questions,
            1
        );
        assert_eq!(Settings::from_json(&json!({})), Settings::default());
    }

    #[test]
    fn form_request_shape() {
        let f = form_request(&questions(), &Settings::default());
        assert_eq!(f["method"], "form");
        assert_eq!(f["title"], "DB · Extras · Stop");
        assert_eq!(
            f["questions"][0]["options"][1],
            json!({"label": "SQLite", "value": "sqlite"})
        );
        assert_eq!(f["questions"][1]["multiSelect"], true);
        assert_eq!(f["questions"][2]["other"], false);
        assert_eq!(f["questions"][2]["options"][1]["action"], "end_turn");
        assert_eq!(f["escape"], "stop");
    }
}
