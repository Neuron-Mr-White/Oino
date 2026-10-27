//! Settings namespaces owned by `unipi.core`: `hints` and the shared `decision-model`.

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Namespace, Section, choice};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.core";
/// Hints namespace.
pub const HINTS: &str = "hints";
/// Hidden command behind the "Reset hint history" action.
pub const RESET_COMMAND: &str = "hint-reset";

/// `hints` defaults (unipi `registerSettings({namespace: "hints"})`).
#[must_use]
pub fn hints_defaults() -> Value {
    json!({"enabled": true, "header": true, "crab": "auto"})
}

/// `hints` namespace schema.
#[must_use]
pub fn hints_namespace() -> Namespace {
    Namespace::new(HINTS, "Hints", EXTENSION_ID)
        .defaults(hints_defaults())
        .section(
            Section::new("hints", "Unicrab hints")
                .field(Field::boolean("enabled", "Hints").description(
                    "One-line Unicrab hints above the editor on startup and matching events.",
                ))
                .field(
                    Field::boolean("header", "Unicrab start screen")
                        .description("Show the Unicrab mascot and welcome banner at startup."),
                )
                .field(
                    Field::enumeration(
                        "crab",
                        "Mascot style",
                        vec![
                            choice("auto", "auto", "half-blocks, or graphics when detected"),
                            choice("blocks", "blocks", "unicode half-blocks, always safe"),
                            choice(
                                "image",
                                "image",
                                "kitty graphics; overlays can't draw over it",
                            ),
                        ],
                    )
                    .description("How to render the Unicrab mascot in the hint widget."),
                )
                .field(
                    Field::action("reset", "Reset hint history", RESET_COMMAND).description(
                        "Clear show counts and learned state — every hint becomes eligible again.",
                    ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_schema_matches_unipi() {
        let v = hints_namespace().to_json();
        assert_eq!(v["namespace"], "hints");
        assert_eq!(v["extensionId"], "unipi.core");
        let f = &v["sections"][0]["fields"];
        assert_eq!(f[0]["key"], "enabled");
        assert_eq!(f[0]["default"], true);
        assert_eq!(f[2]["options"].as_array().map(Vec::len), Some(3));
        assert_eq!(f[2]["default"], "auto");
        assert_eq!(
            f[3],
            json!({
                "key": "reset", "type": "action", "label": "Reset hint history", "command": "hint-reset",
                "description": "Clear show counts and learned state — every hint becomes eligible again."
            })
        );
        assert_eq!(v["defaults"], hints_defaults());
    }
}
