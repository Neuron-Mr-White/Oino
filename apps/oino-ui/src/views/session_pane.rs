//! One open session (gpui-app §3.1–§3.2, design.md §5.2): event pump, transcript list state,
//! composer, docked interactions, panels, side and bottom frame tabs. Rendering lives in
//! `components::*` (`impl SessionPane` blocks); this module keeps state and orchestration.
//!
//! Speed: events are drained in batches, coalesced, reduced once and turned into minimal
//! `ListState` updates (`splice` for inserts, `remeasure_items` for changed rows); the pump
//! waits one frame between batches, so a token burst costs one redraw. Rows render from
//! borrowed transcript rows (no per-frame row clones); each assistant row owns an
//! incremental markdown state.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui_kit::component::input::{InlineToken, InputEvent, TextareaState};
use gpui_kit::component::text::TextViewState;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FollowMode,
    ListAlignment, ListState, Pixels, Subscription, Task, Window, px,
};
use oino_ai::{AgentMessage, ImageContent, JsonValue, Model, ThinkingLevel};
use oino_client::{
    ClientError, FrontendApi as _, OinoCommand, PiCommand, RpcCommand, SessionBackend,
};
use oino_core::events::{ClientEvent, RunState};
use oino_core::session::{EditorEvent, EditorIntent};
use oino_core::ui::UiResponse;

use crate::components::interaction::{InteractionEvent, InteractionView};
use crate::design::brand::CrabState;
use crate::host::{AppSettings, OpenedSession};
use crate::model::coalesce::coalesce;
use crate::model::composer::{self, CompletionRow, Drafts, Intent, ModeChip, SubmitKey};
use crate::model::interaction::{self, InteractionKind};
use crate::model::session::{Applied, SessionModel};
use crate::model::sessions::LiveStatus;
use crate::model::status::StatusModel;
use crate::model::transcript::{NoticeTone, Row, RowChange, RowKind};
use crate::model::ui_state::{PanelPlacement, UiEffect};
use crate::rt::Tokio;

/// Frame pacing between event batches of the focused pane.
const FRAME: Duration = Duration::from_millis(8);
/// Pacing of the other split panes while several stream (≈ 30 fps; keeps the 4-pane frame
/// cost inside the §2.11 budget: each frame re-lays out about one background pane, not three).
const BACKGROUND_FRAME: Duration = Duration::from_millis(33);
/// Debounce of `editor_event{changed}` (ui-protocol §7.1).
const EDITOR_DEBOUNCE: Duration = Duration::from_millis(150);
/// Debounce of backend completions.
const COMPLETION_DEBOUNCE: Duration = Duration::from_millis(60);

/// Events for the workspace.
#[derive(Clone, Debug)]
pub enum PaneEvent {
    /// Perform an effect (toast, OS notification, URL, clipboard, theme, settings).
    Effect(UiEffect),
    /// Sidebar status changed.
    Live(LiveStatus),
    /// Title / id / model changed (sidebar + header refresh).
    Changed,
    /// The model picker should open (mobile sheet; desktop uses the composer menu).
    PickModel,
    /// A toast from the pane itself (copy confirmations, unsupported actions).
    Toast(String),
    /// The side frame should open (a tab was activated).
    ShowSide,
    /// The bottom frame should open.
    ShowBottom,
    /// The pane took focus (split panes).
    Focused,
    /// Show / hide the sidebar.
    ToggleSidebar,
    /// Show / hide the side frame.
    ToggleSide,
    /// Show / hide the bottom frame.
    ToggleBottom,
    /// Close this split pane (the session keeps running).
    ClosePane,
}

/// Side frame tab (design.md §4.20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideTab {
    /// Session file changes.
    Review,
    /// Workspace files.
    Files,
    /// Plan markdown.
    Plan,
    /// Read-only child agent (tool call id).
    Agent {
        /// Tool call id.
        call_id: String,
        /// Tab title.
        title: String,
    },
    /// Session tree.
    Tree,
    /// An extension `sidePane` panel.
    Panel(String),
    /// A single diff opened from a tool row.
    Diff {
        /// Title (path).
        title: String,
        /// Unified patch.
        patch: String,
    },
    /// The embedded browser (design.md A1.4).
    Browser,
    /// Tool output opened from a tool row.
    Output {
        /// Title.
        title: String,
        /// Text.
        text: String,
    },
}

impl SideTab {
    /// Whether the tab has a close button.
    #[must_use]
    pub fn closable(&self) -> bool {
        !matches!(self, Self::Review)
    }
}

/// Tabs of the side frame.
#[derive(Clone, Debug, Default)]
pub struct SidePane {
    /// Open tabs.
    pub tabs: Vec<SideTab>,
    /// Active tab.
    pub active: usize,
}

