//! Single-item queue edits (`_oino/queue_edit`, Oino addition): remove, edit, reorder and
//! send-now for one queued steering / follow-up message.
//!
//! The session keeps the queued texts (`queue_update`) and the agent keeps the queued
//! messages; custom messages queued by extensions have no text entry. Each text is matched to
//! its agent message (first unclaimed user message with the same text, in order), and both
//! lists are edited together.

use oino_ai::{AgentMessage, ImageContent, UserContent, UserContentField, UserMessage};
use serde::{Deserialize, Serialize};

use super::prompt::{InputSource, PromptOptions};
use super::{AgentSession, SessionOpError, lock};

/// Which queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QueueKind {
    /// Steering (delivered after the current tool calls).
    Steering,
    /// Follow-up (delivered when the agent would stop).
    FollowUp,
}

/// One queue edit; `index` is the position in the `queue_update` list of `queue`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum QueueEdit {
    /// Resolve a durable host message identity before applying a legacy-shaped edit.
    ById {
        /// Host-assigned identity returned in the admission ACK.
        #[serde(rename = "messageId")]
        message_id: String,
        /// Edit whose source index is replaced by the resolved identity.
        edit: Box<QueueEdit>,
    },
    /// Drops the item.
    Remove {
        /// Queue.
        queue: QueueKind,
        /// Item.
        index: usize,
    },
    /// Replaces the item's text (images are kept; prompt templates expand).
    Edit {
        /// Queue.
        queue: QueueKind,
        /// Item.
        index: usize,
        /// New text.
        text: String,
    },
    /// Moves the item to position `to` of the same queue.
    Move {
        /// Queue.
        queue: QueueKind,
        /// Item.
        from: usize,
        /// Target position.
        to: usize,
    },
    /// Delivers the item now: while running it becomes the first steering message; when
    /// idle it is removed and prompted.
    SendNow {
        /// Queue.
        queue: QueueKind,
        /// Item.
        index: usize,
    },
}

fn user_text(message: &AgentMessage) -> Option<String> {
    match message {
        AgentMessage::User(user) => Some(user.content.text("")),
        _ => None,
    }
}

