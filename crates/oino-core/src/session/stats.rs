//! Session statistics, context usage and export (pi `getSessionStats`, `getContextUsage`,
//! `exportToJsonl`, `exportToHtml`).

use std::path::PathBuf;

use oino_ai::{AgentMessage, AssistantContent, StopReason, Usage};
use oino_session::{EntryKind, FileLine, SessionEntry};
use serde::{Deserialize, Serialize};

use super::{AgentSession, SessionOpError, lock};
use crate::compaction::{calculate_context_tokens, estimate_projected_context_tokens};

/// pi `ContextUsage`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Estimated context tokens (`null` right after compaction).
    pub tokens: Option<u64>,
    /// Context window.
    pub context_window: u64,
    /// Percent of the window (`null` with unknown tokens).
    pub percent: Option<f64>,
}

/// Token totals of [`SessionStats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Cache read tokens.
    pub cache_read: u64,
    /// Cache write tokens.
    pub cache_write: u64,
    /// Sum of the four.
    pub total: u64,
}

/// pi `SessionStats`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    /// Session file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// Session id.
    pub session_id: String,
    /// User messages.
    pub user_messages: u64,
    /// Assistant messages.
    pub assistant_messages: u64,
    /// Tool calls.
    pub tool_calls: u64,
    /// Tool results.
    pub tool_results: u64,
    /// Message entries.
    pub total_messages: u64,
    /// Token totals.
    pub tokens: TokenTotals,
    /// Total cost (USD).
    pub cost: f64,
    /// Context usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
}

fn add(totals: &mut TokenTotals, cost: &mut f64, usage: &Usage) {
    totals.input += usage.input;
    totals.output += usage.output;
    totals.cache_read += usage.cache_read;
    totals.cache_write += usage.cache_write;
    *cost += usage.cost.total;
}

