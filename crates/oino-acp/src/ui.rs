//! Extension UI → ACP (modes §6.4): which dialogs become `session/request_permission` or
//! `elicitation/create`, the elicitation schemas, and how answers map back to
//! `extension_ui_response` values.

use oino_ai::{JsonObject, JsonValue};
use oino_core::ui::{UiAnswer, UiRequest};
use serde_json::json;

fn text<'a>(params: &'a JsonObject, key: &str) -> Option<&'a str> {
    params.get(key).and_then(JsonValue::as_str)
}

/// How a UI request is presented to the ACP client.
#[derive(Clone, Debug, PartialEq)]
pub enum Presentation {
    /// `session/request_permission` with these options; [`Presentation::permission_answer`] maps the
    /// selected option back.
    Permission {
        /// Tool call the permission belongs to (`None` = the executing one).
        tool_call_id: Option<String>,
        /// Title shown with the tool call.
        title: String,
        /// Extra text shown as tool-call content.
        detail: Option<String>,
        /// `PermissionOption`s (wire shape).
        options: Vec<JsonValue>,
        /// `confirm` (answer `confirmed`) instead of `approval` (answer `{optionId}`).
        confirm: bool,
    },
    /// `elicitation/create` form with a single `value` property.
    Elicitation {
        /// Prompt text.
        message: String,
        /// `requestedSchema`.
        schema: JsonValue,
        /// `confirm` (answer `confirmed`).
        confirm: bool,
    },
    /// Not representable: resolve with this answer at once.
    Immediate(UiAnswer),
}

/// Option id of the "allow" choice of a `confirm` turned into a permission request.
pub const CONFIRM_ALLOW: &str = "allow";
/// Option id of the "reject" choice.
pub const CONFIRM_REJECT: &str = "reject";

fn single_value_schema(property: JsonValue, title: &str) -> JsonValue {
    let mut schema = json!({
        "type": "object",
        "title": title,
        "properties": {},
        "required": ["value"],
    });
    schema["properties"]["value"] = property;
    schema
}

/// Option values of a `select` / `multiSelect` (strings, or `{label, value?}` objects).
fn option_values(params: &JsonObject) -> Vec<String> {
    params
        .get("options")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| match option {
            JsonValue::String(label) => Some(label.clone()),
            JsonValue::Object(object) => object
                .get("value")
                .or_else(|| object.get("label"))
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect()
}

impl Presentation {
    /// Chooses the presentation of dialog `request`. `tool_running` = a tool call is executing;
    /// `elicitation` = the client supports form elicitation.
    #[must_use]
    pub fn of(request: &UiRequest, tool_running: bool, elicitation: bool) -> Self {
        let params = &request.params;
        let title = text(params, "title").unwrap_or_default().to_owned();
        let elicit = |schema: JsonValue, confirm: bool| {
            if elicitation {
                Self::Elicitation {
                    message: title.clone(),
                    schema,
                    confirm,
                }
            } else if confirm {
                Self::Immediate(UiAnswer::Confirmed(false))
            } else {
                Self::Immediate(UiAnswer::Cancelled)
            }
        };
        match request.method.as_str() {
            "approval" => Self::Permission {
                tool_call_id: text(params, "toolCallId").map(str::to_owned),
                title: text(params, "summary")
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&title)
                    .to_owned(),
                detail: None,
                options: params
                    .get("options")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .map(|option| {
                        json!({
                            "optionId": option.get("optionId").cloned().unwrap_or(json!("")),
                            "name": option.get("label").cloned().unwrap_or(json!("")),
                            "kind": option.get("kind").cloned().unwrap_or(json!("allow_once")),
                        })
                    })
                    .collect(),
                confirm: false,
            },
            "confirm" if tool_running => Self::Permission {
                tool_call_id: None,
                title: title.clone(),
                detail: text(params, "message").map(str::to_owned),
                options: vec![
                    json!({"optionId": CONFIRM_ALLOW, "name": "Allow", "kind": "allow_once"}),
                    json!({"optionId": CONFIRM_REJECT, "name": "Reject", "kind": "reject_once"}),
                ],
                confirm: true,
            },
            "confirm" => {
                let message = text(params, "message").unwrap_or_default();
                elicit(
                    single_value_schema(
                        json!({"type": "boolean", "title": title, "description": message}),
                        &title,
                    ),
                    true,
                )
            }
            "select" => elicit(
                single_value_schema(
                    json!({"type": "string", "title": title, "enum": option_values(params)}),
                    &title,
                ),
                false,
            ),
            "input" | "editor" => {
                let mut property = json!({"type": "string", "title": title});
                if let Some(prefill) = text(params, "prefill") {
                    property["default"] = json!(prefill);
                }
                if let Some(placeholder) = text(params, "placeholder") {
                    property["description"] = json!(placeholder);
                }
                elicit(single_value_schema(property, &title), false)
            }
            "multiSelect" => {
                let mut property = json!({
                    "type": "array",
                    "title": title,
                    "items": {"type": "string", "enum": option_values(params)},
                });
                if let Some(min) = params.get("min").and_then(JsonValue::as_u64) {
                    property["minItems"] = json!(min);
                }
                if let Some(max) = params.get("max").and_then(JsonValue::as_u64) {
                    property["maxItems"] = json!(max);
                }
                elicit(single_value_schema(property, &title), false)
            }
            _ => Self::Immediate(UiAnswer::Cancelled),
        }
    }

    /// The dialog answer for a permission outcome (`None` = cancelled).
    #[must_use]
    pub fn permission_answer(confirm: bool, selected: Option<String>) -> UiAnswer {
        match (confirm, selected) {
            (_, None) => UiAnswer::Cancelled,
            (true, Some(id)) => UiAnswer::Confirmed(id == CONFIRM_ALLOW),
            (false, Some(id)) => UiAnswer::Value(json!({ "optionId": id })),
        }
    }

    /// The dialog answer for an elicitation outcome (`None` = declined / cancelled).
    #[must_use]
    pub fn elicitation_answer(confirm: bool, content: Option<JsonValue>) -> UiAnswer {
        let value = content.and_then(|c| c.get("value").cloned());
        match (confirm, value) {
            (true, Some(value)) => UiAnswer::Confirmed(value.as_bool().unwrap_or(false)),
            (true, None) => UiAnswer::Confirmed(false),
            (false, Some(value)) => UiAnswer::Value(value),
            (false, None) => UiAnswer::Cancelled,
        }
    }
}