impl SidePane {
    /// Opens (or focuses) `tab`.
    pub fn open(&mut self, tab: SideTab) {
        if let Some(i) = self.tabs.iter().position(|t| *t == tab) {
            self.active = i;
        } else {
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
    }

    /// Closes tab `i`.
    pub fn close_at(&mut self, i: usize) -> Option<SideTab> {
        if i >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        Some(tab)
    }

    /// Closes every tab.
    pub fn close(&mut self) {
        self.tabs.clear();
        self.active = 0;
    }

    /// Whether a tab is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.tabs.is_empty()
    }

    /// The active tab.
    #[must_use]
    pub fn current(&self) -> Option<&SideTab> {
        self.tabs.get(self.active)
    }
}

/// Bottom frame tab (design.md §5.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BottomTab {
    /// Shell output of `!` commands and bash tool calls.
    Output,
    /// Background tasks.
    Tasks,
    /// An extension `bottomPane` panel.
    Panel(String),
    /// An interactive terminal tab (design.md A1.5; id into `SessionPane::terminals`).
    Terminal(u64),
}

/// One Terminal tab: one view, or two side by side after Split.
#[cfg(feature = "terminal")]
pub struct TerminalTab {
    /// Stable id (`BottomTab::Terminal`).
    pub id: u64,
    /// Views left to right.
    pub views: Vec<Entity<crate::components::terminal::TerminalView>>,
    subs: Vec<Subscription>,
}

#[cfg(feature = "terminal")]
impl TerminalTab {
    /// A tab with its first view.
    #[must_use]
    pub fn new(
        id: u64,
        view: Entity<crate::components::terminal::TerminalView>,
        sub: Subscription,
    ) -> Self {
        Self {
            id,
            views: vec![view],
            subs: vec![sub],
        }
    }

    /// Adds a split view.
    pub fn push(
        &mut self,
        view: Entity<crate::components::terminal::TerminalView>,
        sub: Subscription,
    ) {
        self.views.push(view);
        self.subs.push(sub);
    }
}

/// Open popover menus of the pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneMenu {
    /// Model menu (⌘M).
    Model,
    /// Thinking menu (⌘⇧M).
    Thinking,
    /// Workflow mode chip.
    Mode,
    /// Attach menu.
    Attach,
    /// Header `⋯`.
    More,
    /// Header project chip.
    Project,
    /// Meta row overflow.
    MetaOverflow,
}

/// Backend completion state of the composer.
#[derive(Default)]
pub struct Completion {
    /// Rows.
    pub rows: Vec<CompletionRow>,
    /// Highlighted row.
    pub active: usize,
    /// Char cursor the rows were computed for.
    pub cursor: usize,
    /// Pending request.
    pub task: Option<Task<()>>,
}

/// Find-in-conversation state (design.md §4.31).
pub struct Find {
    /// Query input.
    pub input: Entity<gpui_kit::component::input::InputState>,
    /// Matching row indices.
    pub matches: Vec<usize>,
    /// Current match.
    pub current: usize,
    /// Case sensitive.
    pub case: bool,
}

/// One open session.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent view flags (focus, unread, steering…) of one pane"
)]
pub struct SessionPane {
    /// Reduced session state.
    pub model: SessionModel,
    pub(crate) backend: Arc<dyn SessionBackend>,
    pub(crate) list: ListState,
    pub(crate) markdown: HashMap<String, (Entity<TextViewState>, usize)>,
    pub(crate) thinking_open: HashSet<String>,
    pub(crate) explore_open: HashSet<String>,
    pub(crate) commands: Vec<(String, Option<String>)>,
    /// Models with credentials (`get_available_models`).
    pub models: Vec<Model>,
    pub(crate) composer: Entity<TextareaState>,
    pub(crate) drafts: Drafts,
    pub(crate) pending_text: Option<String>,
    pub(crate) pending_tokens: Vec<(String, String, String)>,
    pub(crate) images: HashMap<String, ImageContent>,
    pub(crate) interaction: Option<Entity<InteractionView>>,
    /// Side frame tabs.
    pub side: SidePane,
    /// Bottom frame tabs and the active one.
    pub bottom: (Vec<BottomTab>, usize),
    pub(crate) settings: AppSettings,
    pub(crate) focus: FocusHandle,
    pub(crate) menu: Option<PaneMenu>,
    pub(crate) completion: Completion,
    pub(crate) find: Option<Find>,
    pub(crate) status: StatusModel,
    pub(crate) status_expanded: Option<bool>,
    pub(crate) width: Pixels,
    pub(crate) run_started: Option<Instant>,
    pub(crate) queue_open: bool,
    pub(crate) hint_index: usize,
    pub(crate) tree: Option<oino_view::tree::TreeModel>,
    pub(crate) files: crate::components::side_pane::FileTree,
    pub(crate) unread: bool,
    pub(crate) focused_pane: bool,
    pub(crate) rename: Option<Entity<gpui_kit::component::input::InputState>>,
    pub(crate) inputs: crate::components::ous::InputStore,
    /// Shell state reflected by the header.
    pub header_state: crate::components::header::HeaderState,
    /// Timeline keyboard cursor (row index) and whether the timeline has focus.
    pub(crate) cursor: Option<usize>,
    /// The composer shows the steering placeholder.
    steering: bool,
    /// `local_previews` keyed by (row count, last row version).
    #[cfg_attr(
        not(feature = "browser"),
        allow(dead_code, reason = "read by the browser preview chip only")
    )]
    previews: std::cell::RefCell<((usize, u64), Vec<String>)>,
    pub(crate) timeline_kb: bool,
    /// Side pane focus, scroll and the Review file cursor.
    pub(crate) side_focus: FocusHandle,
    pub(crate) side_scroll: gpui_kit::ScrollHandle,
    pub(crate) diff_cursor: Option<usize>,
    /// Press state of the header drag region (client-side chrome).
    pub(crate) drag_armed: std::rc::Rc<std::cell::Cell<bool>>,
    /// The side-pane browser (one per pane) and a URL waiting for it.
    #[cfg(feature = "browser")]
    pub browser: Option<Entity<crate::components::browser::BrowserView>>,
    pub(crate) browser_pending: Option<String>,
    /// Terminal tabs of the bottom frame.
    #[cfg(feature = "terminal")]
    pub terminals: Vec<TerminalTab>,
    #[cfg(feature = "terminal")]
    pub(crate) next_terminal: u64,
    editor_task: Option<Task<()>>,
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
    snapshot: Option<Vec<oino_core::ui::UiRequest>>,
    commands: Vec<(String, Option<String>)>,
    models: Vec<Model>,
}

