//! Host-only durable admission. The JSONL user entry id is the delivery receipt.
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use oino_ai::AgentMessage;
use serde_json::Value;

use super::durable_queue::{DurableQueue, DurableQueueItem};
use super::{AgentSession, SessionOpError, lock};

// Scoped to the admission future, never a process/session-wide current-command flag.
tokio::task_local! {
    /// Stable host admission identity, scoped to prompt preparation.
    pub static MESSAGE_ID: String;
    /// Independently admitted extension input, never the outer command receipt.
    pub(crate) static EXTENSION_MESSAGE_ID: String;
    /// Command-local deterministic handoff sequence, propagated across runtime jobs.
    pub static COMMAND_HANDOFF: (String, std::sync::Arc<std::sync::Mutex<HashMap<String, usize>>>, std::sync::Arc<std::sync::atomic::AtomicBool>);
}

#[derive(Default)]
pub(crate) struct DeliveryState {
    pub enabled: bool,
    pub items: Vec<DurableQueueItem>,
    pub messages: HashMap<i64, String>,
    pub clock: i64,
}

fn failed(error: impl std::fmt::Display) -> SessionOpError {
    SessionOpError::Failed(error.to_string())
}

impl AgentSession {
    /// Enable durable host admission and return undelivered commands in FIFO order.
    /// Non-host frontends do not opt in and retain their existing behaviour.
    ///
    /// # Errors
    /// A corrupt or unreadable sidecar is preserved and returned as an error.
    pub fn restore_delivery_queue(&self) -> Result<Vec<DurableQueueItem>, SessionOpError> {
        let mut state = lock(&self.inner.delivery);
        if state.enabled {
            return Ok(state.items.clone());
        }
        self.with_manager(oino_session::SessionManager::materialize)
            .map_err(failed)?;
        let Some(path) = self.session_file() else {
            return Err(failed("durable admission requires a session file"));
        };
        let queue = DurableQueue::new(&path);
        let items = match queue.load(&HashSet::default()) {
            Ok(items) => items,
            Err(error) => {
                // Preserve evidence outside the active queue so fresh sends remain usable.
                queue.quarantine().map_err(failed)?;
                state.enabled = true;
                return Err(failed(error));
            }
        };
        let count = items.len();
        let items: Vec<_> = items
            .into_iter()
            .filter(|item| {
                !self.with_manager(|manager| {
                    manager.contains_entry_id(&receipt_id(&item.message_id))
                })
            })
            .collect();
        if items.len() != count {
            queue.store(&items).map_err(failed)?;
        }
        state.items.clone_from(&items);
        state.enabled = true;
        Ok(items)
    }

    /// Quarantine a structurally valid snapshot whose commands cannot be replayed.
    /// # Errors
    /// Rename or sync failed, so the chat must not overwrite the evidence.
    pub fn quarantine_delivery_queue(&self) -> Result<(), SessionOpError> {
        let mut state = lock(&self.inner.delivery);
        let path = self
            .session_file()
            .ok_or_else(|| failed("session file missing"))?;
        DurableQueue::new(&path).quarantine().map_err(failed)?;
        state.items.clear();
        state.enabled = true;
        Ok(())
    }

    /// Persist a host command before dispatch/ACK. Returns false for a duplicate id.
    ///
    /// # Errors
    /// Disk write or sync failures prevent acceptance.
    pub fn admit_delivery(&self, id: &str, command: Value) -> Result<bool, SessionOpError> {
        let mut state = lock(&self.inner.delivery);
        if !state.enabled {
            return Ok(true);
        }
        if state.items.iter().any(|item| item.message_id == id)
            || self.with_manager(|manager| manager.contains_entry_id(&receipt_id(id)))
        {
            return Ok(false);
        }
        let mut items = state.items.clone();
        items.push(DurableQueueItem {
            message_id: id.to_owned(),
            command,
        });
        let started = std::time::Instant::now();
        self.store_delivery(&items)?;
        tracing::info!(
            message_id = id,
            sync_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            "durable queue admission synced"
        );
        state.items = items;
        Ok(true)
    }

