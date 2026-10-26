//! Tree navigation with branch summaries (pi `navigateTree`, `getUserMessagesForForking`).

use oino_ai::{AgentMessage, Usage};
use oino_session::{BranchSummaryDraft, EntryId, EntryKind};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, SessionOpError, lock};
use crate::compaction::{
    BranchSummaryOptions, collect_entries_for_branch_summary, generate_branch_summary,
};
use crate::events::SummarizationSource;

/// Options of [`AgentSession::navigate_tree`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateTreeOptions {
    /// Summarise the abandoned branch.
    #[serde(default)]
    pub summarize: bool,
    /// Custom summariser instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_instructions: Option<String>,
    /// Instructions replace the default prompt.
    #[serde(default)]
    pub replace_instructions: bool,
    /// Label for the summary (or the target without summary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// pi `navigateTree` result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigateTreeResult {
    /// Text for the editor when the target was a user message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_text: Option<String>,
    /// Cancelled by an extension or the user.
    pub cancelled: bool,
    /// Summarisation was aborted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted: Option<bool>,
    /// The created branch summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_entry: Option<oino_session::SessionEntry>,
}

/// `{entryId, text}` of a user message (pi `getUserMessagesForForking`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkMessage {
    /// Entry id.
    pub entry_id: String,
    /// Message text.
    pub text: String,
}

