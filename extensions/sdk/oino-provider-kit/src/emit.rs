//! The live output message plus the [`ProviderEvent`]s emitted while it is built (pi's
//! `output` object and `stream.push`).

use oino_ai::{
    AssistantContent, AssistantDiagnostic, AssistantMessage, DoneReason, ErrorReason, Model,
    ProviderEvent, StopReason, TimestampMs,
};

/// Output message under construction and pending events.
#[derive(Clone, Debug)]
pub struct Emitter {
    /// pi `output`.
    pub message: AssistantMessage,
    events: Vec<ProviderEvent>,
}

impl Emitter {
    /// pi's initial `output` for `model`.
    #[must_use]
    pub fn new(model: &Model, timestamp: TimestampMs) -> Self {
        Self {
            message: AssistantMessage::empty(&model.api, &model.provider, &model.id, timestamp),
            events: Vec::new(),
        }
    }

    /// Queues an event.
    pub fn push(&mut self, event: ProviderEvent) {
        self.events.push(event);
    }

    /// Takes the queued events.
    pub fn drain(&mut self) -> Vec<ProviderEvent> {
        std::mem::take(&mut self.events)
    }

    /// Index the next content block gets.
    #[must_use]
    pub fn next_index(&self) -> usize {
        self.message.content.len()
    }

    /// Appends a block and returns its index.
    pub fn add(&mut self, block: AssistantContent) -> usize {
        self.message.content.push(block);
        self.message.content.len() - 1
    }

    /// Mutable block at `index`.
    pub fn block(&mut self, index: usize) -> Option<&mut AssistantContent> {
        self.message.content.get_mut(index)
    }

    /// pi `appendAssistantMessageDiagnostic`.
    pub fn diagnostic(&mut self, diagnostic: AssistantDiagnostic) {
        self.message
            .diagnostics
            .get_or_insert_with(Vec::new)
            .push(diagnostic);
    }

    /// Queues `start`.
    pub fn start(&mut self) {
        self.push(ProviderEvent::Start {});
    }

    /// Queues `done` with the current message (`stopReason` must be terminal-successful).
    pub fn done(&mut self) {
        let reason = match self.message.stop_reason {
            StopReason::Length => DoneReason::Length,
            StopReason::ToolUse => DoneReason::ToolUse,
            StopReason::Deferred => DoneReason::Deferred,
            _ => DoneReason::Stop,
        };
        self.push(ProviderEvent::Done {
            reason,
            message: self.message.clone(),
        });
    }

    /// Queues `error` (pi catch block): marks the message failed with `message`.
    pub fn fail(&mut self, message: String, aborted: bool) {
        let reason = if aborted {
            ErrorReason::Aborted
        } else {
            ErrorReason::Error
        };
        self.message.stop_reason = reason.into();
        self.message.error_message = Some(message);
        self.push(ProviderEvent::Error {
            reason,
            error: self.message.clone(),
        });
    }
}