impl AgentSession {
    /// pi `getSessionStats` (all entries, including compacted history and `usage` entries).
    #[must_use]
    pub fn session_stats(&self) -> SessionStats {
        let mut stats = SessionStats {
            session_file: self.session_file().map(|p| p.display().to_string()),
            session_id: self.session_id(),
            user_messages: 0,
            assistant_messages: 0,
            tool_calls: 0,
            tool_results: 0,
            total_messages: 0,
            tokens: TokenTotals::default(),
            cost: 0.0,
            context_usage: None,
        };
        let entries = self.entries();
        for entry in &entries {
            match &entry.kind {
                EntryKind::Usage { usage, .. } => add(&mut stats.tokens, &mut stats.cost, usage),
                EntryKind::BranchSummary(summary) => {
                    if let Some(usage) = &summary.usage {
                        add(&mut stats.tokens, &mut stats.cost, usage);
                    }
                }
                EntryKind::Compaction(compaction) => {
                    if let Some(usage) = &compaction.usage {
                        add(&mut stats.tokens, &mut stats.cost, usage);
                    }
                }
                EntryKind::Message { message } => {
                    stats.total_messages += 1;
                    match message {
                        AgentMessage::User(_) => stats.user_messages += 1,
                        AgentMessage::ToolResult(result) => {
                            stats.tool_results += 1;
                            if let Some(usage) = &result.usage {
                                add(&mut stats.tokens, &mut stats.cost, usage);
                            }
                        }
                        AgentMessage::Assistant(assistant) => {
                            stats.assistant_messages += 1;
                            stats.tool_calls += assistant
                                .content
                                .iter()
                                .filter(|b| matches!(b, AssistantContent::ToolCall(_)))
                                .count() as u64;
                            add(&mut stats.tokens, &mut stats.cost, &assistant.usage);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        stats.tokens.total = stats.tokens.input
            + stats.tokens.output
            + stats.tokens.cache_read
            + stats.tokens.cache_write;
        stats.context_usage = self.context_usage();
        stats
    }

    /// pi `getContextUsage`.
    #[must_use]
    pub fn context_usage(&self) -> Option<ContextUsage> {
        let model = self.routed_model().or_else(|| self.model())?;
        let context_window = model.context_window;
        if context_window == 0 {
            return None;
        }
        let manager = lock(&self.inner.manager);
        let projection = manager.build_session_projection();
        let branch = manager.branch(None);
        if let Some(compaction_index) = branch
            .iter()
            .rposition(|e| matches!(e.kind, EntryKind::Compaction(_)))
        {
            let projected: Vec<&oino_session::EntryId> = projection
                .entries
                .iter()
                .filter(|entry| {
                    entry.messages.iter().any(|m| {
                        matches!(m, AgentMessage::Assistant(a) if !matches!(a.stop_reason, StopReason::Aborted | StopReason::Error) && calculate_context_tokens(&a.usage) > 0)
                    })
                })
                .map(|entry| &entry.source_entry.id)
                .collect();
            let has_post = branch[compaction_index + 1..]
                .iter()
                .any(|e| projected.contains(&&e.id));
            if !has_post {
                return Some(ContextUsage {
                    tokens: None,
                    context_window,
                    percent: None,
                });
            }
        }
        let estimate = estimate_projected_context_tokens(&projection, &branch);
        #[allow(
            clippy::cast_precision_loss,
            reason = "percent is approximate, as in pi"
        )]
        let percent = estimate.tokens as f64 / context_window as f64 * 100.0;
        Some(ContextUsage {
            tokens: Some(estimate.tokens),
            context_window,
            percent: Some(percent),
        })
    }

    /// pi `exportToJsonl`: a fresh header plus the current branch, re-linked linearly.
    ///
    /// # Errors
    ///
    /// Serialisation or write failures.
    pub async fn export_jsonl(&self, path: Option<PathBuf>) -> Result<PathBuf, SessionOpError> {
        let now = oino_session::IsoTimestamp::now();
        let lines = {
            let manager = lock(&self.inner.manager);
            let header = oino_session::SessionHeader {
                version: oino_session::CURRENT_SESSION_VERSION,
                id: manager.session_id().to_owned(),
                timestamp: now.clone(),
                cwd: manager.cwd().display().to_string(),
                parent_session: None,
            };
            let mut lines = vec![
                FileLine::Header(header)
                    .to_line()
                    .map_err(|e| SessionOpError::Failed(e.to_string()))?,
            ];
            let mut parent = None;
            for entry in manager.branch(None) {
                let mut entry = entry.clone();
                entry.parent_id = parent.take();
                parent = Some(entry.id.clone());
                lines.push(
                    FileLine::Entry(entry)
                        .to_line()
                        .map_err(|e| SessionOpError::Failed(e.to_string()))?,
                );
            }
            lines
        };
        let default_name = format!("session-{}.jsonl", now.as_str().replace([':', '.'], "-"));
        let target = match path {
            Some(path) if path.is_absolute() => path,
            Some(path) => self.inner.cwd.join(path),
            None => self.inner.cwd.join(default_name),
        };
        if let Some(dir) = target.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        }
        let mut text = lines.join("\n");
        text.push('\n');
        tokio::fs::write(&target, text)
            .await
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        Ok(target)
    }

    /// Exports the current branch as a self-contained HTML page (pi `exportToHtml`
    /// structure: header, messages, tool calls, collapsed thinking, embedded CSS).
    ///
    /// # Errors
    ///
    /// Write failures.
    pub async fn export_html(&self, path: Option<PathBuf>) -> Result<PathBuf, SessionOpError> {
        let id = self.session_id();
        let branch: Vec<SessionEntry> = lock(&self.inner.manager)
            .branch(None)
            .into_iter()
            .cloned()
            .collect();
        let html = crate::export::render_html(&id, self.session_name().as_deref(), &branch);
        let short: String = id.chars().take(8).collect();
        let target =
            path.unwrap_or_else(|| self.inner.cwd.join(format!("oino-session-{short}.html")));
        tokio::fs::write(&target, html)
            .await
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        Ok(target)
    }
}
