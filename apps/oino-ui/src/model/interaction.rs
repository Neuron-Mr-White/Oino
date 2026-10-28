//! Docked interactions (design.md §4.12–§4.15, §5.3): how a pending dialog renders (docked
//! card or modal), plan-review extraction from workflow `custom` dialogs, the destructive
//! command guard and timeout countdowns. Toolkit-free.

use crate::model::dialog::{Dialog, DialogKind};
use crate::model::ous::{OusNode, Tone, plain};

/// Where and how a dialog renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionKind {
    /// `approval` → `ApprovalCard` (design.md §4.12), docked.
    Approval,
    /// `form` → `QuestionCard`, docked.
    Question,
    /// Workflow plan review (`custom` with markdown + actions) → `PlanReviewCard`, docked.
    PlanReview,
    /// `select` / `confirm` / `input` / `editor` / `multiSelect` → `DockDialog`, docked.
    Dock,
    /// Other `custom` dialogs → modal dialog with an OUS body.
    Modal,
    /// Unsupported methods (answered as cancelled by the user).
    Unsupported,
}

/// Classifies `dialog` (design.md §3.3 table).
#[must_use]
pub fn kind(dialog: &Dialog) -> InteractionKind {
    match &dialog.kind {
        DialogKind::Approval { .. } => InteractionKind::Approval,
        DialogKind::Form { .. } => InteractionKind::Question,
        DialogKind::Custom { .. } if plan_review(dialog).is_some() => InteractionKind::PlanReview,
        DialogKind::Custom { .. } => InteractionKind::Modal,
        DialogKind::Select { .. }
        | DialogKind::Confirm { .. }
        | DialogKind::Input { .. }
        | DialogKind::Editor { .. }
        | DialogKind::MultiSelect { .. } => InteractionKind::Dock,
        DialogKind::Unsupported => InteractionKind::Unsupported,
    }
}

/// One action of a plan review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanAction {
    /// Action id answered with `custom{actionId}`.
    pub action_id: String,
    /// Label without the number prefix.
    pub label: String,
    /// Primary button.
    pub primary: bool,
    /// Destructive.
    pub danger: bool,
}

/// A workflow plan review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanReview {
    /// The plan as markdown (sections joined under `##` headings).
    pub markdown: String,
    /// Plan file path from the title, when present.
    pub path: Option<String>,
    /// Decisions in order.
    pub actions: Vec<PlanAction>,
}

fn collect(node: &OusNode, md: &mut Vec<String>, buttons: &mut Vec<PlanAction>) {
    match node {
        OusNode::Markdown { value } => md.push(value.clone()),
        OusNode::Section { title, body, .. } => {
            let mut inner = Vec::new();
            collect(body, &mut inner, buttons);
            if !inner.is_empty() {
                md.push(format!("## {}\n\n{}", plain(title), inner.join("\n\n")));
            }
        }
        OusNode::Stack { children, .. } | OusNode::Split { children, .. } => {
            for c in children {
                collect(c, md, buttons);
            }
        }
        OusNode::Card { body, .. } => collect(body, md, buttons),
        OusNode::Button {
            label,
            action_id,
            tone,
            primary,
        } => {
            let label = label
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim()
                .to_owned();
            buttons.push(PlanAction {
                action_id: action_id.clone(),
                label,
                primary: *primary,
                danger: *tone == Tone::Error,
            });
        }
        _ => {}
    }
}

/// Extracts a plan review from a workflow `custom` dialog: markdown plus ≥ 2 buttons, from
/// `unipi.workflow` or titled "Review plan…".
#[must_use]
pub fn plan_review(dialog: &Dialog) -> Option<PlanReview> {
    let DialogKind::Custom { spec } = &dialog.kind else {
        return None;
    };
    let from_workflow = dialog.extension_id.as_deref() == Some("unipi.workflow")
        || dialog.title.starts_with("Review plan");
    if !from_workflow {
        return None;
    }
    let mut md = Vec::new();
    let mut actions = Vec::new();
    collect(spec, &mut md, &mut actions);
    if md.is_empty() || actions.len() < 2 {
        return None;
    }
    let path = dialog
        .title
        .strip_prefix("Review plan")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    Some(PlanReview {
        markdown: md.join("\n\n"),
        path,
        actions,
    })
}

