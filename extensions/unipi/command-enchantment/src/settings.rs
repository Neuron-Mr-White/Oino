//! Namespace `command-enchantment` (unipi `src/settings.ts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Field, Namespace, Section};

/// Namespace.
pub const NAMESPACE: &str = "command-enchantment";

/// Defaults.
#[must_use]
pub fn defaults() -> Value {
    json!({"autocompleteEnhanced": true})
}

/// Schema.
#[must_use]
pub fn namespace() -> Namespace {
    Namespace::new(NAMESPACE, "Command Enchantment", crate::EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("autocomplete", "Autocomplete").field(
                Field::boolean("autocompleteEnhanced", "Enhanced autocomplete")
                    .description("Fuzzy command suggestions with feature tags while you type."),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_unipi() {
        let v = namespace().to_json();
        assert_eq!(v["namespace"], "command-enchantment");
        assert_eq!(v["title"], "Command Enchantment");
        let f = &v["sections"][0]["fields"][0];
        assert_eq!(f["key"], "autocompleteEnhanced");
        assert_eq!(f["type"], "boolean");
        assert_eq!(f["default"], true);
        assert_eq!(v["sections"][0]["title"], "Autocomplete");
    }
}