/// Composer placeholder (design.md §4.11).
pub const PLACEHOLDER: &str = "Ask Oino anything, @ files, / commands, $ skills";
/// Composer placeholder while a run is active (design.md §4.11 steering).
pub const PLACEHOLDER_STEER: &str = "Steer Oino… (Enter steers, Alt+Enter queues a follow-up)";

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
        let list = ListState::new(0, ListAlignment::Top, px(crate::design::size::OVERDRAW));
        list.set_follow_mode(FollowMode::Tail);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 12)
                .submit_on_enter(true)
                .placeholder(PLACEHOLDER)
        });
        let subscriptions = vec![cx.subscribe_in(&composer, window, Self::on_composer)];
        let pump = Self::pump(&backend, cx);
        let files = crate::components::side_pane::FileTree::new(&opened.cwd);
        let mut pane = Self {
            model,
            backend,
            list,
            markdown: HashMap::new(),
            thinking_open: HashSet::new(),
            explore_open: HashSet::new(),
            commands: Vec::new(),
            models: Vec::new(),
            composer,
            drafts: Drafts::default(),
            pending_text: None,
            pending_tokens: Vec::new(),
            images: HashMap::new(),
            interaction: None,
            side: SidePane::default(),
            bottom: (Vec::new(), 0),
            settings,
            focus: cx.focus_handle(),
            menu: None,
            completion: Completion::default(),
            find: None,
            status: StatusModel::default(),
            status_expanded: None,
            width: px(crate::design::size::READING),
            run_started: None,
            queue_open: true,
            hint_index: 0,
            tree: None,
            files,
            unread: false,
            focused_pane: true,
            rename: None,
            inputs: crate::components::ous::InputStore::default(),
            header_state: crate::components::header::HeaderState::default(),
            drag_armed: std::rc::Rc::default(),
            cursor: None,
            steering: false,
            previews: std::cell::RefCell::default(),
            timeline_kb: false,
            side_focus: cx.focus_handle(),
            side_scroll: gpui_kit::ScrollHandle::new(),
            diff_cursor: None,
            #[cfg(feature = "browser")]
            browser: None,
            browser_pending: None,
            #[cfg(feature = "terminal")]
            terminals: Vec::new(),
            #[cfg(feature = "terminal")]
            next_terminal: 1,
            editor_task: None,
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

    /// Whether `name` is a known slash command.
    #[must_use]
    pub fn has_command(&self, name: &str) -> bool {
        self.commands.iter().any(|(n, _)| n == name)
    }

    /// Desktop preferences.
    #[must_use]
    pub fn settings(&self) -> &AppSettings {
        &self.settings
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

    /// Unicrab state of this session.
    #[must_use]
    pub fn crab(&self) -> CrabState {
        match self.live() {
            LiveStatus::Attention => CrabState::NeedsYou,
            LiveStatus::Running => CrabState::Working,
            LiveStatus::Unread => CrabState::Done,
            LiveStatus::Idle if self.unread => CrabState::Done,
            LiveStatus::Idle => CrabState::Idle,
        }
    }

    /// The status panel model.
    #[must_use]
    pub fn status(&self) -> &StatusModel {
        &self.status
    }

    /// The workflow mode chip, when `unipi.workflow` is loaded.
    #[must_use]
    pub fn mode(&self) -> Option<ModeChip> {
        if !self.has_command("unipi:permission") {
            return None;
        }
        Some(ModeChip::from_segments(self.model.ui.frame_segments()).unwrap_or(ModeChip::Auto))
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
            let mut batch = Vec::with_capacity(64);
            while let Some(first) = rx.next().await {
                batch.push(first);
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                }
                let reduced = coalesce(std::mem::take(&mut batch));
                let Ok(pace) =
                    this.update(cx, |pane, cx| {
                        pane.apply_batch(&reduced, cx);
                        if pane.focused_pane {
                            FRAME
                        } else {
                            // A per-session phase so background panes do not all re-lay out in
                            // the same frame.
                            let phase =
                                pane.model.id.bytes().fold(0u64, |h, b| {
                                    h.wrapping_mul(31).wrapping_add(u64::from(b))
                                }) % 7;
                            BACKGROUND_FRAME + Duration::from_millis(phase * 3)
                        }
                    })
                else {
                    break;
                };
                cx.background_executor().timer(pace).await;
            }
        })
    }

    fn apply_batch(&mut self, batch: &[ClientEvent], cx: &mut Context<Self>) {
        let before = (self.live(), self.model.title(), self.model.id.clone());
        let was_busy = self.model.is_busy();
        let applied = self.model.apply_all(batch);
        if self.model.is_busy() && !was_busy {
            self.run_started = Some(Instant::now());
        }
        if !self.model.is_busy() && was_busy {
            self.run_started = None;
            self.unread = !self.focused_pane;
        }
        self.absorb(applied, cx);
        let after = (self.live(), self.model.title(), self.model.id.clone());
        if before.0 != after.0 {
            cx.emit(PaneEvent::Live(after.0));
        }
        if before.1 != after.1 || before.2 != after.2 {
            cx.emit(PaneEvent::Changed);
        }
    }

    pub(crate) fn absorb(&mut self, applied: Applied, cx: &mut Context<Self>) {
        for change in &applied.rows {
            match *change {
                RowChange::Inserted { at, count } => {
                    self.list.splice(at..at, count);
                    // A grouped run's header row changes its label when a row joins.
                    if at > 0 {
                        self.list.remeasure_items(at - 1..at);
                    }
                }
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
                UiEffect::InsertToken { token_id, label } => {
                    self.pending_tokens
                        .push((token_id, format!("[{label}]"), label));
                    cx.notify();
                }
                UiEffect::FocusPanel(id) => self.focus_panel(&id, cx),
                UiEffect::OpenUrl(url) => self.open_link(&url, cx),
                other => cx.emit(PaneEvent::Effect(other)),
            }
        }
        // Text deltas of the streaming row (the hot path) do not change the status panel.
        let structural = applied.chrome
            || applied
                .rows
                .iter()
                .any(|c| !matches!(c, RowChange::Updated { at } if self.model.transcript.is_streaming() && *at + 1 == self.model.transcript.len()));
        if structural {
            self.status = StatusModel::from_session(&self.model);
            if let Some(find) = &mut self.find {
                find.matches.clear();
            }
        }
        if applied.reload {
            self.reload(cx);
        }
        if applied.chrome || !applied.rows.is_empty() {
            cx.notify();
        }
    }

    fn focus_panel(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.model.ui.panel(id).map(|p| p.placement) {
            Some(PanelPlacement::SidePane) => {
                self.side.open(SideTab::Panel(id.to_owned()));
                cx.emit(PaneEvent::ShowSide);
            }
            Some(PanelPlacement::BottomPane) => {
                let tab = BottomTab::Panel(id.to_owned());
                if let Some(i) = self.bottom.0.iter().position(|t| *t == tab) {
                    self.bottom.1 = i;
                } else {
                    self.bottom.0.push(tab);
                    self.bottom.1 = self.bottom.0.len() - 1;
                }
                cx.emit(PaneEvent::ShowBottom);
            }
            _ => {}
        }
    }

    /// Keeps one incremental markdown state per assistant row in sync.
    fn sync_markdown(&mut self, changes: &[RowChange], cx: &mut Context<Self>) {
        let mut touched: Vec<usize> = Vec::new();
        for change in changes {
            match *change {
                RowChange::Inserted { at, count } => touched.extend(at..at + count),
                RowChange::Updated { at } => touched.push(at),
                RowChange::Reset => touched.extend(0..self.model.transcript.len()),
            }
        }
        touched.sort_unstable();
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
            if let Some((state, len)) = self.markdown.get_mut(id) {
                let text = &row.text;
                let appended = text.len() >= *len && text.is_char_boundary(*len);
                if appended && text.len() == *len {
                    continue;
                }
                let delta = if appended { &text[*len..] } else { "" };
                *len = text.len();
                state.update(cx, |s, cx| {
                    if appended {
                        s.push_str(delta, cx);
                    } else {
                        s.set_text(text, cx);
                    }
                });
            } else {
                let len = row.text.len();
                let state = cx.new(|cx| TextViewState::markdown(&row.text, cx).selectable(true));
                self.markdown.insert(id.clone(), (state, len));
            }
        }
    }

    pub(crate) fn send(&self, command: RpcCommand, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move { backend.send(command).await });
        cx.spawn(async move |this, cx| {
            let error = match task.await {
                Ok(Ok(_)) => return,
                Ok(Err(e)) => e,
                Err(e) => ClientError::Transport(e.to_string()),
            };
            let _ = this.update(cx, |pane, cx| pane.notice(&error.to_string(), cx));
        })
        .detach();
    }

    pub(crate) fn notice(&mut self, text: &str, cx: &mut Context<Self>) {
        let change = self.model.transcript.notice(NoticeTone::Error, text);
        self.absorb(
            Applied {
                rows: vec![change],
                ..Applied::default()
            },
            cx,
        );
    }

    /// Runs a frontend call on the runtime and reports failures as a toast.
    pub(crate) fn frontend<F, Fut>(&self, cx: &mut Context<Self>, f: F)
    where
        F: FnOnce(Arc<dyn SessionBackend>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), ClientError>> + Send + 'static,
    {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move { f(backend).await });
        cx.spawn(async move |this, cx| {
            let error = match task.await {
                Ok(Ok(())) => return,
                Ok(Err(e)) => e.to_string(),
                Err(e) => e.to_string(),
            };
            let _ = this.update(cx, |_, cx| cx.emit(PaneEvent::Toast(error)));
        })
        .detach();
    }

    /// Sends `extension_ui_action{panelId, actionId, value}` (panel buttons, lists, inputs).
    pub fn panel_action(
        &mut self,
        panel: &str,
        action: &str,
        value: JsonValue,
        cx: &mut Context<Self>,
    ) {
        let (panel, action) = (panel.to_owned(), action.to_owned());
        let value = (!value.is_null()).then_some(value);
        self.frontend(cx, move |b| async move {
            b.ui_action(&panel, &action, value).await
        });
    }

    /// Invokes an extension shortcut.
    pub fn invoke_shortcut(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_owned();
        self.frontend(cx, move |b| async move { b.invoke_shortcut(&id).await });
    }

    fn say_hello(&self, cx: &mut Context<Self>) {
        let caps = serde_json::json!({
            "dialogs": ["select","confirm","input","editor","multiSelect","form","approval","custom"],
            "ous": true,
            "panels": ["overlay","sidePane","editorReplace","bottomPane","fullscreen"],
            "images": false, "osNotify": true, "clipboard": true, "editorTokens": true
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
    pub(crate) fn set_composer_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.pending_text = Some(text.to_owned());
        cx.notify();
    }

    /// Applies pending composer edits (called from render, which has the window).
    pub(crate) fn flush_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let busy = self.model.is_busy();
        if busy != self.steering {
            self.steering = busy;
            self.composer.update(cx, |s, cx| {
                s.set_placeholder(
                    if busy { PLACEHOLDER_STEER } else { PLACEHOLDER },
                    window,
                    cx,
                );
            });
        }
        if let Some(text) = self.pending_text.take() {
            let len = text.len();
            self.composer.update(cx, |s, cx| {
                s.set_value(text, window, cx);
                s.set_selected_range(len..len, cx);
            });
            // Programmatic edits do not emit `Change`: refresh completions / drafts here.
            self.on_text_changed(cx);
        }
        for (id, text, label) in std::mem::take(&mut self.pending_tokens) {
            self.composer.update(cx, |s, cx| {
                let token = InlineToken::new(id, text).with_label(label);
                if s.replace_with_token(token, window, cx).is_ok() {
                    s.insert(" ", window, cx);
                }
            });
        }
    }

    fn on_composer(
        &mut self,
        _: &Entity<TextareaState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter {
                shift: false,
                secondary,
            } => {
                if !self.completion.rows.is_empty() {
                    self.accept_completion(window, cx);
                    return;
                }
                let key = if *secondary {
                    SubmitKey::FollowUp
                } else {
                    SubmitKey::Enter
                };
                self.submit(key, cx);
            }
            InputEvent::Change => {
                self.on_text_changed(cx);
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_text_changed(&mut self, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let s = self.composer.read(cx);
            let text = s.value().to_string();
            let byte = s.cursor().min(text.len());
            let chars = text.get(..byte).map_or(0, |b| b.chars().count());
            (text, chars)
        };
        self.drafts.save(&self.model.id, &text);
        // Editor intents (debounced `changed`, immediate `cleared`).
        let intent = if text.is_empty() {
            Some(EditorIntent::Cleared)
        } else {
            None
        };
        let backend = Arc::clone(&self.backend);
        let report = text.clone();
        self.editor_task = Some(cx.spawn(async move |_, cx| {
            if intent.is_none() {
                cx.background_executor().timer(EDITOR_DEBOUNCE).await;
            }
            let event = EditorEvent {
                event_id: format!("e{}", Instant::now().elapsed().as_nanos()),
                intent: intent.unwrap_or(EditorIntent::Changed),
                text: report,
                token_id: None,
            };
            let _ = cx
                .background_executor()
                .spawn(async move { backend.editor_event(event).await })
                .await;
        }));
        // Completions.
        let Some((prefix, from)) = composer::trigger(&text, cursor) else {
            self.completion = Completion::default();
            return;
        };
        let local = if prefix == '/' {
            composer::slash_completions(&self.commands, &text, 8)
                .into_iter()
                .map(|c| CompletionRow {
                    section: "Commands".into(),
                    value: c.value.trim_end().to_owned(),
                    label: c.label,
                    description: c.description,
                    tag: None,
                    tone: None,
                    from,
                })
                .collect()
        } else {
            Vec::new()
        };
        self.completion.rows = local;
        self.completion.active = 0;
        self.completion.cursor = cursor;
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move {
            tokio::time::sleep(COMPLETION_DEBOUNCE).await;
            backend.completions(&text, cursor).await
        });
        self.completion.task = Some(cx.spawn(async move |this, cx| {
            if let Ok(Ok(groups)) = task.await {
                let _ = this.update(cx, |pane, cx| {
                    let rows = composer::merge_completions(&groups, from, 40);
                    if !rows.is_empty() {
                        pane.completion.rows = rows;
                        pane.completion.active = 0;
                    }
                    cx.notify();
                });
            }
        }));
    }

    /// Accepts the highlighted completion (files/skills become tokens).
    pub(crate) fn accept_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.completion.rows.get(self.completion.active).cloned() else {
            return;
        };
        let text = self.composer.read(cx).value().to_string();
        let cursor = self.completion.cursor;
        self.completion = Completion::default();
        let tokenize = (row.value.starts_with('@') && !row.value.ends_with('/'))
            || row.value.starts_with('$')
            || row.value.starts_with("/skill:");
        if tokenize {
            let (before, _) = composer::apply_completion(&text, cursor, row.from, "");
            let before = before.trim_end().to_owned();
            let prefix_len = before.len();
            self.composer.update(cx, |s, cx| {
                s.set_value(before, window, cx);
                s.set_selected_range(prefix_len..prefix_len, cx);
                let token = InlineToken::new(row.value.clone(), row.value.clone())
                    .with_label(row.label.clone());
                if s.replace_with_token(token, window, cx).is_ok() {
                    s.insert(" ", window, cx);
                }
            });
        } else {
            let (new, new_cursor) = composer::apply_completion(&text, cursor, row.from, &row.value);
            let byte = new
                .char_indices()
                .nth(new_cursor)
                .map_or(new.len(), |(i, _)| i);
            self.composer.update(cx, |s, cx| {
                s.set_value(new, window, cx);
                s.set_selected_range(byte..byte, cx);
            });
        }
        cx.notify();
    }

    /// Submits the composer.
    pub fn submit(&mut self, key: SubmitKey, cx: &mut Context<Self>) {
        if self.model.ui.needs_attention() {
            cx.emit(PaneEvent::Toast("Answer the pending request first".into()));
            return;
        }
        let (text, token_ids) = {
            let s = self.composer.read(cx);
            (
                s.value().to_string(),
                s.tokens()
                    .iter()
                    .map(|t| t.token().id().to_string())
                    .collect::<Vec<_>>(),
            )
        };
        let intent = composer::intent(&text, self.model.is_busy(), key);
        if intent == Intent::Nothing {
            return;
        }
        let images: Vec<ImageContent> = token_ids
            .iter()
            .filter_map(|id| self.images.get(id).cloned())
            .collect();
        let backend = Arc::clone(&self.backend);
        let report = text.clone();
        Tokio::detach(cx, async move {
            let _ = backend
                .editor_event(EditorEvent {
                    event_id: "submit".into(),
                    intent: EditorIntent::Submit,
                    text: report,
                    token_id: None,
                })
                .await;
        });
        self.drafts.history(&self.model.id.clone()).push(&text);
        self.drafts.save(&self.model.id, "");
        if let Some(command) = intent.command(images) {
            self.send(command, cx);
        }
        self.list.scroll_to_end();
        self.set_composer_text("", cx);
    }

    /// Aborts the current run.
    pub fn abort(&mut self, cx: &mut Context<Self>) {
        self.send(PiCommand::Abort.into(), cx);
    }

    /// Selects a model.
    pub fn set_model(&mut self, provider: String, model_id: String, cx: &mut Context<Self>) {
        self.menu = None;
        self.send(PiCommand::SetModel { provider, model_id }.into(), cx);
    }

    /// Sets the thinking level.
    pub fn set_thinking(&mut self, level: ThinkingLevel, cx: &mut Context<Self>) {
        self.menu = None;
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

    /// Clones the session at its leaf (header `⋯` Fork).
    pub fn fork_session(&mut self, cx: &mut Context<Self>) {
        self.send(PiCommand::Clone.into(), cx);
    }

    /// Edit & resend: forks before the `nth` user message and puts its text in the composer.
    pub fn fork_at_user(&mut self, nth: usize, text: String, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move {
            let data = backend
                .send(PiCommand::GetForkMessages.into())
                .await
                .ok()
                .flatten();
            let entry = data
                .as_ref()
                .and_then(|d| d.get("messages"))
                .and_then(JsonValue::as_array)
                .and_then(|m| m.get(nth))
                .and_then(|m| m.get("entryId"))
                .and_then(JsonValue::as_str)
                .map(str::to_owned);
            match entry {
                Some(entry_id) => backend
                    .send(PiCommand::Fork { entry_id }.into())
                    .await
                    .map(drop),
                None => Err(ClientError::Protocol("message not found".into())),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |pane, cx| match result {
                Ok(Ok(())) => pane.set_composer_text(&text, cx),
                Ok(Err(e)) => cx.emit(PaneEvent::Toast(e.to_string())),
                Err(e) => cx.emit(PaneEvent::Toast(e.to_string())),
            });
        })
        .detach();
    }

    /// Exports the session as HTML.
    pub fn export_html(&mut self, cx: &mut Context<Self>) {
        self.send(PiCommand::ExportHtml { output_path: None }.into(), cx);
        cx.emit(PaneEvent::Toast("Exported the session as HTML".into()));
    }

    /// Focuses the composer of `pane`.
    pub fn focus(pane: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let composer = pane.read(cx).composer.clone();
        composer.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Composer key handling (history, completions, abort, work tray).
    pub(crate) fn on_composer_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::design::perf::mark(cx, "keystroke");
        let text = self.composer.read(cx).value().to_string();
        let session = self.model.id.clone();
        let key = event.keystroke.key.as_str();
        let rows = self.completion.rows.len();
        if rows > 0 {
            match key {
                "down" => self.completion.active = (self.completion.active + 1) % rows,
                "up" => self.completion.active = (self.completion.active + rows - 1) % rows,
                "tab" => self.accept_completion(window, cx),
                "escape" => self.completion = Completion::default(),
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "up" && crate::components::secondary(&event.keystroke.modifiers) {
            self.focus_timeline(window, cx);
            cx.stop_propagation();
            return;
        }
        let card = self
            .interaction
            .as_ref()
            .filter(|v| !v.read(cx).is_modal())
            .cloned();
        if let Some(card) = card
            && ((key == "up" && text.is_empty())
                || (key == "tab" && event.keystroke.modifiers.shift))
        {
            card.update(cx, |v, cx| v.focus_card(window, cx));
            cx.stop_propagation();
            return;
        }
        match key {
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
            "down" if text.is_empty() => {
                // Consumable intent: the work tray opens when an extension takes it.
                let backend = Arc::clone(&self.backend);
                Tokio::detach(cx, async move {
                    let _ = backend
                        .editor_event(EditorEvent {
                            event_id: "down".into(),
                            intent: EditorIntent::EmptyArrowDown,
                            text: String::new(),
                            token_id: None,
                        })
                        .await;
                });
                cx.stop_propagation();
            }
            "enter" if event.keystroke.modifiers.alt => {
                self.submit(SubmitKey::FollowUp, cx);
                cx.stop_propagation();
            }
            "escape" if self.model.is_busy() && text.is_empty() => {
                self.abort(cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    // ── interactions & panels ────────────────────────────────────────────────

    fn respond(&mut self, response: UiResponse, cx: &mut Context<Self>) {
        self.model.ui.dismiss(&response.id);
        self.interaction = None;
        self.backend.ui_respond(response);
        cx.emit(PaneEvent::Live(self.live()));
        cx.notify();
    }

    /// Keeps the docked interaction view in sync with the front of the dialog queue.
    pub(crate) fn sync_interaction(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let front = self.model.ui.dialog().cloned();
        let current = self
            .interaction
            .as_ref()
            .map(|d| d.read(cx).dialog().id.clone());
        match front {
            Some(dialog) if current.as_deref() != Some(dialog.id.as_str()) => {
                if interaction::kind(&dialog) == InteractionKind::PlanReview
                    && self.width
                        >= px(crate::design::tokens::breakpoint::WIDE - crate::design::size::SIDEBAR)
                {
                    self.side.open(SideTab::Plan);
                    cx.emit(PaneEvent::ShowSide);
                }
                let count = self.model.ui.dialog_count();
                let typing = self
                    .composer
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx);
                let view = cx.new(|cx| InteractionView::new(dialog, count, typing, window, cx));
                cx.subscribe(&view, |this, _, event: &InteractionEvent, cx| match event {
                    InteractionEvent::Respond(response) => this.respond(response.clone(), cx),
                    InteractionEvent::OpenPlan => {
                        this.side.open(SideTab::Plan);
                        cx.emit(PaneEvent::ShowSide);
                    }
                })
                .detach();
                self.interaction = Some(view);
            }
            None if self.interaction.is_some() => {
                self.interaction = None;
                let composer = self.composer.clone();
                composer.update(cx, |s, cx| s.focus(window, cx));
            }
            _ => {
                if let Some(view) = &self.interaction {
                    let count = self.model.ui.dialog_count();
                    view.update(cx, |v, _| v.set_count(count));
                }
            }
        }
    }

    /// Closes a panel and tells the owning extension (`$close`).
    pub fn close_panel(&mut self, id: &str, cx: &mut Context<Self>) {
        self.model.ui.close_panel(id);
        if let Some(i) = self
            .side
            .tabs
            .iter()
            .position(|t| *t == SideTab::Panel(id.to_owned()))
        {
            self.side.close_at(i);
        }
        self.bottom
            .0
            .retain(|t| *t != BottomTab::Panel(id.to_owned()));
        self.panel_action(id, "$close", JsonValue::Null, cx);
        cx.notify();
    }

    /// Toggles a tool / summary row.
    pub(crate) fn toggle_row(&mut self, id: &str, cx: &mut Context<Self>) {
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

    /// Opens `url` in the side-pane Browser tab (design.md A1.4), or the system browser when
    /// the embedded one is unavailable.
    pub fn open_link(&mut self, url: &str, cx: &mut Context<Self>) {
        if crate::platform::BrowserSupport::get(cx) {
            self.browser_pending = Some(url.to_owned());
            self.open_side(SideTab::Browser, cx);
        } else {
            cx.emit(PaneEvent::Effect(UiEffect::OpenUrl(url.to_owned())));
        }
    }

    /// Shows / hides this pane's native browser view (`shown` = its tab is visible and no
    /// overlay covers the side pane).
    pub fn sync_browser(&mut self, shown: bool, cx: &mut Context<Self>) {
        #[cfg(feature = "browser")]
        if let Some(b) = &self.browser {
            let shown = shown && self.side.current() == Some(&SideTab::Browser);
            b.update(cx, |b, cx| b.set_shown(shown, cx));
        }
        #[cfg(not(feature = "browser"))]
        let _ = (shown, cx);
    }

    /// Local dev-server URLs printed by shell commands of this session (newest first).
    #[must_use]
    pub fn local_previews(&self) -> Vec<String> {
        #[cfg(feature = "browser")]
        {
            let rows = self.model.transcript.rows();
            let key = (rows.len(), rows.last().map_or(0, |r| r.version));
            if let Ok(cache) = self.previews.try_borrow()
                && cache.0 == key
            {
                return cache.1.clone();
            }
            let mut out: Vec<String> = Vec::new();
            for r in self.model.transcript.rows().iter().rev() {
                let text = match &r.kind {
                    RowKind::Bash(b) => b.output.as_str(),
                    RowKind::Tool(t)
                        if crate::model::tools::ToolKind::of(&t.name)
                            == crate::model::tools::ToolKind::Shell =>
                    {
                        t.output.as_str()
                    }
                    _ => continue,
                };
                for u in crate::components::browser::local_urls(text) {
                    if !out.contains(&u) {
                        out.push(u);
                    }
                }
                if out.len() >= 3 {
                    break;
                }
            }
            if let Ok(mut cache) = self.previews.try_borrow_mut() {
                *cache = (key, out.clone());
            }
            out
        }
        #[cfg(not(feature = "browser"))]
        Vec::new()
    }

    /// Opens a side tab and asks the shell to show the side frame.
    pub fn open_side(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if matches!(tab, SideTab::Tree) {
            self.load_tree(cx);
        }
        self.side.open(tab);
        cx.emit(PaneEvent::ShowSide);
        cx.notify();
    }

    /// Loads the session tree for the Tree tab.
    pub fn load_tree(&mut self, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(
            cx,
            async move { backend.send(PiCommand::GetTree.into()).await },
        );
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(data))) = task.await {
                let _ = this.update(cx, |pane, cx| {
                    pane.tree = Some(oino_view::tree::TreeModel::from_json(&data, None));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Switches the session leaf to `entry_id` (Tree tab).
    pub fn navigate_tree(&mut self, entry_id: &str, cx: &mut Context<Self>) {
        self.send(
            OinoCommand::NavigateTree {
                target_id: entry_id.to_owned(),
                options: oino_core::session::NavigateTreeOptions::default(),
            }
            .into(),
            cx,
        );
    }

    /// Records the last measured width of the conversation frame.
    pub fn set_width(&mut self, width: Pixels) {
        self.width = width;
    }

    /// Marks this pane focused or not (split panes, unread state).
    pub fn set_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        self.focused_pane = focused;
        if focused {
            self.unread = false;
        }
        cx.notify();
    }

    /// Opens a popover menu (toggle).
    pub fn toggle_menu(&mut self, menu: PaneMenu, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) {
            None
        } else {
            Some(menu)
        };
        cx.notify();
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
}