/// Destructive command patterns (design.md §4.12).
const DANGEROUS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "rm -r ",
    "git clean -f",
    "git clean -df",
    "git clean -fd",
    "git reset --hard",
    "git push --force",
    "git push -f",
    "push --force",
    "mkfs",
    "dd if=",
    "robocopy /mir",
    "format c:",
    "del /s",
    "rd /s",
    "remove-item -recurse",
    ":(){",
    "chmod -r 777 /",
    "> /dev/sd",
    "shred ",
    "drop table",
    "drop database",
];

/// Whether `command` looks destructive.
#[must_use]
pub fn is_dangerous(command: &str) -> bool {
    let lower = command.to_lowercase();
    DANGEROUS.iter().any(|p| lower.contains(p))
}

/// Whether an approval dialog is for a destructive command (summary or code detail).
#[must_use]
pub fn approval_is_dangerous(dialog: &Dialog) -> bool {
    let DialogKind::Approval {
        summary, detail, ..
    } = &dialog.kind
    else {
        return false;
    };
    is_dangerous(summary)
        || detail
            .as_ref()
            .is_some_and(|d| is_dangerous(&d.to_lines().join("\n")))
}

/// Remaining time of a dialog with `timeout_ms` after `elapsed_ms`, as `m:ss`.
#[must_use]
pub fn countdown(timeout_ms: Option<u64>, elapsed_ms: u64) -> Option<(String, u64)> {
    let left = timeout_ms?.saturating_sub(elapsed_ms);
    let secs = left.div_ceil(1000);
    Some((format!("{}:{:02}", secs / 60, secs % 60), left))
}

/// "1 of 3" when more than one interaction is pending.
#[must_use]
pub fn queue_label(count: usize) -> Option<String> {
    (count > 1).then(|| format!("1 of {count}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_core::ui::UiRequest;
    use serde_json::json;

    fn dialog(value: serde_json::Value) -> Dialog {
        let mut v = value;
        v["type"] = json!("extension_ui_request");
        v["id"] = json!("d");
        let req: UiRequest = serde_json::from_value(v).unwrap();
        Dialog::parse(&req).unwrap()
    }

    #[test]
    fn classifies_dialogs() {
        assert_eq!(
            kind(&dialog(
                json!({"method":"confirm","title":"t","message":"m"})
            )),
            InteractionKind::Dock
        );
        assert_eq!(
            kind(&dialog(json!({"method":"form","title":"q","questions":[]}))),
            InteractionKind::Question
        );
        assert_eq!(
            kind(&dialog(
                json!({"method":"custom","title":"x","spec":{"t":"text","value":"v"}})
            )),
            InteractionKind::Modal
        );
    }

    #[test]
    fn extracts_workflow_plan_review() {
        let d = dialog(
            json!({"method":"custom","title":"Review plan  .oino/plans/a.md","extensionId":"unipi.workflow",
            "spec":{"t":"stack","children":[
                {"t":"split","children":[
                    {"t":"section","title":"Summary","body":{"t":"markdown","value":"Do it"}},
                    {"t":"section","title":"Implementation","body":{"t":"markdown","value":"1. step"}}]},
                {"t":"stack","direction":"horizontal","children":[
                    {"t":"button","label":"1 Approve & implement","actionId":"approve","primary":true},
                    {"t":"button","label":"2 Keep planning…","actionId":"keep"},
                    {"t":"button","label":"3 Discard plan","actionId":"discard","tone":"error"}]}]}}),
        );
        assert_eq!(kind(&d), InteractionKind::PlanReview);
        let review = plan_review(&d).unwrap();
        assert_eq!(review.path.as_deref(), Some(".oino/plans/a.md"));
        assert!(review.markdown.contains("## Summary\n\nDo it"));
        assert_eq!(review.actions.len(), 3);
        assert_eq!(review.actions[0].label, "Approve & implement");
        assert!(review.actions[0].primary);
        assert!(review.actions[2].danger);
    }

    #[test]
    fn danger_guard_and_countdown() {
        assert!(is_dangerous("sudo rm -rf build"));
        assert!(is_dangerous("git push --force origin main"));
        assert!(!is_dangerous("cargo test parser"));
        let d = dialog(
            json!({"method":"approval","title":"Run?","summary":"git clean -fdx","options":[]}),
        );
        assert!(approval_is_dangerous(&d));
        assert_eq!(
            countdown(Some(42_000), 0),
            Some(("0:42".to_owned(), 42_000))
        );
        assert_eq!(countdown(Some(1_000), 5_000).unwrap().1, 0);
        assert_eq!(countdown(None, 0), None);
        assert_eq!(queue_label(3).as_deref(), Some("1 of 3"));
        assert_eq!(queue_label(1), None);
    }
}