impl AgentSession {
    /// User messages of the whole session (pi `getUserMessagesForForking`).
    #[must_use]
    pub fn user_messages_for_forking(&self) -> Vec<ForkMessage> {
        lock(&self.inner.manager)
            .entries()
            .iter()
            .filter_map(|entry| match entry.message() {
                Some(AgentMessage::User(user)) => {
                    let text = user.content.text("");
                    (!text.is_empty()).then(|| ForkMessage {
                        entry_id: entry.id.as_str().to_owned(),
                        text,
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// pi `navigateTree`.
    ///
    /// # Errors
    ///
    /// pi's texts for streaming, compaction, unknown targets and summariser failures.
    #[allow(clippy::too_many_lines, reason = "one-to-one port of pi navigateTree")]
    pub async fn navigate_tree(
        &self,
        target_id: &EntryId,
        options: NavigateTreeOptions,
    ) -> Result<NavigateTreeResult, SessionOpError> {
        if self.is_streaming() {
            return Err(SessionOpError::Failed(
                "Wait for the current response to finish before navigating the session tree."
                    .into(),
            ));
        }
        if self.is_compacting() {
            return Err(SessionOpError::Failed(
                "Wait for the current compaction or tree navigation to finish before navigating the session tree.".into(),
            ));
        }
        let old_leaf = lock(&self.inner.manager).leaf_id().cloned();
        if old_leaf.as_ref() == Some(target_id) {
            return Ok(NavigateTreeResult::default());
        }
        if options.summarize && self.model().is_none() {
            return Err(SessionOpError::Failed(
                "No model available for summarization".into(),
            ));
        }
        let (target, collected) = {
            let manager = lock(&self.inner.manager);
            let target = manager.entry(target_id).cloned().ok_or_else(|| {
                SessionOpError::Failed(format!("Entry {} not found", target_id.as_str()))
            })?;
            let index = manager.index();
            (
                target,
                collect_entries_for_branch_summary(index, old_leaf.as_ref(), target_id),
            )
        };
        let mut custom_instructions = options.custom_instructions.clone();
        let mut replace_instructions = options.replace_instructions;
        let mut label = options.label.clone();
        let cancel = CancellationToken::new();
        lock(&self.inner.cancels).branch_summary = Some(cancel.clone());
        self.mark_busy();
        let result = async {
            let mut extension_summary: Option<(String, Option<serde_json::Value>, Option<Usage>)> =
                None;
            let mut from_extension = false;
            let runner = self.runner();
            if runner.has_handlers("session_before_tree") {
                let mut preparation = json!({
                    "targetId": target_id.as_str(),
                    "oldLeafId": old_leaf.as_ref().map(EntryId::as_str),
                    "commonAncestorId": collected.common_ancestor_id.as_ref().map(EntryId::as_str),
                    "entriesToSummarize": collected.entries,
                    "userWantsSummary": options.summarize,
                });
                if let Some(instructions) = &custom_instructions {
                    preparation["customInstructions"] = json!(instructions);
                }
                if options.replace_instructions {
                    preparation["replaceInstructions"] = json!(true);
                }
                if let Some(label) = &label {
                    preparation["label"] = json!(label);
                }
                if let Some(result) = runner
                    .emit(json!({"type": "session_before_tree", "preparation": preparation}))
                    .await
                {
                    if result.get("cancel") == Some(&serde_json::Value::Bool(true)) {
                        return Ok(NavigateTreeResult {
                            cancelled: true,
                            ..NavigateTreeResult::default()
                        });
                    }
                    if let (Some(summary), true) = (
                        result.get("summary").filter(|s| s.is_object()),
                        options.summarize,
                    ) {
                        extension_summary = Some((
                            summary["summary"].as_str().unwrap_or_default().to_owned(),
                            summary.get("details").cloned(),
                            summary
                                .get("usage")
                                .and_then(|u| serde_json::from_value(u.clone()).ok()),
                        ));
                        from_extension = true;
                    }
                    if let Some(value) = result
                        .get("customInstructions")
                        .and_then(serde_json::Value::as_str)
                    {
                        custom_instructions = Some(value.to_owned());
                    }
                    if let Some(value) = result
                        .get("replaceInstructions")
                        .and_then(serde_json::Value::as_bool)
                    {
                        replace_instructions = value;
                    }
                    if let Some(value) = result.get("label").and_then(serde_json::Value::as_str) {
                        label = Some(value.to_owned());
                    }
                }
            }
            let mut summary: Option<(String, Option<serde_json::Value>, Option<Usage>)> = None;
            if options.summarize && !collected.entries.is_empty() && extension_summary.is_none() {
                let model = self.model().ok_or_else(|| {
                    SessionOpError::Failed("No model available for summarization".into())
                })?;
                let summarizer = self
                    .summarizer(&model, cancel.clone(), SummarizationSource::BranchSummary)
                    .await
                    .map_err(SessionOpError::Failed)?;
                let reserve = lock(&self.inner.settings)
                    .settings()
                    .branch_summary_settings()
                    .reserve_tokens;
                let result = generate_branch_summary(
                    &collected.entries,
                    &summarizer,
                    &BranchSummaryOptions {
                        custom_instructions: custom_instructions.clone(),
                        replace_instructions,
                        reserve_tokens: reserve,
                    },
                )
                .await;
                if result.aborted == Some(true) {
                    return Ok(NavigateTreeResult {
                        cancelled: true,
                        aborted: Some(true),
                        ..NavigateTreeResult::default()
                    });
                }
                if let Some(error) = result.error {
                    return Err(SessionOpError::Failed(error));
                }
                summary = Some((
                    result.summary.unwrap_or_default(),
                    Some(json!({
                        "readFiles": result.read_files.unwrap_or_default(),
                        "modifiedFiles": result.modified_files.unwrap_or_default(),
                    })),
                    result.usage,
                ));
            } else if let Some(extension) = extension_summary {
                summary = Some(extension);
            }
            let (new_leaf, editor_text) = match &target.kind {
                EntryKind::Message {
                    message: AgentMessage::User(user),
                } => (target.parent_id.clone(), Some(user.content.text(""))),
                EntryKind::CustomMessage { content, .. } => {
                    (target.parent_id.clone(), Some(content.text("")))
                }
                _ => (Some(target_id.clone()), None),
            };
            let mut summary_entry = None;
            let summarized = summary
                .as_ref()
                .is_some_and(|(text, _, _)| !text.is_empty());
            {
                let mut manager = lock(&self.inner.manager);
                if let (Some((text, details, usage)), true) = (summary, summarized) {
                    let from_id = old_leaf.clone().unwrap_or_else(|| EntryId::new("root"));
                    let id = manager
                        .branch_with_summary(
                            new_leaf.as_ref(),
                            BranchSummaryDraft {
                                from_id,
                                summary: text,
                                details,
                                from_hook: Some(from_extension),
                                usage,
                            },
                        )
                        .map_err(|e| SessionOpError::Failed(e.to_string()))?;
                    if let Some(label) = &label {
                        manager
                            .append_label_change(&id, Some(label))
                            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
                    }
                    summary_entry = manager.entry(&id).cloned();
                } else {
                    match &new_leaf {
                        None => manager.reset_leaf(),
                        Some(leaf) => manager
                            .set_leaf(leaf)
                            .map_err(|e| SessionOpError::Failed(e.to_string()))?,
                    }
                    if let Some(label) = &label {
                        manager
                            .append_label_change(target_id, Some(label))
                            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
                    }
                }
            }
            self.refresh_context();
            self.restore_tools_from_transcript(&[]);
            let new_leaf_id = lock(&self.inner.manager).leaf_id().cloned();
            let runner = self.runner();
            if runner.has_handlers("session_tree") {
                let mut payload = json!({
                    "type": "session_tree",
                    "newLeafId": new_leaf_id.as_ref().map(EntryId::as_str),
                    "oldLeafId": old_leaf.as_ref().map(EntryId::as_str),
                });
                if let Some(entry) = &summary_entry {
                    payload["summaryEntry"] = json!(entry);
                    payload["fromExtension"] = json!(from_extension);
                }
                runner.emit(payload).await;
            }
            Ok(NavigateTreeResult {
                editor_text,
                cancelled: false,
                aborted: None,
                summary_entry,
            })
        }
        .await;
        lock(&self.inner.cancels).branch_summary = None;
        self.set_idle_if_idle();
        result
    }
}
