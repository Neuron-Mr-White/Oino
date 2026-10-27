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
        if let QueueEdit::SendNow { queue, index } = edit {
            return self.send_queued_now(queue, index).await;
        }
        let edit = match edit {
            QueueEdit::Edit { queue, index, text } => QueueEdit::Edit {
                queue,
                index,
                text: self.expand_queued_text(&text),
            },
            other => other,
        };
        self.apply_queue_edit(&edit)?;
        self.emit_queue_update();
        Ok(())
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

    fn apply_queue_edit(&self, edit: &QueueEdit) -> Result<(), SessionOpError> {
        let queue = match edit {
            QueueEdit::Remove { queue, .. }
            | QueueEdit::Edit { queue, .. }
            | QueueEdit::Move { queue, .. }
            | QueueEdit::SendNow { queue, .. } => *queue,
        };
        let steering = queue == QueueKind::Steering;
        let mut book = lock(&self.inner.book);
        let texts = if steering {
            &mut book.steering
        } else {
            &mut book.follow_up
        };
        self.inner.agent.update_queue(steering, |messages| {
            let matches = match_texts(texts, messages);
            match edit {
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

    async fn send_queued_now(&self, queue: QueueKind, index: usize) -> Result<(), SessionOpError> {
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
        self.apply_queue_edit(&QueueEdit::Remove { queue, index })?;
        if self.is_streaming() {
            let message = message.unwrap_or_else(|| {
                AgentMessage::User(UserMessage {
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
        self.prompt(
            text,
            PromptOptions {
                images,
                source: InputSource::Rpc,
                ..PromptOptions::default()
            },
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
