//! [`SessionModel`]: everything a view shows about one open session (gpui-app §3.2), reduced
//! from the backend's ordered [`ClientEvent`] stream.

use oino_ai::{AgentMessage, Model, ThinkingLevel};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent, RunState};
use oino_core::ui::UiRequest;

use crate::model::transcript::{NoticeTone, RowChange, Transcript};
use crate::model::ui_state::{UiEffect, UiState};

/// Token and cost totals of the session (this process's view).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UsageTotals {
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Cost in USD.
    pub cost: f64,
    /// Context tokens of the last assistant reply (input + cache + output).
    pub context_tokens: u64,
}

/// Result of applying a batch of events (views redraw only what changed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Transcript row changes in order.
    pub rows: Vec<RowChange>,
    /// Header / status / composer chrome changed.
    pub chrome: bool,
    /// Effects to perform.
    pub effects: Vec<UiEffect>,
    /// The session was replaced; reload messages (`get_messages`) and state.
    pub reload: bool,
    /// A run settled (sidebar status, OS notification when unfocused).
    pub settled: bool,
}

impl Applied {
    /// Merges `other` into `self`.
    pub fn merge(&mut self, other: Self) {
        self.rows.extend(other.rows);
        self.chrome |= other.chrome;
        self.effects.extend(other.effects);
        self.reload |= other.reload;
        self.settled |= other.settled;
    }

    /// Nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && !self.chrome && self.effects.is_empty() && !self.reload
    }
}

/// One open session.
#[derive(Clone, Debug)]
pub struct SessionModel {
    /// Session id.
    pub id: String,
    /// Session file.
    pub file: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Working directory.
    pub cwd: String,
    /// Transcript rows.
    pub transcript: Transcript,
    /// Extension UI.
    pub ui: UiState,
    /// Coarse run state.
    pub run: RunState,
    /// Wait reasons while `Waiting`.
    pub waiting_on: Vec<String>,
    /// Selected model.
    pub model: Option<Model>,
    /// Thinking level.
    pub thinking: ThinkingLevel,
    /// Queued steering messages.
    pub steering: Vec<String>,
    /// Queued follow-ups.
    pub follow_up: Vec<String>,
    /// Usage.
    pub usage: UsageTotals,
    /// Extension errors (latest last, capped).
    pub extension_errors: Vec<String>,
    /// Active tool names.
    pub active_tools: Vec<String>,
}

const MAX_ERRORS: usize = 50;