fn user_images(message: &AgentMessage) -> Vec<ImageContent> {
    match message {
        AgentMessage::User(UserMessage {
            author: None,
            content: UserContentField::Blocks(blocks),
            ..
        }) => blocks
            .iter()
            .filter_map(|block| match block {
                UserContent::Image(image) => Some(image.clone()),
                UserContent::Text(_) => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Agent queue index of every text (`None` when the message was already drained).
fn match_texts(texts: &[String], messages: &[AgentMessage]) -> Vec<Option<usize>> {
    let mut claimed = vec![false; messages.len()];
    texts
        .iter()
        .map(|text| {
            let found = messages.iter().enumerate().position(|(i, message)| {
                !claimed[i] && user_text(message).as_deref() == Some(text.as_str())
            });
            if let Some(i) = found {
                claimed[i] = true;
            }
            found
        })
        .collect()
}

fn out_of_range(index: usize, len: usize) -> SessionOpError {
    SessionOpError::Failed(format!("Queue index {index} out of range ({len} queued)"))
}

impl AgentSession {
    /// Applies one queue edit and publishes `queue_update`.
    ///
    /// # Errors
    ///
    /// [`SessionOpError::Failed`] for an index out of range; prompt errors for send-now while
    /// idle.
    pub async fn edit_queue(&self, edit: QueueEdit) -> Result<(), SessionOpError> {
        let expected_id = match &edit {
            QueueEdit::ById { message_id, .. } => Some(message_id.clone()),
            _ => None,
        };
        let edit = if let QueueEdit::ById { message_id, edit } = edit {
            let mut edit = *edit;
            let queue = match &edit {
                QueueEdit::Remove { queue, .. }
                | QueueEdit::Edit { queue, .. }
                | QueueEdit::Move { queue, .. }
                | QueueEdit::SendNow { queue, .. } => *queue,
                QueueEdit::ById { .. } => {
                    return Err(SessionOpError::Failed("Nested identity edit".into()));
                }
            };
            let ids = self.delivery_queue_ids(queue);
            let index = ids
                .iter()
                .position(|id| id.as_deref() == Some(&message_id))
                .ok_or_else(|| SessionOpError::Failed("Message is no longer queued".into()))?;
            match &mut edit {
                QueueEdit::Remove { index: at, .. }
                | QueueEdit::Edit { index: at, .. }
                | QueueEdit::SendNow { index: at, .. } => *at = index,
                QueueEdit::Move { from, .. } => *from = index,
                QueueEdit::ById { .. } => unreachable!(),
            }
            edit
        } else {
            edit
        };
        if let QueueEdit::SendNow { queue, index } = edit {
            return self
                .send_queued_now(queue, index, expected_id.as_deref())
                .await;
        }
        let edit = match edit {
            QueueEdit::Edit { queue, index, text } => QueueEdit::Edit {
                queue,
                index,
                text: self.expand_queued_text(&text),
            },
            other => other,
        };
        self.apply_queue_edit(&edit, expected_id.as_deref())?;
        self.emit_queue_update();
        Ok(())
    }

    /// Durable identities aligned with the visible queue; non-host items have no identity.
    #[must_use]
    pub fn delivery_queue_ids(&self, queue: QueueKind) -> Vec<Option<String>> {
        let delivery = lock(&self.inner.delivery);
        if !delivery.enabled {
            return vec![];
        }
        let book = lock(&self.inner.book);
        let steering = queue == QueueKind::Steering;
        let texts = if steering {
            &book.steering
        } else {
            &book.follow_up
        };
        self.inner.agent.update_queue(steering, |messages| {
            match_texts(texts, messages)
                .into_iter()
                .map(|at| {
                    at.and_then(|at| match &messages[at] {
                        AgentMessage::User(user) => delivery.messages.get(&user.timestamp).cloned(),
                        _ => None,
                    })
                })
                .collect()
        })
    }

    fn expand_queued_text(&self, text: &str) -> String {
        let resources = self
            .inner
            .resources
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        oino_resources::templates::expand_prompt_template(text, resources.prompts())
            .unwrap_or_else(|| text.to_owned())
    }

    fn apply_queue_edit(
        &self,
        edit: &QueueEdit,
        expected_id: Option<&str>,
    ) -> Result<(), SessionOpError> {
        let queue = match edit {
            QueueEdit::Remove { queue, .. }
            | QueueEdit::Edit { queue, .. }
            | QueueEdit::Move { queue, .. }
            | QueueEdit::SendNow { queue, .. } => *queue,
            QueueEdit::ById { .. } => unreachable!("identity resolved before mutation"),
        };
        let steering = queue == QueueKind::Steering;
        let mut delivery = lock(&self.inner.delivery);
        let mut book = lock(&self.inner.book);
        let texts = if steering {
            &mut book.steering
        } else {
            &mut book.follow_up
        };
        self.inner.agent.update_queue(steering, |messages| {
            let matches = match_texts(texts, messages);
            // Persist the proposed edit while the agent queue is locked. A drained
            // message is no longer editable, even if MessageEnd has not arrived yet.
            if delivery.enabled {
                let ids: Vec<_> = matches
                    .iter()
                    .map(|at| {
                        at.and_then(|at| match &messages[at] {
                            AgentMessage::User(user) => {
                                delivery.messages.get(&user.timestamp).cloned()
                            }
                            _ => None,
                        })
                    })
                    .collect();
                if let Some(expected) = expected_id {
                    let index = match edit {
                        QueueEdit::Remove { index, .. }
                        | QueueEdit::Edit { index, .. }
                        | QueueEdit::SendNow { index, .. } => *index,
                        QueueEdit::Move { from, .. } => *from,
                        QueueEdit::ById { .. } => unreachable!(),
                    };
                    if ids.get(index).and_then(|id| id.as_deref()) != Some(expected) {
                        return Err(SessionOpError::Failed(
                            "Queue changed; refresh before editing".into(),
                        ));
                    }
                }
                self.edit_delivery_items(edit, &matches, &ids, texts.len(), &mut delivery)?;
            }
            match edit {
                QueueEdit::ById { .. } => unreachable!("identity resolved"),
                QueueEdit::Remove { index, .. } | QueueEdit::SendNow { index, .. } => {
                    if *index >= texts.len() {
                        return Err(out_of_range(*index, texts.len()));
                    }
                    texts.remove(*index);
                    if let Some(at) = matches[*index] {
                        messages.remove(at);
                    }
                }
                QueueEdit::Edit { index, text, .. } => {
                    if *index >= texts.len() {
                        return Err(out_of_range(*index, texts.len()));
                    }
                    texts[*index].clone_from(text);
                    if let Some(at) = matches[*index] {
                        let images = user_images(&messages[at]);
                        let mut content = vec![UserContent::text(text.clone())];
                        content.extend(images.into_iter().map(UserContent::Image));
                        if let AgentMessage::User(user) = &mut messages[at] {
                            user.content = UserContentField::Blocks(content);
                        }
                    }
                }
                QueueEdit::Move { from, to, .. } => {
                    let len = texts.len();
                    if *from >= len || *to >= len {
                        return Err(out_of_range((*from).max(*to), len));
                    }
                    let item = texts.remove(*from);
                    texts.insert(*to, item);
                    let mut slots: Vec<usize> = matches.iter().flatten().copied().collect();
                    if slots.len() == len {
                        let mut ordered: Vec<AgentMessage> = matches
                            .iter()
                            .flatten()
                            .map(|&i| messages[i].clone())
                            .collect();
                        let moved = ordered.remove(*from);
                        ordered.insert(*to, moved);
                        slots.sort_unstable();
                        for (slot, message) in slots.into_iter().zip(ordered) {
                            messages[slot] = message;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn edit_delivery_items(
        &self,
        edit: &QueueEdit,
        matches: &[Option<usize>],
        ids: &[Option<String>],
        text_count: usize,
        delivery: &mut super::delivery::DeliveryState,
    ) -> Result<(), SessionOpError> {
        let mut items = delivery.items.clone();
        let position = |index: usize| -> Result<Option<usize>, SessionOpError> {
            if index >= text_count {
                return Err(out_of_range(index, text_count));
            }
            if matches[index].is_none() {
                return Err(SessionOpError::Failed(
                    "Message is already being delivered".into(),
                ));
            }
            Ok(ids[index]
                .as_ref()
                .and_then(|id| items.iter().position(|item| &item.message_id == id)))
        };
        match edit {
            QueueEdit::ById { .. } => unreachable!("identity resolved"),
            QueueEdit::Remove { index, .. } => {
                if let Some(at) = position(*index)? {
                    items.remove(at);
                }
            }
            QueueEdit::Edit { index, text, .. } => {
                if let Some(at) = position(*index)? {
                    items[at].command["message"] = serde_json::json!(text);
                }
            }
            QueueEdit::Move { from, to, .. } => {
                let a = position(*from)?;
                let b = position(*to)?;
                if let (Some(a), Some(b)) = (a, b) {
                    let item = items.remove(a);
                    items.insert(b, item);
                }
            }
            QueueEdit::SendNow { index, .. } => {
                if let Some(at) = position(*index)? {
                    let mut item = items.remove(at);
                    item.command["type"] = serde_json::json!("steer");
                    items.insert(0, item);
                }
            }
        }
        self.store_delivery(&items)?;
        delivery.items = items;
        Ok(())
    }

    async fn send_queued_now(
        &self,
        queue: QueueKind,
        index: usize,
        expected_id: Option<&str>,
    ) -> Result<(), SessionOpError> {
        let steering = queue == QueueKind::Steering;
        let taken = {
            let book = lock(&self.inner.book);
            let texts = if steering {
                &book.steering
            } else {
                &book.follow_up
            };
            let text = texts
                .get(index)
                .cloned()
                .ok_or_else(|| out_of_range(index, texts.len()))?;
            let message = self.inner.agent.update_queue(steering, |messages| {
                match_texts(texts, messages)[index].map(|at| messages[at].clone())
            });
            (text, message)
        };
        let (text, message) = taken;
        self.apply_queue_edit(&QueueEdit::SendNow { queue, index }, expected_id)?;
        if self.is_streaming() {
            let message = message.unwrap_or_else(|| {
                AgentMessage::User(UserMessage {
                    author: None,
                    content: UserContentField::Blocks(vec![UserContent::text(text.clone())]),
                    timestamp: oino_agent::now_ms(),
                })
            });
            lock(&self.inner.book).steering.insert(0, text);
            self.inner
                .agent
                .update_queue(true, |messages| messages.insert(0, message));
            self.emit_queue_update();
            return Ok(());
        }
        self.emit_queue_update();
        let images = message.as_ref().map(user_images).unwrap_or_default();
        let id = message
            .as_ref()
            .and_then(|message| match message {
                AgentMessage::User(user) => lock(&self.inner.delivery)
                    .messages
                    .get(&user.timestamp)
                    .cloned(),
                _ => None,
            })
            .unwrap_or_default();
        super::delivery::MESSAGE_ID
            .scope(
                id,
                self.prompt(
                    text,
                    PromptOptions {
                        images,
                        source: InputSource::Rpc,
                        ..PromptOptions::default()
                    },
                ),
            )
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            author: None,
            content: UserContentField::Blocks(vec![UserContent::text(text.to_owned())]),
            timestamp: 0,
        })
    }

    #[test]
    fn texts_match_user_messages_in_order() {
        let messages = vec![user("a"), user("b"), user("a")];
        let texts = vec!["a".to_owned(), "a".to_owned(), "c".to_owned()];
        assert_eq!(match_texts(&texts, &messages), vec![Some(0), Some(2), None]);
    }

    #[test]
    fn edits_round_trip_as_json() {
        let edit: QueueEdit = serde_json::from_value(
            serde_json::json!({"op": "move", "queue": "followUp", "from": 1, "to": 0}),
        )
        .unwrap();
        assert_eq!(
            edit,
            QueueEdit::Move {
                queue: QueueKind::FollowUp,
                from: 1,
                to: 0
            }
        );
    }
}
