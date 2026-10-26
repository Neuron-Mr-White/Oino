use oino_agent_loop::{
    AbortSignal, BoxFuture, LoopError, LoopResult, Tool, ToolCall, ToolDefinition,
    ToolExecutionMode, ToolResult, ToolUpdateCallback,
};
use oino_tui::{AskUserOutcome, AskUserRequest};
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc, time::Duration};

pub const ASK_USER_TOOL_NAME: &str = "ask_user";

pub type AskUserRequester =
    Arc<dyn Fn(AskUserRequest) -> BoxFuture<'static, LoopResult<AskUserOutcome>> + Send + Sync>;

#[derive(Clone)]
pub struct AskUserTool {
    requester: Option<AskUserRequester>,
}

impl AskUserTool {
    #[must_use]
    pub fn new(requester: Option<AskUserRequester>) -> Self {
        Self { requester }
    }
}

#[async_trait::async_trait]
impl Tool for AskUserTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: ASK_USER_TOOL_NAME.into(),
            description: "Ask the user one or more structured questions and wait for their answer before continuing. Use this for ambiguous requirements, risky choices, or user preferences.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["questions"],
                "properties": {
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["question", "options"],
                            "properties": {
                                "question": {"type":"string", "description":"Question to ask the user."},
                                "header": {"type":"string", "description":"Short label shown in the modal, max 16 characters."},
                                "options": {
                                    "type":"array",
                                    "minItems": 2,
                                    "maxItems": 4,
                                    "items": {
                                        "type":"object",
                                        "additionalProperties": false,
                                        "required": ["label", "description"],
                                        "properties": {
                                            "label": {"type":"string", "description":"Option label, max 60 characters."},
                                            "description": {"type":"string", "description":"Brief option description."},
                                            "preview": {"type":"string", "description":"Optional markdown/code/text preview."}
                                        }
                                    }
                                },
                                "multi_select": {"type":"boolean", "description":"Allow selecting more than one option."},
                                "multiSelect": {"type":"boolean", "description":"Alias for multi_select."}
                            }
                        }
                    }
                }
            }),
        }
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        if signal.is_aborted() {
            return Err(LoopError::Aborted);
        }
        let request: AskUserRequest = serde_json::from_value(call.arguments.clone())
            .map_err(|err| LoopError::Tool(format!("invalid ask_user input: {err}")))?;
        if let Some(error) = validate_request(&request) {
            return Ok(tool_result(
                &call,
                AskUserOutcome {
                    answers: Vec::new(),
                    cancelled: true,
                    error: Some(error),
                },
            ));
        }
        let Some(requester) = &self.requester else {
            return Ok(tool_result(&call, no_ui_outcome()));
        };
        let response = requester(request);
        let outcome = tokio::select! {
            _ = async {
                while !signal.is_aborted() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => return Err(LoopError::Aborted),
            received = response => received.unwrap_or_else(|err| AskUserOutcome {
                answers: Vec::new(),
                cancelled: true,
                error: Some(err.to_string()),
            }),
        };
        Ok(tool_result(&call, outcome))
    }
}

fn no_ui_outcome() -> AskUserOutcome {
    AskUserOutcome {
        answers: Vec::new(),
        cancelled: true,
        error: Some("no_ui".into()),
    }
}

pub fn validate_request(request: &AskUserRequest) -> Option<String> {
    if request.questions.is_empty() {
        return Some("no_questions".into());
    }
    if request.questions.len() > 4 {
        return Some("too_many_questions".into());
    }
    let reserved = ["Other", "Type something.", "Chat about this", "Next"];
    let mut questions = BTreeSet::new();
    for question in &request.questions {
        let text = question.question.trim();
        if text.is_empty() {
            return Some("no_questions".into());
        }
        if !questions.insert(text.to_ascii_lowercase()) {
            return Some("duplicate_question".into());
        }
        if !question.header.is_empty() && question.header.chars().count() > 16 {
            return Some("header_too_long".into());
        }
        if question.options.len() < 2 {
            return Some("empty_options".into());
        }
        if question.options.len() > 4 {
            return Some("too_many_options".into());
        }
        let mut labels = BTreeSet::new();
        for option in &question.options {
            let label = option.label.trim();
            if label.is_empty() {
                return Some("empty_option_label".into());
            }
            if label.chars().count() > 60 {
                return Some("option_label_too_long".into());
            }
            if reserved
                .iter()
                .any(|reserved| reserved.eq_ignore_ascii_case(label))
            {
                return Some("reserved_label".into());
            }
            if !labels.insert(label.to_ascii_lowercase()) {
                return Some("duplicate_option_label".into());
            }
        }
    }
    None
}

pub fn tool_result(call: &ToolCall, outcome: AskUserOutcome) -> ToolResult {
    let text = if outcome.cancelled {
        "User declined to answer questions".to_string()
    } else {
        let answers = outcome
            .answers
            .iter()
            .map(|answer| {
                let value = if answer.selected.is_empty() {
                    answer.answer.clone().unwrap_or_default()
                } else {
                    answer.selected.join(", ")
                };
                format!("\"{}\"=\"{}\"", answer.question, value)
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("User has answered your questions: {answers}. You can now continue with the user's answers in mind.")
    };
    let mut result = ToolResult::text(call, text);
    result.details = Some(json!({
        "answers": outcome.answers,
        "cancelled": outcome.cancelled,
        "error": outcome.error,
    }));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_tui::{AskUserOption, AskUserQuestion};

    fn request() -> AskUserRequest {
        AskUserRequest {
            questions: vec![AskUserQuestion {
                question: "Pick one?".into(),
                header: "Pick".into(),
                options: vec![
                    AskUserOption {
                        label: "A".into(),
                        description: "Alpha".into(),
                        preview: None,
                    },
                    AskUserOption {
                        label: "B".into(),
                        description: "Beta".into(),
                        preview: None,
                    },
                ],
                multi_select: false,
            }],
        }
    }

    #[test]
    fn validates_reserved_labels() {
        let mut request = request();
        request.questions[0].options[0].label = "Other".into();
        assert_eq!(validate_request(&request), Some("reserved_label".into()));
    }

    #[test]
    fn accepts_basic_question() {
        assert_eq!(validate_request(&request()), None);
    }
}