impl SessionModel {
    /// A fresh model for session `id` in `cwd`.
    #[must_use]
    pub fn new(id: impl Into<String>, cwd: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            file: None,
            name: None,
            cwd: cwd.into(),
            transcript: Transcript::new(),
            ui: UiState::new(),
            run: RunState::Idle,
            waiting_on: Vec::new(),
            model: None,
            thinking: ThinkingLevel::Off,
            steering: Vec::new(),
            follow_up: Vec::new(),
            usage: UsageTotals::default(),
            extension_errors: Vec::new(),
            active_tools: Vec::new(),
        }
    }

    /// Whether an agent run (or compaction / retry) is active.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(
            self.run,
            RunState::Running | RunState::Compacting | RunState::Retrying
        )
    }

    /// Title for headers and the sidebar: extension title, name, else first prompt.
    #[must_use]
    pub fn title(&self) -> String {
        if let Some(title) = self.ui.title.as_ref().or(self.name.as_ref()) {
            return title.clone();
        }
        self.transcript
            .rows()
            .iter()
            .find_map(|r| match &r.kind {
                crate::model::transcript::RowKind::User(u) => {
                    Some(u.text.lines().next().unwrap_or_default().to_owned())
                }
                _ => None,
            })
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "New session".to_owned())
    }

    /// `provider/model` label.
    #[must_use]
    pub fn model_label(&self) -> String {
        self.model.as_ref().map_or_else(
            || "No model".to_owned(),
            |m| {
                if m.name.is_empty() {
                    m.id.clone()
                } else {
                    m.name.clone()
                }
            },
        )
    }

    /// Context window use in percent, when known.
    #[must_use]
    pub fn context_percent(&self) -> Option<u8> {
        let window = self.model.as_ref()?.context_window;
        if window == 0 || self.usage.context_tokens == 0 {
            return None;
        }
        let percent = (self.usage.context_tokens.saturating_mul(100) / window).min(100);
        u8::try_from(percent).ok()
    }

    /// Replaces the transcript with a full message list (history load).
    pub fn load_messages(&mut self, messages: &[AgentMessage]) -> Applied {
        let change = self.transcript.load(messages);
        self.usage = UsageTotals::default();
        for message in messages {
            self.count_usage(message);
        }
        Applied {
            rows: vec![change],
            chrome: true,
            ..Applied::default()
        }
    }

    /// Restores retained UI state (`_oino/ui_snapshot`).
    pub fn load_ui_snapshot(&mut self, requests: &[UiRequest]) -> Applied {
        let mut applied = Applied {
            chrome: true,
            ..Applied::default()
        };
        for request in requests {
            applied.effects.extend(self.ui.apply(request));
        }
        // Replayed toasts are stale.
        applied
            .effects
            .retain(|e| matches!(e, UiEffect::DialogOpened | UiEffect::FocusPanel(_)));
        applied
    }

    fn count_usage(&mut self, message: &AgentMessage) {
        if let AgentMessage::Assistant(assistant) = message {
            let u = &assistant.usage;
            self.usage.input += u.input;
            self.usage.output += u.output;
            self.usage.cost += u.cost.total;
            let context = u.input + u.cache_read + u.cache_write + u.output;
            if context > 0 {
                self.usage.context_tokens = context;
            }
        }
    }

    /// Applies one event.
    #[allow(clippy::too_many_lines, reason = "one arm per event / method kind")]
    pub fn apply(&mut self, event: &ClientEvent) -> Applied {
        let mut applied = Applied::default();
        match event {
            ClientEvent::Session(event) => {
                applied.rows = self.transcript.apply(event);
                match event {
                    AgentSessionEvent::AgentStart => {
                        self.run = RunState::Running;
                        applied.chrome = true;
                    }
                    AgentSessionEvent::AgentSettled { .. } => {
                        self.run = RunState::Idle;
                        applied.chrome = true;
                        applied.settled = true;
                    }
                    AgentSessionEvent::MessageEnd { message } => {
                        self.count_usage(message);
                        applied.chrome = true;
                    }
                    AgentSessionEvent::QueueUpdate {
                        steering,
                        follow_up,
                    } => {
                        self.steering.clone_from(steering);
                        self.follow_up.clone_from(follow_up);
                        applied.chrome = true;
                    }
                    AgentSessionEvent::SessionInfoChanged { name } => {
                        self.name.clone_from(name);
                        applied.chrome = true;
                    }
                    AgentSessionEvent::ThinkingLevelChanged { level } => {
                        self.thinking = *level;
                        applied.chrome = true;
                    }
                    AgentSessionEvent::CompactionStart { .. } => {
                        self.run = RunState::Compacting;
                        applied.chrome = true;
                    }
                    AgentSessionEvent::CompactionEnd { .. } => {
                        if self.run == RunState::Compacting {
                            self.run = RunState::Idle;
                        }
                        applied.chrome = true;
                    }
                    _ => {}
                }
            }
            ClientEvent::Ui(request) => {
                applied.effects.extend(self.ui.apply(request));
                applied.chrome = true;
            }
            ClientEvent::ExtensionError(error) => {
                let e = &error.error;
                let text = format!("{} ({}): {}", e.extension_path, e.event, e.error);
                self.extension_errors.push(text.clone());
                if self.extension_errors.len() > MAX_ERRORS {
                    self.extension_errors.remove(0);
                }
                applied
                    .rows
                    .push(self.transcript.notice(NoticeTone::Error, text));
            }
            ClientEvent::Oino(event) => match event {
                OinoEvent::SessionReplaced {
                    session_file,
                    session_id,
                    ..
                } => {
                    self.id.clone_from(session_id);
                    self.file.clone_from(session_file);
                    self.ui = UiState::new();
                    self.steering.clear();
                    self.follow_up.clear();
                    applied.rows.push(self.transcript.clear());
                    applied.reload = true;
                    applied.chrome = true;
                }
                OinoEvent::ModelChanged { model, .. } => {
                    self.model = Some(model.clone());
                    applied.chrome = true;
                }
                OinoEvent::RunState { state, waiting_on } => {
                    self.run = *state;
                    self.waiting_on = waiting_on.clone().unwrap_or_default();
                    applied.chrome = true;
                }
                OinoEvent::ToolsChanged { active } => {
                    self.active_tools.clone_from(active);
                }
                OinoEvent::ExtensionUiDismiss { id, .. } => {
                    applied.chrome = self.ui.dismiss(id);
                }
                OinoEvent::Lagged { dropped } => {
                    applied.rows.push(self.transcript.notice(
                        NoticeTone::Warning,
                        format!("{dropped} events were dropped; reloading"),
                    ));
                    applied.reload = true;
                }
                OinoEvent::CommandsChanged => {}
            },
        }
        applied
    }

    /// Applies a batch (one frame's worth) of events.
    pub fn apply_all<'a>(&mut self, events: impl IntoIterator<Item = &'a ClientEvent>) -> Applied {
        let mut applied = Applied::default();
        for event in events {
            applied.merge(self.apply(event));
        }
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_core::events::ExtensionErrorEvent;
    use serde_json::json;

    fn event(value: serde_json::Value) -> ClientEvent {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn run_state_queue_and_name() {
        let mut s = SessionModel::new("s1", "/w");
        let applied = s.apply_all(&[
            event(json!({"type":"agent_start"})),
            event(json!({"type":"queue_update","steering":["a"],"followUp":["b"]})),
            event(json!({"type":"session_info_changed","name":"Fix bug"})),
        ]);
        assert!(applied.chrome);
        assert!(s.is_busy());
        assert_eq!(s.steering, vec!["a"]);
        assert_eq!(s.follow_up, vec!["b"]);
        assert_eq!(s.title(), "Fix bug");
        let applied = s.apply(&event(json!({"type":"agent_settled","aborted":false})));
        assert!(applied.settled);
        assert!(!s.is_busy());
    }

    #[test]
    fn dialogs_dismissed_by_core_leave_the_queue() {
        let mut s = SessionModel::new("s1", "/w");
        let applied = s.apply(&event(json!({"type":"extension_ui_request","id":"d1","method":"confirm","title":"t","message":"m"})));
        assert_eq!(applied.effects, vec![UiEffect::DialogOpened]);
        assert!(s.ui.needs_attention());
        s.apply(&event(
            json!({"type":"extension_ui_dismiss","id":"d1","reason":"answered"}),
        ));
        assert!(!s.ui.needs_attention());
    }

    #[test]
    fn session_replaced_requests_reload() {
        let mut s = SessionModel::new("s1", "/w");
        s.apply(&event(
            json!({"type":"message_start","message":{"role":"user","content":"hi","timestamp":0}}),
        ));
        assert_eq!(s.transcript.len(), 1);
        assert_eq!(s.title(), "hi");
        let applied = s.apply(&event(
            json!({"type":"session_replaced","sessionId":"s2","reason":"new"}),
        ));
        assert!(applied.reload);
        assert_eq!(s.id, "s2");
        assert!(s.transcript.is_empty());
        assert_eq!(s.title(), "New session");
    }

    #[test]
    fn usage_and_context_percent() {
        let mut s = SessionModel::new("s1", "/w");
        s.model = Some(
            serde_json::from_value(json!({"id":"m","name":"M","api":"x","provider":"p","baseUrl":"","input":["text"],"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"reasoning":false,"contextWindow":1000,"maxTokens":100}))
                .unwrap(),
        );
        s.apply(&event(json!({"type":"message_end","message":{
            "role":"assistant","content":[],"api":"x","provider":"p","model":"m",
            "usage":{"input":200,"output":50,"cacheRead":0,"cacheWrite":0,"totalTokens":250,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0.5}},
            "stopReason":"stop","timestamp":0}})));
        assert_eq!(s.usage.input, 200);
        assert!((s.usage.cost - 0.5).abs() < f64::EPSILON);
        assert_eq!(s.context_percent(), Some(25));
        assert_eq!(s.model_label(), "M");
    }

    #[test]
    fn extension_errors_become_notices() {
        let mut s = SessionModel::new("s1", "/w");
        let applied = s.apply(&ClientEvent::from(ExtensionErrorEvent {
            extension_path: "unipi.notify".into(),
            event: "agent_end".into(),
            error: "boom".into(),
            stack: None,
        }));
        assert_eq!(applied.rows.len(), 1);
        assert_eq!(s.extension_errors.len(), 1);
    }
}
