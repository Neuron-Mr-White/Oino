//! One open session: event pump, conversation list, composer, dialogs, panels and the side
//! pane (gpui-app §3.1, §5.2–§5.4).
//!
//! Speed: events are drained from the backend in batches, coalesced, reduced once and turned
//! into minimal `ListState` updates (`splice` for inserts, `remeasure_items` for changed rows);
//! the pump waits one frame (8 ms) between batches, so a token burst costs one redraw. Rows are
//! virtualised by GPUI's `list`; each assistant row owns an incremental markdown state.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FollowMode, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ListAlignment, ListState, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, list, px,
};
use oino_ai::{AgentMessage, JsonValue, Model, ThinkingLevel};
use oino_client::{ClientError, OinoCommand, PiCommand, RpcCommand, SessionBackend};
use oino_core::events::{ClientEvent, RunState};
use oino_core::ui::{UiRequest, UiResponse};

use crate::host::{AppSettings, OpenedSession};
use crate::model::coalesce::coalesce;
use crate::model::composer::{self, Drafts, Intent, SubmitKey};
use crate::model::session::{Applied, SessionModel};
use crate::model::sessions::LiveStatus;
use crate::model::tools::{self, INLINE_DIFF_LINES, TAIL_LINES, ToolKind};
use crate::model::transcript::{NoticeTone, Row, RowChange, RowKind, SummaryKind, ToolStatus};
use crate::model::ui_state::{
    Align, PanelPlacement, Segment, UiEffect, WidgetPlacement, fit_segments,
};
use crate::rt::Tokio;
use crate::views::dialog::{DialogEvent, DialogView};
use crate::views::ous::{self, ActionFn};
use crate::views::theme::{self, tone_color};

/// Frame pacing between event batches.
const FRAME: Duration = Duration::from_millis(8);

/// Events for the workspace.
#[derive(Clone, Debug)]
pub enum PaneEvent {
    /// Perform an effect (toast, OS notification, URL, clipboard, theme, settings).
    Effect(UiEffect),
    /// Sidebar status changed.
    Live(LiveStatus),
    /// Title / id / model changed (sidebar + header refresh).
    Changed,
    /// The palette should open in models mode.
    PickModel,
}

/// What the side pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideTab {
    /// A full diff.
    Diff {
        /// Title (path).
        title: String,
        /// Unified patch.
        patch: String,
    },
    /// Tool / bash output.
    Output {
        /// Title.
        title: String,
        /// Text.
        text: String,
    },
    /// An extension panel (`sidePane` placement).
    Panel(String),
}

/// One open session.
pub struct SessionPane {
    /// Reduced session state.
    pub model: SessionModel,
    backend: Arc<dyn SessionBackend>,
    list: ListState,
    markdown: HashMap<String, (Entity<TextViewState>, usize)>,
    thinking_open: HashSet<String>,
    commands: Vec<(String, Option<String>)>,
    /// Models with credentials (`get_available_models`).
    pub models: Vec<Model>,
    composer: Entity<TextareaState>,
    drafts: Drafts,
    pending_text: Option<String>,
    dialog: Option<Entity<DialogView>>,
    /// Side pane content.
    pub side: Option<SideTab>,
    settings: AppSettings,
    focus: FocusHandle,
    _pump: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaneEvent> for SessionPane {}

impl Focusable for SessionPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.composer.focus_handle(cx)
    }
}

fn response_field<T: serde::de::DeserializeOwned>(data: Option<JsonValue>, key: &str) -> Option<T> {
    let value = data?;
    let field = if key.is_empty() {
        value
    } else {
        value.get(key)?.clone()
    };
    serde_json::from_value(field).ok()
}

/// Results of the initial (or post-replace) load.
struct Loaded {
    state: Option<JsonValue>,
    messages: Option<Vec<AgentMessage>>,
    snapshot: Option<Vec<UiRequest>>,
    commands: Vec<(String, Option<String>)>,
    models: Vec<Model>,
}

