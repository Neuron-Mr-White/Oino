//! `AgentSpec` (runtime §12, extension-api §4.3): camelCase JSON as sent by guests.

use std::path::PathBuf;

use oino_ai::{AgentMessage, ThinkingLevel};
use serde::{Deserialize, Serialize};

/// Default nesting allowance of a child (runtime §12).
pub const DEFAULT_MAX_DEPTH: u8 = 2;
/// Hard cap of the nesting depth.
pub const HARD_MAX_DEPTH: u8 = 4;
/// Default concurrent children per root session.
pub const DEFAULT_MAX_CONCURRENT: usize = 8;
/// Hard cap of concurrent children per root session.
pub const HARD_MAX_CONCURRENT: usize = 16;
/// Upper bound of the close grace period (`ChildAgent::close`).
pub const CLOSE_GRACE_CAP: std::time::Duration = std::time::Duration::from_secs(5);

/// How the child's system prompt is built.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SystemPromptSpec {
    /// The default Oino system prompt for the child's tools.
    #[default]
    Default,
    /// Replace the prompt body (pi `--system-prompt`).
    Replace(String),
    /// Append to the default prompt (pi `--append-system-prompt`).
    Append(String),
}

/// Tools of the child.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolPolicy {
    /// Built-in tools (`read`, `bash`, …); `None` = the `defaultTools` setting.
    pub builtin: Option<Vec<String>>,
    /// Extension tool names or `*` patterns (only tools of the child's extensions exist).
    pub extension_tools: Vec<String>,
    /// Names or patterns removed from both lists.
    pub deny: Vec<String>,
}

/// Which extensions the child session runs with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionPolicy {
    /// No extension events or tools (providers still serve requests).
    #[default]
    None,
    /// Every extension of the parent.
    Inherit,
    /// Only these extension ids.
    Only(Vec<String>),
}

/// Initial transcript of the child.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Seed {
    /// Empty session.
    #[default]
    None,
    /// The parent's current branch (its context messages).
    BranchCopy,
    /// Continue an existing child session file (resume).
    SessionFile(PathBuf),
    /// These messages.
    Messages(Vec<AgentMessage>),
}

/// Where the child's session lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Persistence {
    /// Never written.
    #[default]
    Memory,
    /// A session file under `session_dir` (default `<parent session dir>/children/`).
    File {
        /// Sessions root of the child.
        #[serde(default)]
        session_dir: Option<PathBuf>,
    },
}

/// Dialog routing of the child.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UiPolicy {
    /// Dialogs and notifications go to the parent's UI, titles prefixed with `[label] `.
    Forward,
    /// Dialogs are cancelled; approvals are rejected with `note` as feedback.
    AutoDeny {
        /// Reason returned to approval dialogs.
        note: String,
    },
    /// No UI: dialogs are cancelled, notifications dropped.
    #[default]
    None,
}

/// A nested agent request (runtime §12).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentSpec {
    /// Shown in run state, UI prefixes and `child:<label>` usage entries.
    pub label: String,
    /// `provider/id` (default: the parent's model).
    pub model: Option<String>,
    /// Thinking level (default: the parent's), clamped to the model.
    pub thinking: Option<ThinkingLevel>,
    /// System prompt.
    pub system_prompt: SystemPromptSpec,
    /// Tools.
    pub tools: ToolPolicy,
    /// Extensions.
    pub extensions: ExtensionPolicy,
    /// Initial transcript.
    pub seed: Seed,
    /// Persistence.
    pub persistence: Persistence,
    /// Working directory (default: the parent's).
    pub cwd: Option<PathBuf>,
    /// Role visible to extensions as `childRole`.
    pub child_role: Option<String>,
    /// Dialog routing.
    pub ui: UiPolicy,
    /// Nesting allowance of the child (default 2, capped by the parent's and 4).
    pub max_depth: Option<u8>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_guest_specs() {
        let spec: AgentSpec = serde_json::from_value(json!({
            "label": "Subagent", "model": "a/b", "thinking": "high",
            "systemPrompt": {"append": "extra"},
            "tools": {"builtin": ["read"], "extensionTools": ["*"], "deny": ["run_subagent"]},
            "extensions": "inherit", "seed": {"sessionFile": "/s.jsonl"},
            "persistence": {"file": {"sessionDir": "/d"}}, "childRole": "subagent",
            "ui": {"autoDeny": {"note": "no"}}, "maxDepth": 1
        }))
        .unwrap();
        assert_eq!(spec.thinking, Some(ThinkingLevel::High));
        assert_eq!(spec.system_prompt, SystemPromptSpec::Append("extra".into()));
        assert_eq!(spec.tools.builtin, Some(vec!["read".to_owned()]));
        assert_eq!(spec.extensions, ExtensionPolicy::Inherit);
        assert_eq!(spec.seed, Seed::SessionFile("/s.jsonl".into()));
        assert_eq!(
            spec.persistence,
            Persistence::File {
                session_dir: Some("/d".into())
            }
        );
        assert_eq!(spec.ui, UiPolicy::AutoDeny { note: "no".into() });
        let minimal: AgentSpec = serde_json::from_value(json!({
            "label": "btw", "extensions": {"only": ["unipi.memory"]}, "seed": "branchCopy",
            "persistence": "memory", "ui": "forward"
        }))
        .unwrap();
        assert_eq!(
            minimal.extensions,
            ExtensionPolicy::Only(vec!["unipi.memory".into()])
        );
        assert_eq!(minimal.seed, Seed::BranchCopy);
        assert_eq!(minimal.ui, UiPolicy::Forward);
        assert_eq!(minimal.tools.builtin, None);
    }
}
