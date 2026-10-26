//! Typed extension dialogs (ui-protocol §4.1, §4.3) and their answers.
//!
//! A [`Dialog`] is parsed from an `extension_ui_request` whose method waits for an answer.
//! Answer builders produce the exact pi / Oino response forms, so views only deal with
//! user intents.

use oino_ai::{JsonObject, JsonValue};
use oino_core::ui::{UiAnswer, UiRequest, UiResponse};

use crate::model::ous::OusNode;

/// One option of a `select` dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    /// Label (also the answer value, pi semantics).
    pub label: String,
    /// Oino `optionDetails[i].description`.
    pub description: Option<String>,
}

/// One option of a `multiSelect` dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiOption {
    /// Label.
    pub label: String,
    /// Description.
    pub description: Option<String>,
    /// Answer value (defaults to the label).
    pub value: String,
    /// Initially selected.
    pub selected: bool,
}

/// `approval` option kinds (ACP permission option kinds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalKind {
    /// Allow this call.
    AllowOnce,
    /// Allow and remember.
    AllowAlways,
    /// Reject this call.
    RejectOnce,
    /// Reject and remember.
    RejectAlways,
}

impl ApprovalKind {
    fn parse(text: &str) -> Self {
        match text {
            "allow_always" => Self::AllowAlways,
            "reject_once" => Self::RejectOnce,
            "reject_always" => Self::RejectAlways,
            _ => Self::AllowOnce,
        }
    }

    /// Whether this option allows the call.
    #[must_use]
    pub const fn is_allow(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }
}

/// One `approval` option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalOption {
    /// Option id returned in the answer.
    pub option_id: String,
    /// Button label.
    pub label: String,
    /// Kind.
    pub kind: ApprovalKind,
}

/// One `form` question (unipi `ask_user`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormQuestion {
    /// Question id.
    pub id: String,
    /// Question text.
    pub question: String,
    /// Short header chip.
    pub header: Option<String>,
    /// Options (label, description, value).
    pub options: Vec<MultiOption>,
    /// Multiple answers allowed.
    pub multi: bool,
    /// Free-text "Other" allowed.
    pub other: bool,
}

/// Dialog payload by method.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogKind {
    /// pi `select`.
    Select {
        /// Options.
        options: Vec<SelectOption>,
    },
    /// pi `confirm`.
    Confirm {
        /// Body.
        message: String,
    },
    /// pi `input` (+ Oino `prefill`, `secret`).
    Input {
        /// Placeholder.
        placeholder: Option<String>,
        /// Prefilled text.
        prefill: Option<String>,
        /// Mask input.
        secret: bool,
    },
    /// pi `editor` (+ Oino `language`).
    Editor {
        /// Prefilled text.
        prefill: Option<String>,
        /// Language hint.
        language: Option<String>,
    },
    /// Oino `multiSelect`.
    MultiSelect {
        /// Options.
        options: Vec<MultiOption>,
        /// Minimum selections.
        min: Option<usize>,
        /// Maximum selections.
        max: Option<usize>,
    },
    /// Oino `form`.
    Form {
        /// Questions.
        questions: Vec<FormQuestion>,
        /// Submit button label.
        submit_label: Option<String>,
    },
    /// Oino `approval`.
    Approval {
        /// Tool name.
        tool_name: Option<String>,
        /// One-line summary.
        summary: String,
        /// Detail node (diff, command).
        detail: Option<OusNode>,
        /// Options.
        options: Vec<ApprovalOption>,
        /// Feedback placeholder when free-text feedback is offered.
        feedback: Option<String>,
    },
    /// Oino `custom` (modal OUS).
    Custom {
        /// Body.
        spec: OusNode,
    },
    /// A dialog this frontend cannot show (`clipboardReadImage`, `editExternally`): answered
    /// as cancelled.
    Unsupported,
}