/// ACP update for a fire-and-forget request; `None` = forward as `_oino/extension_ui`.
#[must_use]
pub fn fire_update(request: &UiRequest) -> Option<JsonValue> {
    let params = &request.params;
    match request.method.as_str() {
        "notify" => {
            let severity = match text(params, "notifyType") {
                Some("warning") => "warning",
                Some("error") => "error",
                _ => "info",
            };
            Some(json!({
                "sessionUpdate": "notice",
                "severity": severity,
                "title": text(params, "message").unwrap_or_default(),
            }))
        }
        "setTitle" => Some(json!({
            "sessionUpdate": "session_info_update",
            "title": text(params, "title"),
        })),
        // Oino addition: an extension-maintained plan (`entries: [{content, priority, status}]`).
        "setPlan" => Some(json!({
            "sessionUpdate": "plan",
            "entries": params.get("entries").cloned().unwrap_or(json!([])),
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: JsonValue) -> UiRequest {
        UiRequest {
            id: "1".into(),
            method: method.into(),
            params: serde_json::from_value(params).unwrap(),
            extension_id: None,
            session_id: None,
        }
    }

    #[test]
    fn approval_maps_options() {
        let presentation = Presentation::of(
            &request(
                "approval",
                json!({"title": "Run?", "toolCallId": "t1", "summary": "rm -rf",
                       "options": [{"optionId": "y", "label": "Yes", "kind": "allow_always"}]}),
            ),
            true,
            false,
        );
        let Presentation::Permission {
            tool_call_id,
            title,
            options,
            confirm,
            ..
        } = presentation
        else {
            panic!("expected permission");
        };
        assert_eq!(tool_call_id.as_deref(), Some("t1"));
        assert_eq!(title, "rm -rf");
        assert!(!confirm);
        assert_eq!(
            options,
            vec![json!({"optionId": "y", "name": "Yes", "kind": "allow_always"})]
        );
        assert_eq!(
            Presentation::permission_answer(false, Some("y".into())),
            UiAnswer::Value(json!({"optionId": "y"}))
        );
    }

    #[test]
    fn confirm_without_tool_or_elicitation_is_false() {
        let confirm = request("confirm", json!({"title": "Sure?", "message": "m"}));
        assert_eq!(
            Presentation::of(&confirm, false, false),
            Presentation::Immediate(UiAnswer::Confirmed(false))
        );
        assert!(matches!(
            Presentation::of(&confirm, true, false),
            Presentation::Permission { confirm: true, .. }
        ));
        assert!(matches!(
            Presentation::of(&confirm, false, true),
            Presentation::Elicitation { confirm: true, .. }
        ));
        assert_eq!(
            Presentation::elicitation_answer(true, Some(json!({"value": true}))),
            UiAnswer::Confirmed(true)
        );
    }

    #[test]
    fn select_schema_and_fire() {
        let Presentation::Elicitation { schema, .. } = Presentation::of(
            &request("select", json!({"title": "Pick", "options": ["a", "b"]})),
            false,
            true,
        ) else {
            panic!("expected elicitation");
        };
        assert_eq!(schema["properties"]["value"]["enum"], json!(["a", "b"]));
        assert_eq!(
            fire_update(&request(
                "notify",
                json!({"message": "hi", "notifyType": "warning"})
            )),
            Some(json!({"sessionUpdate": "notice", "severity": "warning", "title": "hi"}))
        );
        assert_eq!(fire_update(&request("setStatus", json!({}))), None);
    }
}