    /// Persist clearing all accepted host commands before clearing the live queues.
    ///
    /// # Errors
    /// Disk removal or directory sync failure.
    pub fn clear_delivery_queue(&self) -> Result<(), SessionOpError> {
        let mut state = lock(&self.inner.delivery);
        if state.enabled {
            self.store_delivery(&[])?;
            state.items.clear();
        }
        Ok(())
    }

    pub(crate) fn store_delivery(&self, items: &[DurableQueueItem]) -> Result<(), SessionOpError> {
        let path = self
            .session_file()
            .ok_or_else(|| failed("session file missing"))?;
        DurableQueue::new(&path).store(items).map_err(failed)
    }

    /// Remove a command consumed by an input/command hook (no user entry will follow).
    ///
    /// # Errors
    /// Disk write or sync failures.
    pub fn finish_delivery(&self, id: &str) -> Result<(), SessionOpError> {
        let mut state = lock(&self.inner.delivery);
        if !state.enabled {
            return Ok(());
        }
        let mut items = state.items.clone();
        items.retain(|item| item.message_id != id);
        self.store_delivery(&items)?;
        state.items = items;
        Ok(())
    }

    pub(crate) fn identify_delivery(&self, message: &mut AgentMessage) {
        let Ok(id) = MESSAGE_ID.try_with(Clone::clone) else {
            return;
        };
        let AgentMessage::User(user) = message else {
            return;
        };
        let mut state = lock(&self.inner.delivery);
        if !state.enabled || id.is_empty() || !state.items.iter().any(|item| item.message_id == id)
        {
            return;
        }
        state.clock = (state.clock + 1).max(user.timestamp);
        user.timestamp = state.clock;
        state.messages.insert(user.timestamp, id);
    }
}

pub(crate) fn receipt_id(id: &str) -> oino_session::EntryId {
    oino_session::EntryId::new(id.to_owned())
}

impl AgentSession {
    /// Persist an extension handoff before its caller can acknowledge the command.
    /// Identity is payload plus occurrence among equal payloads, independent of replay order.
    pub(crate) fn admit_extension_handoff(
        &self,
        content: &oino_ai::UserContentField,
        deliver_as: Option<crate::extensions::host::Delivery>,
    ) -> Result<Option<(String, bool)>, SessionOpError> {
        let Ok((parent, sequence, failed)) = COMMAND_HANDOFF.try_with(Clone::clone) else {
            return Ok(None);
        };
        if parent.is_empty() || !lock(&self.inner.delivery).enabled {
            return Ok(None);
        }
        let (text, images) = match content {
            oino_ai::UserContentField::Text(text) => (text.clone(), Vec::new()),
            oino_ai::UserContentField::Blocks(blocks) => {
                let mut text = Vec::new();
                let mut images = Vec::new();
                for block in blocks {
                    match block {
                        oino_ai::UserContent::Text(value) => text.push(value.text.clone()),
                        oino_ai::UserContent::Image(value) => images.push(serde_json::json!({
                            "type":"image", "data":value.data,"mimeType":value.mime_type
                        })),
                    }
                }
                (text.join("\n"), images)
            }
        };
        let kind = if matches!(deliver_as, Some(crate::extensions::host::Delivery::Steer)) {
            "steer"
        } else {
            "follow_up"
        };
        let command = serde_json::json!({"type":kind,"message":text,"images":images});
        let hash = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&command)
                    .map_err(|error| SessionOpError::Failed(error.to_string()))?
            )
        );
        let occurrence = {
            let mut counts = lock(&sequence);
            let count = counts.entry(hash.clone()).or_default();
            let occurrence = *count;
            *count += 1;
            occurrence
        };
        let id = format!("{parent}:extension:{hash}:{occurrence}");
        let fresh = self
            .admit_delivery(&id, command)
            .inspect_err(|_| failed.store(true, std::sync::atomic::Ordering::Release))?;
        Ok(Some((id, fresh)))
    }
}