/// A pending dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    /// Request id.
    pub id: String,
    /// Method name.
    pub method: String,
    /// Title.
    pub title: String,
    /// Owning extension.
    pub extension_id: Option<String>,
    /// Timeout (ms), shown as a countdown hint.
    pub timeout_ms: Option<u64>,
    /// Payload.
    pub kind: DialogKind,
}

fn s(params: &JsonObject, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn n(params: &JsonObject, key: &str) -> Option<usize> {
    params
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|v| usize::try_from(v).ok())
}

fn multi_options(value: Option<&JsonValue>) -> Vec<MultiOption> {
    value
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            if let Some(label) = option.as_str() {
                return Some(MultiOption {
                    label: label.to_owned(),
                    description: None,
                    value: label.to_owned(),
                    selected: false,
                });
            }
            let label = option.get("label")?.as_str()?.to_owned();
            Some(MultiOption {
                description: option
                    .get("description")
                    .and_then(JsonValue::as_str)
                    .map(str::to_owned),
                value: option
                    .get("value")
                    .and_then(JsonValue::as_str)
                    .map_or_else(|| label.clone(), str::to_owned),
                selected: option
                    .get("selected")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                label,
            })
        })
        .collect()
}

impl Dialog {
    /// Parses a dialog request; `None` for fire-and-forget methods.
    #[must_use]
    #[allow(clippy::too_many_lines, reason = "one arm per event / method kind")]
    pub fn parse(request: &UiRequest) -> Option<Self> {
        if !oino_core::ui::is_dialog(&request.method) {
            return None;
        }
        let p = &request.params;
        let kind = match request.method.as_str() {
            "select" => {
                let details = p.get("optionDetails").and_then(JsonValue::as_array);
                let options = p
                    .get("options")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(i, o)| {
                        Some(SelectOption {
                            label: o.as_str()?.to_owned(),
                            description: details
                                .and_then(|d| d.get(i))
                                .and_then(|d| d.get("description"))
                                .and_then(JsonValue::as_str)
                                .map(str::to_owned),
                        })
                    })
                    .collect();
                DialogKind::Select { options }
            }
            "confirm" => DialogKind::Confirm {
                message: s(p, "message").unwrap_or_default(),
            },
            "input" => DialogKind::Input {
                placeholder: s(p, "placeholder"),
                prefill: s(p, "prefill"),
                secret: p.get("secret").and_then(JsonValue::as_bool) == Some(true),
            },
            "editor" => DialogKind::Editor {
                prefill: s(p, "prefill"),
                language: s(p, "language"),
            },
            "multiSelect" => DialogKind::MultiSelect {
                options: multi_options(p.get("options")),
                min: n(p, "min"),
                max: n(p, "max"),
            },
            "form" => DialogKind::Form {
                questions: p
                    .get("questions")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|q| {
                        let q = q.as_object()?;
                        Some(FormQuestion {
                            id: s(q, "id")?,
                            question: s(q, "question").unwrap_or_default(),
                            header: s(q, "header"),
                            options: multi_options(q.get("options")),
                            multi: q.get("multiSelect").and_then(JsonValue::as_bool) == Some(true),
                            other: q.get("other").and_then(JsonValue::as_bool) == Some(true),
                        })
                    })
                    .collect(),
                submit_label: s(p, "submitLabel"),
            },
            "approval" => DialogKind::Approval {
                tool_name: s(p, "toolName"),
                summary: s(p, "summary").unwrap_or_default(),
                detail: p.get("detail").map(OusNode::parse),
                options: p
                    .get("options")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|o| {
                        let o = o.as_object()?;
                        Some(ApprovalOption {
                            option_id: s(o, "optionId")?,
                            label: s(o, "label").unwrap_or_default(),
                            kind: ApprovalKind::parse(&s(o, "kind").unwrap_or_default()),
                        })
                    })
                    .collect(),
                feedback: p.get("feedback").map(|f| {
                    f.get("placeholder")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("Feedback for the agent (optional)")
                        .to_owned()
                }),
            },
            "custom" => DialogKind::Custom {
                spec: p
                    .get("spec")
                    .map_or(OusNode::Empty { message: None }, OusNode::parse),
            },
            _ => DialogKind::Unsupported,
        };
        Some(Self {
            id: request.id.clone(),
            method: request.method.clone(),
            title: s(p, "title").unwrap_or_default(),
            extension_id: request.extension_id.clone(),
            timeout_ms: p.get("timeout").and_then(JsonValue::as_u64),
            kind,
        })
    }

    fn answer(&self, answer: UiAnswer) -> UiResponse {
        UiResponse {
            id: self.id.clone(),
            answer,
        }
    }

    /// Cancels (Escape / close button).
    #[must_use]
    pub fn cancel(&self) -> UiResponse {
        self.answer(UiAnswer::Cancelled)
    }

    /// Answers `confirm`.
    #[must_use]
    pub fn confirm(&self, yes: bool) -> UiResponse {
        self.answer(UiAnswer::Confirmed(yes))
    }

    /// Answers with a string value (`select`, `input`, `editor`).
    #[must_use]
    pub fn text(&self, value: impl Into<String>) -> UiResponse {
        self.answer(UiAnswer::Value(JsonValue::String(value.into())))
    }

    /// Answers `multiSelect` with the selected option values, enforcing `min`/`max`.
    ///
    /// # Errors
    ///
    /// The message to show when the selection count is out of bounds.
    pub fn multi(&self, selected: &[String]) -> Result<UiResponse, String> {
        if let DialogKind::MultiSelect { min, max, .. } = &self.kind {
            if let Some(min) = min
                && selected.len() < *min
            {
                return Err(format!("Select at least {min}"));
            }
            if let Some(max) = max
                && selected.len() > *max
            {
                return Err(format!("Select at most {max}"));
            }
        }
        Ok(self.answer(UiAnswer::Value(JsonValue::from(selected.to_vec()))))
    }

    /// Answers `approval`.
    #[must_use]
    pub fn approve(&self, option_id: &str, feedback: Option<&str>) -> UiResponse {
        let mut value = serde_json::Map::new();
        value.insert("optionId".into(), JsonValue::from(option_id));
        if let Some(feedback) = feedback.map(str::trim).filter(|f| !f.is_empty()) {
            value.insert("feedback".into(), JsonValue::from(feedback));
        }
        self.answer(UiAnswer::Value(JsonValue::Object(value)))
    }

    /// Answers `form`: `answers[i] = (question id, selected values, other text)`.
    #[must_use]
    pub fn form(&self, answers: &[(String, Vec<String>, Option<String>)]) -> UiResponse {
        let answers: Vec<JsonValue> = answers
            .iter()
            .map(|(id, selected, other)| {
                let mut a = serde_json::Map::new();
                a.insert("id".into(), JsonValue::from(id.as_str()));
                a.insert("selected".into(), JsonValue::from(selected.clone()));
                if let Some(other) = other.as_deref().filter(|o| !o.trim().is_empty()) {
                    a.insert("other".into(), JsonValue::from(other));
                }
                JsonValue::Object(a)
            })
            .collect();
        let mut value = serde_json::Map::new();
        value.insert("answers".into(), JsonValue::from(answers));
        self.answer(UiAnswer::Value(JsonValue::Object(value)))
    }

    /// Answers `custom` with the first submitting action.
    #[must_use]
    pub fn action(&self, action_id: &str, value: Option<JsonValue>) -> UiResponse {
        let mut v = serde_json::Map::new();
        v.insert("actionId".into(), JsonValue::from(action_id));
        if let Some(value) = value {
            v.insert("value".into(), value);
        }
        self.answer(UiAnswer::Value(JsonValue::Object(v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(value: JsonValue) -> UiRequest {
        serde_json::from_value(value).unwrap()
    }

    fn wire(response: &UiResponse) -> JsonValue {
        serde_json::to_value(response).unwrap()
    }

    #[test]
    fn pi_dialogs_round_trip() {
        let select = Dialog::parse(&request(json!({
            "type":"extension_ui_request","id":"1","method":"select","title":"Pick",
            "options":["a","b"],"optionDetails":[{"label":"a","description":"first"}],"timeout":5000
        })))
        .unwrap();
        let DialogKind::Select { options } = &select.kind else {
            panic!()
        };
        assert_eq!(options[0].description.as_deref(), Some("first"));
        assert_eq!(options[1].description, None);
        assert_eq!(select.timeout_ms, Some(5000));
        assert_eq!(
            wire(&select.text("b")),
            json!({"type":"extension_ui_response","id":"1","value":"b"})
        );
        assert_eq!(
            wire(&select.cancel()),
            json!({"type":"extension_ui_response","id":"1","cancelled":true})
        );
        let confirm = Dialog::parse(&request(json!({
            "type":"extension_ui_request","id":"2","method":"confirm","title":"Sure?","message":"Really"
        })))
        .unwrap();
        assert_eq!(
            wire(&confirm.confirm(true)),
            json!({"type":"extension_ui_response","id":"2","confirmed":true})
        );
        assert!(
            Dialog::parse(&request(json!({
                "type":"extension_ui_request","id":"3","method":"notify","message":"x"
            })))
            .is_none()
        );
    }

    #[test]
    fn approval_with_feedback() {
        let dialog = Dialog::parse(&request(json!({
            "type":"extension_ui_request","id":"9","method":"approval","title":"Run command?",
            "toolName":"bash","summary":"rm -rf build",
            "detail":{"t":"code","value":"rm -rf build","language":"sh"},
            "options":[{"optionId":"y","label":"Allow","kind":"allow_once"},
                       {"optionId":"n","label":"Deny","kind":"reject_once"}],
            "feedback":{}
        })))
        .unwrap();
        let DialogKind::Approval {
            options,
            feedback,
            detail,
            ..
        } = &dialog.kind
        else {
            panic!()
        };
        assert!(options[0].kind.is_allow());
        assert!(!options[1].kind.is_allow());
        assert!(feedback.is_some());
        assert!(matches!(detail, Some(OusNode::Code { .. })));
        assert_eq!(
            wire(&dialog.approve("n", Some("  use cargo clean  "))),
            json!({"type":"extension_ui_response","id":"9","value":{"optionId":"n","feedback":"use cargo clean"}})
        );
        assert_eq!(
            wire(&dialog.approve("y", Some(" "))),
            json!({"type":"extension_ui_response","id":"9","value":{"optionId":"y"}})
        );
    }

    #[test]
    fn multi_select_bounds_and_form_answers() {
        let dialog = Dialog::parse(&request(json!({
            "type":"extension_ui_request","id":"m","method":"multiSelect","title":"T",
            "options":[{"label":"A","value":"a","selected":true},{"label":"B"}],"min":1,"max":1
        })))
        .unwrap();
        let DialogKind::MultiSelect { options, .. } = &dialog.kind else {
            panic!()
        };
        assert_eq!(options[1].value, "B");
        assert!(options[0].selected);
        assert_eq!(dialog.multi(&[]).unwrap_err(), "Select at least 1");
        assert!(dialog.multi(&["a".into(), "B".into()]).is_err());
        assert_eq!(
            wire(&dialog.multi(&["a".into()]).unwrap())["value"],
            json!(["a"])
        );
        let form = Dialog::parse(&request(json!({
            "type":"extension_ui_request","id":"f","method":"form","title":"Q",
            "questions":[{"id":"q1","question":"Which?","options":[{"label":"X"}],"other":true}]
        })))
        .unwrap();
        let DialogKind::Form { questions, .. } = &form.kind else {
            panic!()
        };
        assert!(questions[0].other);
        assert_eq!(
            wire(&form.form(&[("q1".into(), vec!["X".into()], Some("more".into()))]))["value"],
            json!({"answers":[{"id":"q1","selected":["X"],"other":"more"}]})
        );
    }
}