impl SessionPane {
    /// A pane for `opened`; starts the event pump and loads history.
    pub fn new(
        opened: &OpenedSession,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let backend = Arc::clone(&opened.backend);
        let mut model = SessionModel::new(backend.session_id(), opened.cwd.clone());
        model.file = opened.file.as_ref().map(|p| p.display().to_string());
        let list = ListState::new(0, ListAlignment::Top, px(1200.));
        list.set_follow_mode(FollowMode::Tail);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .submit_on_enter(true)
                .placeholder("Ask Oino…   / commands · ! shell · Shift+Enter newline")
        });
        let subscriptions = vec![cx.subscribe_in(&composer, window, Self::on_composer)];
        let pump = Self::pump(&backend, cx);
        let mut pane = Self {
            model,
            backend,
            list,
            markdown: HashMap::new(),
            thinking_open: HashSet::new(),
            commands: Vec::new(),
            models: Vec::new(),
            composer,
            drafts: Drafts::default(),
            pending_text: None,
            dialog: None,
            side: None,
            settings,
            focus: cx.focus_handle(),
            _pump: pump,
            _subscriptions: subscriptions,
        };
        pane.say_hello(cx);
        pane.reload(cx);
        pane
    }

    /// The backend.
    #[must_use]
    pub fn backend(&self) -> &Arc<dyn SessionBackend> {
        &self.backend
    }

    /// Slash commands of this session.
    #[must_use]
    pub fn commands(&self) -> &[(String, Option<String>)] {
        &self.commands
    }

    /// Updates desktop preferences (thinking blocks, density).
    pub fn set_settings(&mut self, settings: AppSettings, cx: &mut Context<Self>) {
        self.settings = settings;
        self.list.remeasure();
        cx.notify();
    }

    /// Live status for the sidebar.
    #[must_use]
    pub fn live(&self) -> LiveStatus {
        if self.model.ui.needs_attention() {
            LiveStatus::Attention
        } else if self.model.is_busy() {
            LiveStatus::Running
        } else {
            LiveStatus::Idle
        }
    }

    fn pump(backend: &Arc<dyn SessionBackend>, cx: &mut Context<Self>) -> Task<()> {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<ClientEvent>();
        let mut stream = backend.events();
        let forward = Tokio::spawn(cx, async move {
            while let Some(event) = stream.next().await {
                if tx.unbounded_send(event).is_err() {
                    break;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let _forward = forward;
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                }
                let batch = coalesce(batch);
                if this
                    .update(cx, |pane, cx| pane.apply_batch(&batch, cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(FRAME).await;
            }
        })
    }

    fn apply_batch(&mut self, batch: &[ClientEvent], cx: &mut Context<Self>) {
        let before = (self.live(), self.model.id.clone(), self.model.title());
        let applied = self.model.apply_all(batch);
        self.absorb(applied, cx);
        let after = (self.live(), self.model.id.clone(), self.model.title());
        if before.0 != after.0 {
            cx.emit(PaneEvent::Live(after.0));
        }
        if before.1 != after.1 || before.2 != after.2 {
            cx.emit(PaneEvent::Changed);
        }
    }

    fn absorb(&mut self, applied: Applied, cx: &mut Context<Self>) {
        for change in &applied.rows {
            match *change {
                RowChange::Inserted { at, count } => self.list.splice(at..at, count),
                RowChange::Updated { at } => self.list.remeasure_items(at..at + 1),
                RowChange::Reset => {
                    self.markdown.clear();
                    self.list.reset(self.model.transcript.len());
                }
            }
        }
        self.sync_markdown(&applied.rows, cx);
        for effect in applied.effects {
            match effect {
                UiEffect::DialogOpened => {}
                UiEffect::SetEditorText(text) => self.set_composer_text(&text, cx),
                UiEffect::InsertEditorText(text) => {
                    let current = self.composer.read(cx).value().to_string();
                    self.set_composer_text(&format!("{current}{text}"), cx);
                }
                UiEffect::InsertToken { label, .. } => {
                    let current = self.composer.read(cx).value().to_string();
                    self.set_composer_text(&format!("{current}[{label}] "), cx);
                }
                UiEffect::FocusPanel(id) => {
                    if self
                        .model
                        .ui
                        .panel(&id)
                        .is_some_and(|p| p.placement == PanelPlacement::SidePane)
                    {
                        self.side = Some(SideTab::Panel(id));
                    }
                }
                other => cx.emit(PaneEvent::Effect(other)),
            }
        }
        if applied.reload {
            self.reload(cx);
        }
        if applied.chrome || !applied.rows.is_empty() {
            cx.notify();
        }
    }

    /// Keeps one incremental markdown state per assistant row in sync.
    fn sync_markdown(&mut self, changes: &[RowChange], cx: &mut Context<Self>) {
        let mut touched: Vec<usize> = Vec::new();
        for change in changes {
            match *change {
                RowChange::Inserted { at, count } => touched.extend(at..at + count),
                RowChange::Updated { at } => touched.push(at),
                RowChange::Reset => {}
            }
        }
        touched.dedup();
        for at in touched {
            let Some(Row {
                id,
                kind: RowKind::Assistant(row),
                ..
            }) = self.model.transcript.rows().get(at)
            else {
                continue;
            };
            let (id, text) = (id.clone(), row.text.clone());
            if let Some((state, len)) = self.markdown.get_mut(&id) {
                let appended = text.len() >= *len && text.is_char_boundary(*len);
                let delta = if appended {
                    text[*len..].to_owned()
                } else {
                    String::new()
                };
                *len = text.len();
                state.update(cx, |s, cx| {
                    if appended {
                        if !delta.is_empty() {
                            s.push_str(&delta, cx);
                        }
                    } else {
                        s.set_text(&text, cx);
                    }
                });
            } else {
                let len = text.len();
                let state = cx.new(|cx| TextViewState::markdown(&text, cx).selectable(true));
                self.markdown.insert(id, (state, len));
            }
        }
    }

    fn send(&self, command: RpcCommand, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move { backend.send(command).await });
        cx.spawn(async move |this, cx| {
            let error = match task.await {
                Ok(Ok(_)) => return,
                Ok(Err(e)) => e,
                Err(e) => ClientError::Transport(e.to_string()),
            };
            let _ = this.update(cx, |pane, cx| {
                let change = pane
                    .model
                    .transcript
                    .notice(NoticeTone::Error, error.to_string());
                pane.absorb(
                    Applied {
                        rows: vec![change],
                        ..Applied::default()
                    },
                    cx,
                );
            });
        })
        .detach();
    }

    fn say_hello(&self, cx: &mut Context<Self>) {
        let caps = serde_json::json!({
            "dialogs": ["select","confirm","input","editor","multiSelect","form","approval","custom"],
            "ous": true,
            "panels": ["overlay","sidePane","editorReplace","bottomPane","fullscreen"],
            "images": false, "osNotify": true, "clipboard": true
        });
        self.send(
            OinoCommand::Hello {
                client: oino_client::protocol::ClientInfo {
                    name: "oino-desktop".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
                ui: Some(caps),
                protocol: 1,
            }
            .into(),
            cx,
        );
    }

    /// Reloads state, messages, UI snapshot, commands and models.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move {
            let send = |c: RpcCommand| {
                let backend = Arc::clone(&backend);
                async move { backend.send(c).await.ok().flatten() }
            };
            let commands: Vec<JsonValue> =
                response_field(send(PiCommand::GetCommands.into()).await, "commands")
                    .unwrap_or_default();
            Loaded {
                state: send(PiCommand::GetState.into()).await,
                messages: response_field(send(PiCommand::GetMessages.into()).await, "messages"),
                snapshot: response_field(send(OinoCommand::UiSnapshot.into()).await, "requests"),
                commands: commands
                    .iter()
                    .filter_map(|c| {
                        Some((
                            c.get("name")?.as_str()?.to_owned(),
                            c.get("description")
                                .and_then(JsonValue::as_str)
                                .map(str::to_owned),
                        ))
                    })
                    .collect(),
                models: response_field(send(PiCommand::GetAvailableModels.into()).await, "models")
                    .unwrap_or_default(),
            }
        });
        cx.spawn(async move |this, cx| {
            if let Ok(loaded) = task.await {
                let _ = this.update(cx, |pane, cx| pane.loaded(loaded, cx));
            }
        })
        .detach();
    }

    fn loaded(&mut self, loaded: Loaded, cx: &mut Context<Self>) {
        if let Some(state) = &loaded.state {
            if let Some(model) = state
                .get("model")
                .and_then(|m| serde_json::from_value(m.clone()).ok())
            {
                self.model.model = Some(model);
            }
            if let Some(level) = state
                .get("thinkingLevel")
                .and_then(|l| serde_json::from_value::<ThinkingLevel>(l.clone()).ok())
            {
                self.model.thinking = level;
            }
            self.model.name = state
                .get("sessionName")
                .and_then(JsonValue::as_str)
                .map(str::to_owned);
            if state.get("isStreaming").and_then(JsonValue::as_bool) == Some(true) {
                self.model.run = RunState::Running;
            }
        }
        let mut applied = Applied {
            chrome: true,
            ..Applied::default()
        };
        if let Some(messages) = &loaded.messages {
            applied.merge(self.model.load_messages(messages));
        }
        if let Some(snapshot) = &loaded.snapshot {
            applied.merge(self.model.load_ui_snapshot(snapshot));
        }
        self.commands = loaded.commands;
        self.models = loaded.models;
        self.absorb(applied, cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Live(self.live()));
        self.list.scroll_to_end();
    }

    // ── composer ─────────────────────────────────────────────────────────────

    /// Replaces the composer text on the next render (setting it needs the window).
    fn set_composer_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.pending_text = Some(text.to_owned());
        cx.notify();
    }

    fn on_composer(
        &mut self,
        _: &Entity<TextareaState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter {
                shift: false,
                secondary,
            } => {
                let key = if *secondary {
                    SubmitKey::FollowUp
                } else {
                    SubmitKey::Enter
                };
                self.submit(key, cx);
            }
            InputEvent::Change => cx.notify(),
            _ => {}
        }
    }

    fn submit(&mut self, key: SubmitKey, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        let intent = composer::intent(&text, self.model.is_busy(), key);
        if intent == Intent::Nothing {
            return;
        }
        self.drafts.history(&self.model.id.clone()).push(&text);
        if let Some(command) = intent.command(Vec::new()) {
            self.send(command, cx);
        }
        self.set_composer_text("", cx);
    }

    /// Aborts the current run.
    pub fn abort(&mut self, cx: &mut Context<Self>) {
        self.send(PiCommand::Abort.into(), cx);
    }

    /// Selects a model.
    pub fn set_model(&mut self, provider: String, model_id: String, cx: &mut Context<Self>) {
        self.send(PiCommand::SetModel { provider, model_id }.into(), cx);
    }

    /// Sets the thinking level.
    pub fn set_thinking(&mut self, level: ThinkingLevel, cx: &mut Context<Self>) {
        self.model.thinking = level;
        self.send(PiCommand::SetThinkingLevel { level }.into(), cx);
        cx.notify();
    }

    /// Sends a slash command (or any text) as a prompt.
    pub fn run_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        let intent = composer::intent(text, self.model.is_busy(), SubmitKey::Enter);
        if let Some(command) = intent.command(Vec::new()) {
            self.send(command, cx);
        }
    }

    /// Starts a new session in this pane (runtime replace).
    pub fn new_session(&mut self, cx: &mut Context<Self>) {
        self.send(
            PiCommand::NewSession {
                parent_session: None,
            }
            .into(),
            cx,
        );
    }

    /// Compacts the context.
    pub fn compact(&mut self, cx: &mut Context<Self>) {
        self.send(
            PiCommand::Compact {
                custom_instructions: None,
            }
            .into(),
            cx,
        );
    }

    /// Focuses the composer of `pane`.
    pub fn focus(pane: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let composer = pane.read(cx).composer.clone();
        composer.update(cx, |s, cx| s.focus(window, cx));
    }

    fn on_composer_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.composer.read(cx).value().to_string();
        let session = self.model.id.clone();
        match event.keystroke.key.as_str() {
            "up" if text.is_empty() || self.drafts.history(&session).is_browsing() => {
                if let Some(prev) = self.drafts.history(&session).older().map(str::to_owned) {
                    self.composer
                        .update(cx, |s, cx| s.set_value(prev, window, cx));
                    cx.stop_propagation();
                }
            }
            "down" if self.drafts.history(&session).is_browsing() => {
                if let Some(next) = self.drafts.history(&session).newer().map(str::to_owned) {
                    self.composer
                        .update(cx, |s, cx| s.set_value(next, window, cx));
                    cx.stop_propagation();
                }
            }
            "tab" => {
                if let Some(first) = composer::slash_completions(&self.commands, &text, 1)
                    .into_iter()
                    .next()
                {
                    self.composer
                        .update(cx, |s, cx| s.set_value(first.value, window, cx));
                    cx.stop_propagation();
                }
            }
            "escape" if self.model.is_busy() => {
                self.abort(cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    // ── dialogs & panels ─────────────────────────────────────────────────────

    fn respond(&mut self, response: UiResponse, cx: &mut Context<Self>) {
        self.model.ui.dismiss(&response.id);
        self.dialog = None;
        self.backend.ui_respond(response);
        cx.emit(PaneEvent::Live(self.live()));
        cx.notify();
    }

    fn sync_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let front = self.model.ui.dialog().cloned();
        let current = self.dialog.as_ref().map(|d| d.read(cx).id().to_owned());
        match front {
            Some(dialog) if current.as_deref() != Some(dialog.id.as_str()) => {
                let view = cx.new(|cx| DialogView::new(dialog, window, cx));
                cx.subscribe(&view, |this, _, event: &DialogEvent, cx| match event {
                    DialogEvent::Respond(response) => this.respond(response.clone(), cx),
                })
                .detach();
                self.dialog = Some(view);
            }
            None if self.dialog.is_some() => {
                self.dialog = None;
                let composer = self.composer.clone();
                composer.update(cx, |s, cx| s.focus(window, cx));
            }
            _ => {}
        }
    }

    /// Interactive OUS nodes inside panels. `extension_ui_action` is not part of the
    /// `oino-client` command set yet (requested from modes), so actions are logged only.
    #[allow(
        clippy::unused_self,
        reason = "view helper kept as a method for uniform call sites"
    )]
    fn panel_action(&self, panel: &str) -> ActionFn {
        let panel = panel.to_owned();
        std::rc::Rc::new(move |action, value, _, _| {
            tracing::info!(panel = %panel, action, %value, "extension_ui_action (not wired: no client command)");
        })
    }

    fn close_panel(&mut self, id: &str, cx: &mut Context<Self>) {
        self.model.ui.close_panel(id);
        if self.side == Some(SideTab::Panel(id.to_owned())) {
            self.side = None;
        }
        tracing::info!(panel = %id, "extension_ui_action $close (not wired: no client command)");
        cx.notify();
    }

    // ── rendering ────────────────────────────────────────────────────────────

    fn toggle_row(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(change) = self.model.transcript.toggle_expanded(id) {
            self.absorb(
                Applied {
                    rows: vec![change],
                    ..Applied::default()
                },
                cx,
            );
        }
    }

    fn render_row(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.model.transcript.rows().get(ix).cloned() else {
            return div().into_any_element();
        };
        let pad = if self.settings.compact {
            px(3.)
        } else {
            px(6.)
        };
        let inner = match &row.kind {
            RowKind::User(user) => self.render_user(&row.id, user, cx),
            RowKind::Assistant(_) => self.render_assistant(&row, cx),
            RowKind::Tool(tool) => self.render_tool(&row.id, tool, cx),
            RowKind::Bash(bash) => self.render_bash(&row.id, bash, cx),
            RowKind::Custom(custom) => {
                let body = ous::render(
                    &crate::model::ous::OusNode::Markdown {
                        value: custom.text.clone(),
                    },
                    &row.id,
                    &ous::no_action(),
                    cx,
                );
                card(
                    custom
                        .provenance()
                        .map(str::to_owned)
                        .or_else(|| Some(custom.custom_type.clone())),
                    custom.provenance().is_some(),
                    body,
                    cx,
                )
            }
            RowKind::Summary(summary) => {
                let id = row.id.clone();
                let title = match summary.kind {
                    SummaryKind::Compaction => format!(
                        "Compaction summary · {} tokens before",
                        summary.tokens_before
                    ),
                    SummaryKind::Branch => "Branch summary".to_owned(),
                };
                v_flex()
                    .id(SharedString::from(format!("{id}-sum")))
                    .w_full()
                    .p_2()
                    .gap_1()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_row(&id, cx)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} {title}",
                                if summary.expanded { "▾" } else { "▸" }
                            )),
                    )
                    .when(summary.expanded, |el| {
                        el.child(TextView::markdown(
                            SharedString::from(format!("{}-md", row.id)),
                            summary.summary.clone(),
                        ))
                    })
                    .into_any_element()
            }
            RowKind::Notice(tone, text) => {
                let color = match tone {
                    NoticeTone::Info => cx.theme().muted_foreground,
                    NoticeTone::Warning => cx.theme().warning,
                    NoticeTone::Error => cx.theme().danger,
                };
                div()
                    .text_xs()
                    .text_color(color)
                    .child(SharedString::from(text.clone()))
                    .into_any_element()
            }
        };
        div()
            .w_full()
            .px_6()
            .py(pad)
            .child(div().max_w(px(920.)).mx_auto().child(inner))
            .into_any_element()
    }

    #[allow(
        clippy::unused_self,
        reason = "view helper kept as a method for uniform call sites"
    )]
    fn render_user(
        &self,
        _id: &str,
        user: &crate::model::transcript::UserRow,
        cx: &App,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .justify_end()
            .child(
                v_flex()
                    .max_w(px(720.))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(cx.theme().secondary)
                    .text_sm()
                    .child(SharedString::from(user.text.clone()))
                    .when(user.images > 0, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} image(s) attached", user.images)),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_assistant(&mut self, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let RowKind::Assistant(a) = &row.kind else {
            return div().into_any_element();
        };
        let id = row.id.clone();
        let muted = cx.theme().muted_foreground;
        let show_thinking = !self.settings.hide_thinking || self.thinking_open.contains(&id);
        let thinking = (!a.thinking.is_empty()).then(|| {
            let toggle_id = id.clone();
            v_flex()
                .id(SharedString::from(format!("{id}-think")))
                .gap_1()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !this.thinking_open.remove(&toggle_id) {
                        this.thinking_open.insert(toggle_id.clone());
                    }
                    if let Some(ix) = this.model.transcript.position(&toggle_id) {
                        this.list.remeasure_items(ix..ix + 1);
                    }
                    cx.notify();
                }))
                .child(div().text_xs().text_color(muted).child(if show_thinking {
                    "▾ Thinking"
                } else if a.streaming && a.text.is_empty() {
                    "▸ Thinking…"
                } else {
                    "▸ Thinking"
                }))
                .when(show_thinking, |el| {
                    el.child(
                        div()
                            .pl_3()
                            .border_l_2()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(muted)
                            .italic()
                            .child(SharedString::from(a.thinking.clone())),
                    )
                })
        });
        let body = self
            .markdown
            .get(&id)
            .map(|(state, _)| TextView::new(state).selectable(true).into_any_element());
        v_flex()
            .w_full()
            .gap_1()
            .text_sm()
            .children(thinking)
            .children(body)
            .when(
                a.streaming && a.text.is_empty() && a.thinking.is_empty(),
                |el| {
                    el.child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(muted)
                            .child(Spinner::new().xsmall())
                            .child("Working…"),
                    )
                },
            )
            .when_some(a.error.clone(), |el, e| {
                el.child(
                    div()
                        .text_xs()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(cx.theme().danger.opacity(0.12))
                        .text_color(cx.theme().danger)
                        .child(SharedString::from(e)),
                )
            })
            .into_any_element()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "declarative element tree; splitting scatters the layout"
    )]
    fn render_tool(
        &mut self,
        id: &str,
        tool: &crate::model::transcript::ToolRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let kind = ToolKind::of(&tool.name);
        let icon: AnyElement = match tool.status {
            ToolStatus::Preparing | ToolStatus::Running => {
                Spinner::new().xsmall().into_any_element()
            }
            ToolStatus::Done => div()
                .text_color(theme.success)
                .child("✓")
                .into_any_element(),
            ToolStatus::Error => div().text_color(theme.danger).child("✗").into_any_element(),
        };
        let patch = tool.patch();
        let preview = match (&patch, tool.status) {
            (Some(p), _) => {
                let d = tools::diff_preview(p, 0);
                Some(format!("+{} −{}", d.added, d.removed))
            }
            (None, ToolStatus::Preparing) => tools::args_preview(&tool.name, &tool.args),
            _ => None,
        };
        let duration = tool.duration_ms.filter(|ms| *ms >= 100).map(|ms| {
            if ms >= 1000 {
                format!("{}.{}s", ms / 1000, (ms % 1000) / 100)
            } else {
                format!("{ms}ms")
            }
        });
        let toggle = id.to_owned();
        let header = h_flex()
            .id(SharedString::from(format!("{id}-hdr")))
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(theme.list_hover))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_row(&toggle, cx)))
            .child(div().w(px(14.)).flex().justify_center().child(icon))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .when(kind == ToolKind::Shell, |el| {
                        el.font_family(theme.mono_font_family.clone())
                    })
                    .child(SharedString::from(tool.title())),
            )
            .when_some(preview, |el, p| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(p)),
                )
            })
            .when_some(duration, |el, d| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(SharedString::from(d)),
                )
            })
            .child(div().text_xs().text_color(muted).child(if tool.expanded {
                "▾"
            } else {
                "▸"
            }));
        let expanded = tool.expanded || self.settings.expand_tools;
        let mut body: Option<AnyElement> = None;
        if let Some(p) = patch.clone() {
            let title = tool.title();
            let open = p.clone();
            body = Some(
                v_flex()
                    .w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .overflow_hidden()
                    .child(ous::diff_lines(&p, INLINE_DIFF_LINES, cx))
                    .child(
                        h_flex().justify_end().p_1().child(
                            Button::new(SharedString::from(format!("{id}-opendiff")))
                                .ghost()
                                .xsmall()
                                .label("Open diff")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.side = Some(SideTab::Diff {
                                        title: title.clone(),
                                        patch: open.clone(),
                                    });
                                    cx.notify();
                                })),
                        ),
                    )
                    .into_any_element(),
            );
        } else if (kind == ToolKind::Shell && (tool.status == ToolStatus::Running || expanded))
            || (expanded && !tool.output.is_empty())
        {
            let lines = if expanded {
                tools::tail(&tool.output, 30)
            } else {
                tools::tail(&tool.output, TAIL_LINES)
            };
            let title = tool.title();
            let text = tool.output.clone();
            body = Some(
                v_flex()
                    .w_full()
                    .p_2()
                    .rounded_md()
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(if tool.status == ToolStatus::Error {
                        theme.danger
                    } else {
                        theme.foreground
                    })
                    .children(
                        lines
                            .into_iter()
                            .map(|l| div().whitespace_nowrap().child(SharedString::from(l))),
                    )
                    .child(
                        h_flex().justify_end().child(
                            Button::new(SharedString::from(format!("{id}-openout")))
                                .ghost()
                                .xsmall()
                                .label("Open output")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.side = Some(SideTab::Output {
                                        title: title.clone(),
                                        text: text.clone(),
                                    });
                                    cx.notify();
                                })),
                        ),
                    )
                    .into_any_element(),
            );
        }
        v_flex()
            .w_full()
            .gap_1()
            .when(tool.parent.is_some(), gpui_kit::Styled::pl_6)
            .child(header)
            .children(body)
            .into_any_element()
    }

    #[allow(
        clippy::unused_self,
        reason = "view helper kept as a method for uniform call sites"
    )]
    fn render_bash(
        &self,
        _id: &str,
        bash: &crate::model::transcript::BashRow,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .child(format!("$ {}", bash.command)),
            )
            .child(
                v_flex()
                    .p_2()
                    .rounded_md()
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .children(
                        tools::tail(&bash.output, 12)
                            .into_iter()
                            .map(|l| div().child(SharedString::from(l))),
                    )
                    .when_some(bash.exit_code.filter(|c| *c != 0), |el, c| {
                        el.child(div().text_color(theme.danger).child(format!("exit {c}")))
                    }),
            )
            .into_any_element()
    }

    fn render_segments(segments: &[Segment], cx: &App) -> AnyElement {
        let kept = fit_segments(segments, 140, 3);
        let (left, right): (Vec<&Segment>, Vec<&Segment>) =
            kept.into_iter().partition(|s| s.align == Align::Left);
        let seg = |s: &Segment| {
            div()
                .text_xs()
                .text_color(tone_color(s.tone, cx))
                .child(SharedString::from(s.text.clone()))
        };
        h_flex()
            .w_full()
            .px_1()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .flex_1()
                    .children(left.into_iter().map(seg)),
            )
            .child(h_flex().gap_3().children(right.into_iter().map(seg)))
            .into_any_element()
    }

    fn render_widgets(&self, placement: WidgetPlacement, cx: &App) -> Vec<AnyElement> {
        self.model
            .ui
            .widgets(placement)
            .enumerate()
            .map(|(i, w)| {
                let body = match &w.spec {
                    Some(spec) => ous::render(spec, &format!("w{i}"), &ous::no_action(), cx),
                    None => v_flex()
                        .text_xs()
                        .children(
                            w.lines
                                .iter()
                                .map(|l| div().child(SharedString::from(l.clone()))),
                        )
                        .into_any_element(),
                };
                div()
                    .w_full()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(body)
                    .into_any_element()
            })
            .collect()
    }

    fn render_queue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.model.steering.is_empty() && self.model.follow_up.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let items = self
            .model
            .steering
            .iter()
            .map(|t| ("steer", t))
            .chain(self.model.follow_up.iter().map(|t| ("follow-up", t)));
        Some(
            v_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(muted)
                        .child("Queued")
                        .child(
                            Button::new("queue-clear")
                                .ghost()
                                .xsmall()
                                .label("Clear")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send(PiCommand::ClearQueue.into(), cx);
                                })),
                        ),
                )
                .children(items.map(|(kind, text)| {
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(div().text_color(muted).w(px(64.)).child(kind))
                        .child(div().truncate().child(SharedString::from(text.clone())))
                }))
                .into_any_element(),
        )
    }

    fn render_completions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let text = self.composer.read(cx).value().to_string();
        let items = composer::slash_completions(&self.commands, &text, 8);
        if items.is_empty() {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        Some(
            v_flex()
                .w_full()
                .p_1()
                .rounded_md()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .children(items.into_iter().enumerate().map(|(i, c)| {
                    let value = c.value.clone();
                    h_flex()
                        .id(SharedString::from(format!("cmp{i}")))
                        .gap_3()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().list_hover))
                        .when(i == 0, |el| el.bg(cx.theme().list_active))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_composer_text(&value, cx);
                        }))
                        .child(
                            div()
                                .text_sm()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(SharedString::from(c.label)),
                        )
                        .when_some(c.description, |el, d| {
                            el.child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(muted)
                                    .child(SharedString::from(d)),
                            )
                        })
                }))
                .into_any_element(),
        )
    }

    fn render_model_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let thinking = self.model.thinking;
        h_flex()
            .gap_1()
            .child(
                Button::new("model-pick")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Bot)
                    .label(SharedString::from(self.model.model_label()))
                    .tooltip("Select model (Ctrl+L)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::PickModel))),
            )
            .when(
                self.model.model.as_ref().is_some_and(|m| m.reasoning),
                |el| el.child(thinking_slider(thinking, cx)),
            )
            .into_any_element()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "declarative element tree; splitting scatters the layout"
    )]
    fn render_composer(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let editor_panel = self
            .model
            .ui
            .panels()
            .find(|p| {
                matches!(
                    p.placement,
                    PanelPlacement::EditorReplace | PanelPlacement::BottomPane
                )
            })
            .cloned();
        let frame = self.model.ui.frame();
        let busy = self.model.is_busy();
        let theme = cx.theme().clone();
        let core: AnyElement = if let Some(panel) = editor_panel {
            let id = panel.id.clone();
            v_flex()
                .w_full()
                .p_2()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(SharedString::from(panel.title.clone())),
                        )
                        .child(
                            Button::new("editor-panel-close")
                                .ghost()
                                .xsmall()
                                .label("Close (Esc)")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.close_panel(&id, cx);
                                })),
                        ),
                )
                .child(ous::render(
                    &panel.spec,
                    &panel.id,
                    &self.panel_action(&panel.id),
                    cx,
                ))
                .into_any_element()
        } else {
            let _ = window;
            v_flex()
                .w_full()
                .rounded_lg()
                .border_1()
                .border_color(if busy {
                    theme.primary.opacity(0.6)
                } else {
                    theme.border
                })
                .bg(theme.background)
                .child(
                    div()
                        .id("composer")
                        .capture_key_down(cx.listener(Self::on_composer_key))
                        .child(Textarea::new(&self.composer).appearance(false).p_2()),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_1()
                        .pb_1()
                        .justify_between()
                        .child(self.render_model_control(cx))
                        .child(
                            h_flex()
                                .gap_1()
                                .when(busy, |el| {
                                    el.child(
                                        Button::new("abort")
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::Square)
                                            .label("Stop")
                                            .tooltip("Abort (Ctrl+. / Esc)")
                                            .on_click(cx.listener(|this, _, _, cx| this.abort(cx))),
                                    )
                                })
                                .child(
                                    Button::new("send")
                                        .primary()
                                        .xsmall()
                                        .icon(IconName::ArrowUp)
                                        .tooltip(if busy {
                                            "Steer (Enter) · Follow-up (Ctrl+Enter)"
                                        } else {
                                            "Send (Enter)"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.submit(SubmitKey::Enter, cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element()
        };
        v_flex()
            .w_full()
            .max_w(px(920.))
            .mx_auto()
            .gap_1()
            .children(self.render_widgets(WidgetPlacement::AboveEditor, cx))
            .children(self.render_queue(cx))
            .children(self.render_completions(cx))
            .when(!frame.top.is_empty(), |el| {
                el.child(Self::render_segments(&frame.top, cx))
            })
            .child(core)
            .when(!frame.bottom.is_empty(), |el| {
                el.child(Self::render_segments(&frame.bottom, cx))
            })
            .children(self.render_widgets(WidgetPlacement::BelowEditor, cx))
            .into_any_element()
    }

    /// Side pane body (diff, output or a `sidePane` panel).
    pub fn render_side(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let side = self.side.clone()?;
        let theme = cx.theme().clone();
        let (title, body): (String, AnyElement) = match side {
            SideTab::Diff { title, patch } => (title, ous::diff_lines(&patch, 5000, cx)),
            SideTab::Output { title, text } => (
                title,
                v_flex()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .children(text.lines().map(|l| {
                        div()
                            .whitespace_nowrap()
                            .child(SharedString::from(l.to_owned()))
                    }))
                    .into_any_element(),
            ),
            SideTab::Panel(id) => {
                let panel = self.model.ui.panel(&id)?;
                (
                    panel.title.clone(),
                    ous::render(&panel.spec, &id, &self.panel_action(&id), cx),
                )
            }
        };
        Some(
            v_flex()
                .size_full()
                .child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .justify_between()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(SharedString::from(title)),
                        )
                        .child(
                            Button::new("side-close")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.side = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("side-body")
                        .flex_1()
                        .overflow_y_scroll()
                        .p_2()
                        .child(body),
                )
                .into_any_element(),
        )
    }

    /// Side-pane tabs available (panels with `sidePane` placement).
    #[must_use]
    pub fn side_panels(&self) -> Vec<(String, String)> {
        self.model
            .ui
            .panels()
            .filter(|p| p.placement == PanelPlacement::SidePane)
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect()
    }

    fn render_overlay_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let panel = self
            .model
            .ui
            .panels()
            .find(|p| {
                matches!(
                    p.placement,
                    PanelPlacement::Overlay | PanelPlacement::Fullscreen
                )
            })?
            .clone();
        let id = panel.id.clone();
        let close_id = id.clone();
        let fullscreen = panel.placement == PanelPlacement::Fullscreen;
        let theme = cx.theme().clone();
        let keys = panel.keys.clone();
        let action = self.panel_action(&id);
        Some(
            div()
                .id("panel-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::scrim())
                .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                    if e.keystroke.key == "escape" {
                        this.close_panel(&close_id, cx);
                    }
                }))
                .child(
                    v_flex()
                        .id("panel")
                        .when(fullscreen, gpui_kit::Styled::size_full)
                        .when(!fullscreen, |el| {
                            el.w(px(760.))
                                .max_w(gpui_kit::relative(0.96))
                                .max_h(px(640.))
                        })
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.popover)
                        .shadow_lg()
                        .child(
                            h_flex()
                                .px_3()
                                .py_2()
                                .gap_2()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(
                                    div()
                                        .flex_1()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(SharedString::from(panel.title.clone())),
                                )
                                .children(keys.into_iter().map(|k| {
                                    let a = std::rc::Rc::clone(&action);
                                    let action_id = k.action_id.clone();
                                    Button::new(SharedString::from(format!("pk-{}", k.action_id)))
                                        .ghost()
                                        .xsmall()
                                        .label(SharedString::from(format!(
                                            "{} ({})",
                                            k.label, k.key
                                        )))
                                        .on_click(move |_, window, cx| {
                                            a(&action_id, JsonValue::Null, window, cx);
                                        })
                                }))
                                .child(
                                    Button::new("panel-close")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.close_panel(&id, cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .id("panel-body")
                                .flex_1()
                                .overflow_y_scroll()
                                .p_3()
                                .child(ous::render(
                                    &panel.spec,
                                    &panel.id,
                                    &self.panel_action(&panel.id),
                                    cx,
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let prompts = self.model.transcript.prompt_indices();
        v_flex()
            .w(px(12.))
            .h_full()
            .py_4()
            .gap_1()
            .items_center()
            .children(prompts.into_iter().take(80).map(|ix| {
                div()
                    .id(SharedString::from(format!("rail{ix}")))
                    .w(px(6.))
                    .h(px(6.))
                    .rounded_full()
                    .bg(cx.theme().muted_foreground.opacity(0.45))
                    .hover(|s| s.bg(cx.theme().primary))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.list.scroll_to_reveal_item(ix);
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    fn render_empty(&self, cx: &App) -> AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("What should Oino do?"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(self.model.cwd.clone())),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        "Type / for commands · ! for shell · Ctrl+K command palette · Ctrl+L model",
                    ),
            )
            .into_any_element()
    }
}

/// Discrete thinking-level control (`ZCode` composer: model name + thinking slider).
fn thinking_slider(level: ThinkingLevel, cx: &mut Context<SessionPane>) -> AnyElement {
    const LEVELS: [ThinkingLevel; 6] = [
        ThinkingLevel::Off,
        ThinkingLevel::Minimal,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::Xhigh,
    ];
    let theme = cx.theme().clone();
    let current = LEVELS.iter().position(|l| *l == level).unwrap_or(0);
    h_flex()
        .gap(px(2.))
        .px_1()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .mr_1()
                .child(SharedString::from(level.to_string())),
        )
        .children(LEVELS.iter().enumerate().map(|(i, l)| {
            let l = *l;
            div()
                .id(SharedString::from(format!("think{i}")))
                .w(px(10.))
                .h(px(6.))
                .rounded_sm()
                .cursor_pointer()
                .bg(if i <= current && current > 0 {
                    theme.primary
                } else {
                    theme.muted
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_thinking(l, cx)))
        }))
        .into_any_element()
}

fn card(source: Option<String>, harness: bool, body: AnyElement, cx: &App) -> AnyElement {
    let rail = if harness {
        theme::harness()
    } else {
        cx.theme().border
    };
    h_flex()
        .w_full()
        .items_start()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
        .child(div().w(px(3.)).min_h(px(28.)).h_full().bg(rail))
        .child(
            v_flex()
                .flex_1()
                .p_2()
                .gap_1()
                .text_sm()
                .when_some(source, |el, s| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(if harness {
                                theme::harness()
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(SharedString::from(s)),
                    )
                })
                .child(body),
        )
        .into_any_element()
}

impl Render for SessionPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(text) = self.pending_text.take() {
            self.composer
                .update(cx, |s, cx| s.set_value(text, window, cx));
        }
        self.sync_dialog(window, cx);
        let empty = self.model.transcript.is_empty();
        let conversation: AnyElement = if empty {
            self.render_empty(cx)
        } else {
            list(self.list.clone(), cx.processor(SessionPane::render_row))
                .size_full()
                .into_any_element()
        };
        let headers: Vec<AnyElement> = self
            .model
            .ui
            .headers()
            .enumerate()
            .map(|(i, h)| ous::render(h, &format!("hdr{i}"), &ous::no_action(), cx))
            .chain(self.render_widgets(WidgetPlacement::Header, cx))
            .collect();
        let composer = self.render_composer(window, cx);
        let overlay = self.render_overlay_panel(cx);
        let working = self
            .model
            .ui
            .working_message
            .clone()
            .filter(|_| self.model.is_busy());
        v_flex()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .when(!headers.is_empty(), |el| {
                el.child(v_flex().px_6().py_2().gap_1().children(headers))
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.render_rail(cx))
                    .child(div().flex_1().h_full().min_w_0().child(conversation)),
            )
            .when_some(working, |el, w| {
                el.child(
                    h_flex()
                        .px_6()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Spinner::new().xsmall())
                        .child(SharedString::from(w)),
                )
            })
            .child(div().w_full().px_6().pt_1().pb_3().child(composer))
            .children(overlay)
            .when_some(self.dialog.clone(), gpui_kit::ParentElement::child)
    }
}
