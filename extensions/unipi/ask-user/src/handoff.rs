//! `new_session` handoffs (unipi `handoff.ts`, `launcher-ui.ts`): the launcher choices, prefill
//! normalisation, result texts and the once-only delivery guard for the compact path.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::{Value, json};

/// Compaction fallback: deliver anyway after this long.
pub const COMPACT_HANDOFF_FALLBACK_MS: u64 = 1_500;

/// Launcher choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    /// Compact, then run the prefill.
    Compact,
    /// Run the prefill directly.
    Direct,
    /// Fresh session with the prefill in the editor (unipi-port §3.5).
    NewSession,
    /// Abort the handoff.
    Cancel,
}

/// Launcher rows: `(icon + label, choice)`.
pub const OPTIONS: &[(&str, Launch)] = &[
    ("🧹 Compact & run", Launch::Compact),
    ("▶ Run directly", Launch::Direct),
    ("✚ New session", Launch::NewSession),
    ("✕ Cancel", Launch::Cancel),
];

impl Launch {
    /// unipi `SessionLaunchAction` name in result texts.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Direct => "direct",
            Self::NewSession => "new-session",
            Self::Cancel => "cancel",
        }
    }
}

/// The launcher dialog (`select`, ui-protocol §4.1).
#[must_use]
pub fn launcher_request(prefill: &str) -> Value {
    let shown = if prefill.is_empty() {
        "(no command)"
    } else {
        prefill
    };
    json!({
        "method": "select",
        "title": format!("🚀 {shown}"),
        "options": OPTIONS.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
    })
}

/// Decodes the launcher answer (`None` = dismissed).
#[must_use]
pub fn decode_launch(response: &Value) -> Launch {
    response
        .get("value")
        .and_then(Value::as_str)
        .and_then(|v| OPTIONS.iter().find(|(l, _)| *l == v))
        .map_or(Launch::Cancel, |(_, c)| *c)
}

/// Trimmed prefill, `None` when empty (unipi `empty-prefill`).
#[must_use]
pub fn normalize_prefill(prefill: Option<&str>) -> Option<String> {
    let t = prefill.unwrap_or_default().trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// Compaction instructions for the compact path (unipi `COMPACTOR_INSTRUCTION` + suffix).
#[must_use]
pub fn compact_instructions(prefill: &str) -> String {
    format!(
        "{COMPACTOR_INSTRUCTION}\nPreparing for new task. Summarize previous work concisely, preserving only what's essential for: {prefill}"
    )
}

/// unipi core `COMPACTOR_INSTRUCTION`: the marker telling the compactor its own method applies.
pub const COMPACTOR_INSTRUCTION: &str = "__compactor__";

/// Hidden command that opens a fresh session with the prefill in the editor.
pub const NEW_SESSION_COMMAND: &str = "ask-user-new-session";

/// Delivers at most once (compaction callback vs fallback timer).
#[derive(Clone, Debug, Default)]
pub struct Once(Rc<Cell<bool>>);

impl Once {
    /// `true` the first time only.
    #[must_use]
    pub fn claim(&self) -> bool {
        !self.0.replace(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_choices() {
        let r = launcher_request("/plan x");
        assert_eq!(r["title"], "🚀 /plan x");
        assert_eq!(launcher_request("")["title"], "🚀 (no command)");
        assert_eq!(
            decode_launch(&json!({"value": "🧹 Compact & run"})),
            Launch::Compact
        );
        assert_eq!(
            decode_launch(&json!({"value": "▶ Run directly"})),
            Launch::Direct
        );
        assert_eq!(
            decode_launch(&json!({"value": "✚ New session"})),
            Launch::NewSession
        );
        assert_eq!(decode_launch(&json!({"cancelled": true})), Launch::Cancel);
    }

    #[test]
    fn prefill_and_once() {
        assert_eq!(
            normalize_prefill(Some("  /plan specs:plan.md  ")).as_deref(),
            Some("/plan specs:plan.md")
        );
        assert_eq!(normalize_prefill(Some("   ")), None);
        assert_eq!(normalize_prefill(None), None);
        let once = Once::default();
        let twin = once.clone();
        assert!(once.claim());
        assert!(!twin.claim());
        assert_eq!(
            compact_instructions("go"),
            "__compactor__\nPreparing for new task. Summarize previous work concisely, preserving only what's essential for: go"
        );
    }
}
