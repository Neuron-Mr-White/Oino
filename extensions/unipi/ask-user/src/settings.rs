//! Namespace `ask-user` (unipi `config.ts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Kind, Namespace, Section, choice};

/// Namespace.
pub const NAMESPACE: &str = "ask-user";

/// Defaults (unipi `DEFAULT_SETTINGS`).
#[must_use]
pub fn defaults() -> Value {
    json!({
        "enabled": false,
        "notifyOnAsk": true,
        "escape": "stop",
        "digitAdvance": true,
        "helpLine": true,
        "other": "agent",
        "maxQuestions": 4,
    })
}

/// Schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Ask User", crate::EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("tool", "Tool")
                .field(
                    Field::boolean("enabled", "Enable ask_user")
                        .description("Let the agent stop and ask you multiple-choice questions."),
                )
                .field(
                    Field::boolean("notifyOnAsk", "Notify when asked").description(
                        "Send a notification when the agent is waiting for your answer.",
                    ),
                )
                .field(
                    Field::number("maxQuestions", "Questions per call")
                        .description(
                            "How many questions one dialog may hold. Applies to new sessions.",
                        )
                        .with_kind(|k| {
                            if let Kind::Number { min, max, .. } = k {
                                *min = Some(1.0);
                                *max = Some(4.0);
                            }
                        }),
                ),
        )
        .section(
            Section::new("dialog", "Dialog")
                .description("How the question dialog behaves")
                .field(
                    Field::enumeration(
                        "escape",
                        "Esc",
                        vec![
                            choice("stop", "stop", "stop the agent's turn"),
                            choice("send", "send", "send what's answered, skip the rest"),
                        ],
                    )
                    .description("What Esc does in the dialog."),
                )
                .field(
                    Field::boolean("digitAdvance", "Number keys move on").description(
                        "In a single-choice question, picking with 1–9 also advances.",
                    ),
                )
                .field(
                    Field::enumeration(
                        "other",
                        "\"Other\" choice",
                        vec![
                            choice("agent", "agent decides", "on unless the agent opts out"),
                            choice("always", "always", "always offer the Other row"),
                            choice("never", "never", "never offer the Other row"),
                        ],
                    )
                    .description("The free-text \"Other (type your own)\" row."),
                )
                .field(
                    Field::boolean("helpLine", "\"Not ready\" line").description(
                        "Show \"? Not ready to answer, help me out!\" under the dialog.",
                    ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_unipi() {
        let v = namespace().to_json();
        assert_eq!(v["title"], "Ask User");
        let tool = &v["sections"][0]["fields"];
        assert_eq!(tool[0]["default"], false);
        assert_eq!(tool[2]["min"], 1.0);
        assert_eq!(tool[2]["max"], 4.0);
        let dialog = &v["sections"][1];
        assert_eq!(dialog["description"], "How the question dialog behaves");
        assert_eq!(
            dialog["fields"][0]["options"][1]["description"],
            "send what's answered, skip the rest"
        );
        assert_eq!(dialog["fields"][2]["default"], "agent");
    }
}
