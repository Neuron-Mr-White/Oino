//! Inter-package service conventions (unipi-port §2.1, §2.2).
//!
//! unipi's `Symbol.for` registries become services: a package that *contributes* registers a
//! service under one of the fan-out names below; the package that *consumes* calls every
//! provider through [`crate::fanout`]. Point services (`subagents.list`, `fusion.status`, …)
//! are called on their owning extension. Payload types serialise to the JSON both sides use.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fan-out service names (every provider is called).
pub mod fanout {
    /// Long-horizon verifier evidence → [`super::EvidenceContribution`].
    pub const EVIDENCE_CONTRIBUTE: &str = "evidence.contribute";
    /// `[Active Work]` blocks for compaction summaries → [`super::CompactionContext`].
    pub const COMPACTION_CONTEXT_PROVIDE: &str = "compaction-context.provide";
    /// Hide a skill → [`super::SkillConditionReply`] for [`super::SkillConditionRequest`].
    pub const SKILL_CONDITION_CHECK: &str = "skill-condition.check";
    /// `/unipi:move` state migration → [`super::MoveReport`] for [`super::MoveRequest`].
    pub const MOVE_HANDLE: &str = "move.handle";
    /// Work-tray tabs → `Vec<`[`crate::tray::TrayTab`]`>` for [`crate::tray::TabRequest`].
    pub const WORK_TRAY_TAB: &str = "work-tray.tab";
    /// Info-screen page/group specs (JSON owned by info-screen).
    pub const INFO_SCREEN_GROUPS: &str = "info-screen.groups";
}

/// Point service names (served by one package).
pub mod point {
    /// `unipi.core`: open the work tray `{tab?, initialId?}`.
    pub const CORE_OPEN_TRAY: &str = "core.open-tray";
    /// Tab owner: a work-tray panel action → [`crate::tray::ActionReply`].
    pub const WORK_TRAY_ACTION: &str = "work-tray.action";
    /// `unipi.background-tasks`: render the completion notification.
    pub const TASKS_NOTIFICATION: &str = "tasks.notification";
    /// `unipi.subagents`: list of runs.
    pub const SUBAGENTS_LIST: &str = "subagents.list";
    /// `unipi.subagents`: transcript `{id}`.
    pub const SUBAGENTS_TRANSCRIPT: &str = "subagents.transcript";
    /// `unipi.subagents`: cancel `{id}`.
    pub const SUBAGENTS_CANCEL: &str = "subagents.cancel";
    /// `unipi.subagents`: foreground `{id}`.
    pub const SUBAGENTS_FOREGROUND: &str = "subagents.foreground";
    /// `unipi.subagents`: graph step → `{runId}`.
    pub const SUBAGENTS_GRAPH_RUN: &str = "subagents.graph-run";
    /// `unipi.subagents`: register an external reader (fusion sidekick).
    pub const SUBAGENTS_REGISTER_EXTERNAL: &str = "subagents.register-external";
    /// `unipi.fusion`: status.
    pub const FUSION_STATUS: &str = "fusion.status";
    /// `unipi.fusion`: set the lead/sidekick pair.
    pub const FUSION_SET_PAIR: &str = "fusion.set-pair";
    /// `unipi.btw`: ask `{question}`.
    pub const BTW_ASK: &str = "btw.ask";
    /// `unipi.dream`: status.
    pub const DREAM_STATUS: &str = "dream.status";
    /// `unipi.dream`: run.
    pub const DREAM_RUN: &str = "dream.run";
    /// `unipi.dream`: stop.
    pub const DREAM_STOP: &str = "dream.stop";
    /// `unipi.kanboard`: capture `{title, body?, priority?}`.
    pub const KANBOARD_CAPTURE: &str = "kanboard.capture";
}

/// Evidence for the long-horizon verifier (unipi `EvidenceContribution`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceContribution {
    /// Items that force a `not_met` verdict.
    #[serde(default)]
    pub blocking: Vec<String>,
    /// Context notes for the evidence brief.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl EvidenceContribution {
    /// Appends another contribution.
    pub fn merge(&mut self, other: Self) {
        self.blocking.extend(other.blocking);
        self.notes.extend(other.notes);
    }
}

/// One `[Active Work]` block (unipi `registerCompactionContext(id, () => string|null)`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionContext {
    /// Provider id (`long-horizon`, `kanboard`).
    pub id: String,
    /// Block text; `None` = nothing to add.
    pub text: Option<String>,
}

/// `skill-condition.check` request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillConditionRequest {
    /// Skill name.
    pub skill: String,
}

/// `skill-condition.check` reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillConditionReply {
    /// Hide the skill.
    pub hidden: bool,
}

/// `move.handle` request (unipi move handler input minus the `log` callback).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRequest {
    /// Old project root.
    pub old_root: String,
    /// New project root.
    pub new_root: String,
    /// Report only.
    pub dry_run: bool,
    /// Keep a backup of rewritten state.
    pub backup: bool,
}

/// `move.handle` reply: the log lines unipi handlers wrote through `log`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveReport {
    /// Log lines.
    #[serde(default)]
    pub log: Vec<String>,
}

/// `core.open-tray` request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTray {
    /// Tab to open on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    /// Item to open directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_id: Option<String>,
}

/// Decodes a service reply, treating `null`/mismatches as `None`.
#[must_use]
pub fn decode<T: for<'de> Deserialize<'de>>(value: Value) -> Option<T> {
    if value.is_null() {
        return None;
    }
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn shapes() {
        let mut e = EvidenceContribution {
            blocking: vec!["a".into()],
            notes: vec![],
        };
        e.merge(EvidenceContribution {
            blocking: vec!["b".into()],
            notes: vec!["n".into()],
        });
        assert_eq!(e.blocking, ["a", "b"]);
        assert_eq!(e.notes, ["n"]);
        assert_eq!(
            serde_json::to_value(MoveRequest {
                old_root: "/a".into(),
                new_root: "/b".into(),
                dry_run: true,
                backup: false
            })
            .unwrap_or_default(),
            json!({"oldRoot": "/a", "newRoot": "/b", "dryRun": true, "backup": false})
        );
        assert_eq!(
            decode::<EvidenceContribution>(json!({"blocking": ["x"]})).map(|e| e.blocking),
            Some(vec!["x".to_owned()])
        );
        assert_eq!(decode::<SkillConditionReply>(Value::Null), None);
        assert_eq!(
            decode::<OpenTray>(json!({"tab": "bg"})).and_then(|o| o.tab),
            Some("bg".into())
        );
    }
}
