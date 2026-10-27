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
use std::fmt::Write as _;
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
use oino_client::protocol::QueueKind;
use oino_client::{
    ClientError, FrontendApi as _, OinoCommand, PiCommand, RewindOptions, RpcCommand,
    SessionBackend,
};
use oino_core::events::{ClientEvent, OinoEvent, RunState};
use oino_core::session::{EditorEvent, EditorIntent};
use oino_core::ui::UiResponse;

use crate::components::interaction::{InteractionEvent, InteractionView};
use crate::design::brand::CrabState;
use crate::host::{AppSettings, OpenedSession};
use crate::model::coalesce::coalesce;
use crate::model::composer::{self, Builtin, CompletionRow, Drafts, Intent, ModeChip, SubmitKey};
use crate::model::interaction::{self, InteractionKind};
use crate::model::session::{Applied, SessionModel};
use crate::model::sessions::LiveStatus;
use crate::model::status::StatusModel;
use crate::model::timeline::AgentFeed;
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
    /// Send chevron: Steer / Queue (A2.11).
    Send,
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
    editor_task: Option<Task<Result<(), tokio::task::JoinError>>>,
    /// Live child agents (`_oino/agents` feed): agent rows and the Agent tab.
    pub agents: AgentFeed,
    /// Open harness cards (collapsed by default, A2.10).
    pub(crate) cards_open: HashSet<String>,
    /// Prompt numbers (nth user message) with a file checkpoint (`_oino/checkpoints`).
    pub(crate) checkpoints: HashSet<usize>,
    /// Queue item being edited in place: (queue, index, field).
    pub(crate) queue_edit: Option<(
        QueueKind,
        usize,
        Entity<gpui_kit::component::input::InputState>,
    )>,
    /// When the last user prompt was sent (turn end "Done in …").
    pub(crate) turn_started: Option<Instant>,
    /// Duration and cost of finished turns, keyed by the last row id of the turn.
    pub(crate) turn_stats: HashMap<String, (Duration, f64)>,
    turn_cost_start: f64,
    /// Last mode the footer showed: only a fallback while the topic has no permission mode
    /// (turning the footer off keeps it, GA-25).
    mode_seen: Option<ModeChip>,
    /// Window y of the composer shell's top (caps the completion menu above it).
    pub(crate) composer_top: std::rc::Rc<std::cell::Cell<f32>>,
    /// Focus of an `editorReplace` panel in the composer slot, and the panel it was given to.
    pub(crate) panel_focus: (FocusHandle, Option<String>),
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

/// Composer placeholder (design.md A2.11).
pub const PLACEHOLDER: &str = "Message Oino, / commands, @ files";
/// Composer placeholder while a run is active (A2.11 steering).
pub const PLACEHOLDER_STEER: &str = "Steer Oino…";
/// The workflow status topic (mode chip, GA-25).
pub const WORKFLOW_TOPIC: &str = "unipi:workflow:status";

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
            agents: AgentFeed::default(),
            cards_open: HashSet::new(),
            checkpoints: HashSet::new(),
            queue_edit: None,
            turn_started: None,
            turn_stats: HashMap::new(),
            turn_cost_start: 0.0,
            mode_seen: None,
            composer_top: std::rc::Rc::default(),
            panel_focus: (cx.focus_handle(), None),
            _pump: pump,
            _subscriptions: subscriptions,
        };
        // The greeting's local hour spawns `date` once: keep it off the UI thread.
        cx.background_spawn(async {
            crate::components::conversation::utc_offset();
        })
        .detach();
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

    /// The workflow mode chip, when `unipi.workflow` is loaded. It reads the sticky
    /// `unipi:workflow:status` topic, never footer segments (GA-25).
    #[must_use]
    pub fn mode(&self) -> Option<ModeChip> {
        if !self.has_command("permission") {
            return None;
        }
        Some(
            self.model
                .topic(WORKFLOW_TOPIC)
                .and_then(ModeChip::from_topic)
                .or(self.mode_seen)
                .unwrap_or(ModeChip::Auto),
        )
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
        let _activity = crate::stability::activity("pane apply_batch");
        let before = (self.live(), self.model.title(), self.model.id.clone());
        let was_busy = self.model.is_busy();
        let mut feed = self.apply_feeds(batch);
        let applied = self.model.apply_all(batch);
        if let Some(m) = ModeChip::from_segments(self.model.ui.frame_segments()) {
            self.mode_seen = Some(m);
        }
        if self.model.is_busy() && !was_busy {
            self.run_started = Some(Instant::now());
            if self.turn_started.is_none() {
                self.turn_started = self.run_started;
                self.turn_cost_start = self.model.usage.cost;
            }
        }
        if !self.model.is_busy() && was_busy {
            self.run_started = None;
            self.unread = !self.focused_pane;
            if let (Some(started), Some(last)) = (
                self.turn_started.take(),
                self.model.transcript.rows().last(),
            ) {
                let cost = (self.model.usage.cost - self.turn_cost_start).max(0.0);
                self.turn_stats
                    .insert(last.id.clone(), (started.elapsed(), cost));
                if let Some(ix) = self.model.transcript.position(&last.id) {
                    self.list.remeasure_items(ix..ix + 1);
                }
            }
            self.load_checkpoints(cx);
            feed = true;
        }
        if feed {
            cx.notify();
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
        self.subscribe_feeds(cx);
    }

    /// Subscribes to the sticky topic feed and the nested agents feed (frontend-api §2–§3).
    /// Called again after a session replacement (fresh snapshot).
    fn subscribe_feeds(&self, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move {
            let topics = backend
                .topics(&["unipi:*", oino_core::topics::GIT_TOPIC], &[])
                .await
                .unwrap_or_default();
            let agents = backend.agents(Some(true)).await.ok();
            (topics, agents)
        });
        cx.spawn(async move |this, cx| {
            if let Ok((topics, agents)) = task.await {
                let _ = this.update(cx, |pane, cx| {
                    for t in topics {
                        pane.model.set_topic(&t.topic, t.data);
                    }
                    if let Some(tree) = agents {
                        pane.agents.set_tree(&tree.agents);
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Applies agent events of a batch (child transcripts; the session model keeps the tree
    /// and topics). Returns
    /// whether anything visible changed.
    fn apply_feeds(&mut self, batch: &[ClientEvent]) -> bool {
        let mut changed = false;
        for event in batch {
            match event {
                ClientEvent::Oino(OinoEvent::Topic { topic, .. }) if topic == WORKFLOW_TOPIC => {
                    changed = true;
                }
                ClientEvent::Oino(OinoEvent::AgentsChanged { agents }) => {
                    self.agents.set_tree(agents);
                    changed = true;
                }
                ClientEvent::Oino(OinoEvent::AgentEvent { agent_id, event }) => {
                    changed |= self.agents.apply(agent_id, event);
                }
                _ => {}
            }
        }
        if changed {
            // Agent rows show the child's latest action.
            let rows = self.model.transcript.rows();
            for (i, r) in rows.iter().enumerate() {
                if let RowKind::Tool(t) = &r.kind
                    && crate::model::tools::ToolKind::of(&t.name)
                        == crate::model::tools::ToolKind::Agent
                {
                    self.list.remeasure_items(i..i + 1);
                }
            }
        }
        changed
    }

    /// Refreshes the user entries that have file checkpoints (undo / rewind buttons).
    pub(crate) fn load_checkpoints(&mut self, cx: &mut Context<Self>) {
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move {
            let list = backend.checkpoints().await.ok()?;
            if list.is_empty() {
                return Some(HashSet::new());
            }
            let data = backend
                .send(PiCommand::GetForkMessages.into())
                .await
                .ok()
                .flatten()?;
            let entries: Vec<String> = data
                .get("messages")?
                .as_array()?
                .iter()
                .filter_map(|m| m.get("entryId")?.as_str().map(str::to_owned))
                .collect();
            Some(
                entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| list.iter().any(|c| &c.entry_id == *e))
                    .map(|(i, _)| i)
                    .collect::<HashSet<usize>>(),
            )
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Some(set)) = task.await {
                let _ = this.update(cx, |pane, cx| {
                    if pane.checkpoints != set {
                        pane.checkpoints = set;
                        pane.list.remeasure();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
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
        let _activity = crate::stability::activity("pane load history");
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
        // Reconcile the dialog queue with core's snapshot: a card core no longer has (its
        // handler died, or the session was replaced) must not block the composer (GA-30).
        if let Some(snapshot) = &loaded.snapshot {
            let live: HashSet<&str> = snapshot.iter().map(|r| r.id.as_str()).collect();
            let stale: Vec<String> = self
                .model
                .ui
                .dialog_ids()
                .into_iter()
                .filter(|id| !live.contains(id.as_str()))
                .collect();
            for id in stale {
                self.model.ui.dismiss(&id);
            }
        }
        self.absorb(applied, cx);
        self.subscribe_feeds(cx);
        self.load_checkpoints(cx);
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
        // Backend futures need the tokio runtime (the in-process core `tokio::spawn`s): never
        // poll them on a GPUI executor. A panic there killed a GPUI worker thread, and the
        // main thread then panicked polling the dead task (`Task polled after completion`).
        // Dropping the task (next keystroke) aborts the debounce and the call.
        self.editor_task = Some(Tokio::spawn(cx, async move {
            if intent.is_none() {
                tokio::time::sleep(EDITOR_DEBOUNCE).await;
            }
            let event = EditorEvent {
                event_id: format!("e{}", Instant::now().elapsed().as_nanos()),
                intent: intent.unwrap_or(EditorIntent::Changed),
                text: report,
                token_id: None,
            };
            let _ = backend.editor_event(event).await;
        }));
        // Completions.
        let Some((prefix, from)) = composer::trigger(&text, cursor) else {
            self.completion = Completion::default();
            return;
        };
        let builtins = if prefix == '/' {
            composer::builtin_completions(&text)
        } else {
            Vec::new()
        };
        let mut local: Vec<CompletionRow> = if prefix == '/' {
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
        local.extend(builtins.iter().cloned());
        self.completion.rows = local;
        self.completion.active = 0;
        self.completion.cursor = cursor;
        let asked = text.clone();
        let backend = Arc::clone(&self.backend);
        let task = Tokio::spawn(cx, async move {
            tokio::time::sleep(COMPLETION_DEBOUNCE).await;
            backend.completions(&text, cursor).await
        });
        self.completion.task = Some(cx.spawn(async move |this, cx| {
            if let Ok(Ok(groups)) = task.await {
                let _ = this.update(cx, |pane, cx| {
                    // A late answer for older text never replaces the rows of newer keys.
                    if pane.composer.read(cx).value().as_ref() != asked.as_str() {
                        return;
                    }
                    let mut rows = composer::merge_completions(&groups, from, 40);
                    for b in &builtins {
                        if !rows.iter().any(|r| r.value.trim_end() == b.value) {
                            rows.push(b.clone());
                        }
                    }
                    if !rows.is_empty() {
                        let keep = pane
                            .completion
                            .rows
                            .get(pane.completion.active)
                            .map(|r| r.value.clone());
                        pane.completion.active = keep
                            .and_then(|v| rows.iter().position(|r| r.value == v))
                            .unwrap_or(0);
                        pane.completion.rows = rows;
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
        if let Intent::Builtin(builtin, arg) = intent {
            // pi built-ins act locally and never reach the model (GA-3).
            self.drafts.history(&self.model.id.clone()).push(&text);
            self.drafts.save(&self.model.id, "");
            self.set_composer_text("", cx);
            self.run_builtin(builtin, arg, cx);
            return;
        }
        if text.trim() == "/subagents" {
            // The Subagents tab of the work tray, even before the first subagent (GA-21).
            self.drafts.history(&self.model.id.clone()).push(&text);
            self.set_composer_text("", cx);
            self.open_tray(Some("subagents"), Some(text.trim().to_owned()), cx);
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

    /// Sends a slash command (or any text) as a prompt; pi built-ins run locally.
    pub fn run_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        let intent = composer::intent(text, self.model.is_busy(), SubmitKey::Enter);
        if let Intent::Builtin(builtin, arg) = intent {
            self.run_builtin(builtin, arg, cx);
            return;
        }
        if let Some(command) = intent.command(Vec::new()) {
            self.send(command, cx);
        }
    }

    /// Runs a pi built-in slash command in the frontend (pi interactive mode, GA-3).
    #[allow(clippy::too_many_lines, reason = "one arm per built-in")]
    pub fn run_builtin(&mut self, builtin: Builtin, arg: Option<String>, cx: &mut Context<Self>) {
        match builtin {
            Builtin::New => self.new_session(cx),
            Builtin::Compact => self.send(
                PiCommand::Compact {
                    custom_instructions: arg,
                }
                .into(),
                cx,
            ),
            Builtin::Model => {
                let query = arg.unwrap_or_default().to_lowercase();
                let found = (!query.is_empty())
                    .then(|| {
                        self.models
                            .iter()
                            .find(|m| format!("{}/{}", m.provider, m.id).to_lowercase() == query)
                            .or_else(|| self.models.iter().find(|m| m.id.to_lowercase() == query))
                            .or_else(|| {
                                self.models.iter().find(|m| {
                                    format!("{}/{} {}", m.provider, m.id, m.name)
                                        .to_lowercase()
                                        .contains(&query)
                                })
                            })
                    })
                    .flatten()
                    .map(|m| (m.provider.clone(), m.id.clone()));
                if let Some((provider, model_id)) = found {
                    self.set_model(provider, model_id, cx);
                } else {
                    if !query.is_empty() {
                        self.toast(&format!("No model matches “{query}”"), cx);
                    }
                    self.menu = Some(PaneMenu::Model);
                    cx.notify();
                }
            }
            Builtin::Thinking => {
                if let Some(level) = arg.and_then(|a| a.parse::<ThinkingLevel>().ok()) {
                    self.set_thinking(level, cx);
                } else {
                    self.menu = Some(PaneMenu::Thinking);
                    cx.notify();
                }
            }
            Builtin::Name => {
                if let Some(name) = arg {
                    self.model.name = Some(name.clone());
                    self.send(PiCommand::SetSessionName { name }.into(), cx);
                    cx.emit(PaneEvent::Changed);
                } else {
                    let title = self.model.title();
                    self.toast(&format!("Session name: {title}"), cx);
                }
            }
            Builtin::Session => {
                let u = &self.model.usage;
                let text = format!(
                    "Session {} · {} · {} prompts · {}k in / {}k out · ${:.4}{}",
                    self.model.id,
                    self.model.model_label(),
                    self.model.transcript.prompt_indices().len(),
                    u.input / 1000,
                    u.output / 1000,
                    u.cost,
                    self.model
                        .file
                        .as_ref()
                        .map(|f| format!(" · {f}"))
                        .unwrap_or_default()
                );
                let change = self.model.transcript.notice(NoticeTone::Info, text);
                self.absorb(
                    Applied {
                        rows: vec![change],
                        ..Applied::default()
                    },
                    cx,
                );
                self.list.scroll_to_end();
            }
            Builtin::Tree => self.open_side(SideTab::Tree, cx),
            Builtin::Fork => {
                self.open_side(SideTab::Tree, cx);
                self.toast(
                    "Pick a message in the tree, or hover a prompt → Edit & resend",
                    cx,
                );
            }
            Builtin::Clone => self.fork_session(cx),
            Builtin::Copy => {
                let backend = Arc::clone(&self.backend);
                let task = Tokio::spawn(cx, async move {
                    backend
                        .send(PiCommand::GetLastAssistantText.into())
                        .await
                        .ok()
                        .flatten()
                });
                cx.spawn(async move |this, cx| {
                    let text =
                        task.await.ok().flatten().and_then(|d| {
                            d.get("text").and_then(JsonValue::as_str).map(str::to_owned)
                        });
                    let _ = this.update(cx, |_, cx| match text {
                        Some(t) if !t.is_empty() => {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(t));
                            cx.emit(PaneEvent::Toast("Copied the last reply".into()));
                        }
                        _ => cx.emit(PaneEvent::Toast("Nothing to copy yet".into())),
                    });
                })
                .detach();
            }
            Builtin::Export => {
                let jsonl = arg.as_deref().is_some_and(|a| {
                    std::path::Path::new(a)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
                });
                if jsonl {
                    self.send(OinoCommand::ExportJsonl { output_path: arg }.into(), cx);
                    cx.emit(PaneEvent::Toast("Exported the session as JSONL".into()));
                } else {
                    self.send(PiCommand::ExportHtml { output_path: arg }.into(), cx);
                    cx.emit(PaneEvent::Toast("Exported the session as HTML".into()));
                }
            }
            Builtin::Import => {
                if let Some(path) = arg {
                    self.send(OinoCommand::ImportJsonl { path }.into(), cx);
                } else {
                    self.toast("Usage: /import <path to .jsonl>", cx);
                }
            }
            Builtin::Settings | Builtin::Login | Builtin::Logout | Builtin::ScopedModels => {
                cx.emit(PaneEvent::Effect(UiEffect::OpenSettings {
                    namespace: None,
                }));
            }
            Builtin::Hotkeys => {
                self.toast(
                    "Keyboard shortcuts are in Settings → Keyboard shortcuts",
                    cx,
                );
                cx.emit(PaneEvent::Effect(UiEffect::OpenSettings {
                    namespace: None,
                }));
            }
            Builtin::Resume => self.toast("Pick a session in the list on the left", cx),
            Builtin::Reload => {
                self.reload(cx);
                self.toast("Reloaded the session state", cx);
            }
            Builtin::Changelog => {
                if self.has_command("changelog") {
                    self.send(
                        PiCommand::Prompt {
                            message: "/changelog".into(),
                            images: Vec::new(),
                            streaming_behavior: None,
                        }
                        .into(),
                        cx,
                    );
                } else {
                    self.toast("No changelog in this build", cx);
                }
            }
            Builtin::Trust => self.toast("Project trust is saved in Settings → Permissions", cx),
            Builtin::Extensions => self.toast("Open Extensions from the sidebar", cx),
            Builtin::Quit => cx.quit(),
        }
    }

    /// Opens the work tray (`core.open-tray`), on `tab` when given. Falls back to sending
    /// `fallback` as a prompt when the tray service is missing.
    pub fn open_tray(
        &mut self,
        tab: Option<&str>,
        fallback: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let backend = Arc::clone(&self.backend);
        let payload = match tab {
            Some(tab) => serde_json::json!({"tab": tab}),
            None => serde_json::json!({}),
        };
        let task = Tokio::spawn(cx, async move {
            backend
                .service_call(Some("unipi.core"), "core.open-tray", payload)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            tracing::debug!(?result, "core.open-tray");
            let ok = matches!(result, Ok(Ok(_)));
            if !ok && let Some(text) = fallback {
                let _ = this.update(cx, |pane, cx| {
                    pane.send(
                        PiCommand::Prompt {
                            message: text,
                            images: Vec::new(),
                            streaming_behavior: None,
                        }
                        .into(),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    // ── queue (frontend-api §4) ──────────────────────────────────────────────

    /// Index of row `i` of the combined queue strip (steering first) in its own queue.
    fn queue_slot(&self, i: usize) -> (QueueKind, usize) {
        let steer = self.model.steering.len();
        if i < steer {
            (QueueKind::Steering, i)
        } else {
            (QueueKind::FollowUp, i - steer)
        }
    }

    /// Removes queue item `i`.
    pub fn queue_remove(&mut self, i: usize, cx: &mut Context<Self>) {
        let (queue, index) = self.queue_slot(i);
        self.queue_edit = None;
        self.frontend(
            cx,
            move |b| async move { b.queue_remove(queue, index).await },
        );
    }

    /// Sends queue item `i` now (front of steering while running).
    pub fn queue_send_now(&mut self, i: usize, cx: &mut Context<Self>) {
        let (queue, index) = self.queue_slot(i);
        self.frontend(
            cx,
            move |b| async move { b.queue_send_now(queue, index).await },
        );
    }

    /// Moves queue item `i` one up within its queue.
    pub fn queue_up(&mut self, i: usize, cx: &mut Context<Self>) {
        let (queue, index) = self.queue_slot(i);
        if index == 0 {
            return;
        }
        self.frontend(cx, move |b| async move {
            b.queue_move(queue, index, index - 1).await
        });
    }

    /// Starts editing queue item `i` in place.
    pub fn queue_begin_edit(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (queue, index) = self.queue_slot(i);
        let text = match queue {
            QueueKind::Steering => self.model.steering.get(index),
            QueueKind::FollowUp => self.model.follow_up.get(index),
        }
        .cloned()
        .unwrap_or_default();
        let input = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx)
                .default_value(gpui_kit::SharedString::from(text))
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.queue_commit_edit(cx);
            }
        })
        .detach();
        self.queue_edit = Some((queue, index, input));
        cx.notify();
    }

    /// Saves the in-place queue edit (`queue_set_text`).
    pub fn queue_commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some((queue, index, input)) = self.queue_edit.take() else {
            return;
        };
        let text = input.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.frontend(
                cx,
                move |b| async move { b.queue_remove(queue, index).await },
            );
        } else {
            self.frontend(cx, move |b| async move {
                b.queue_set_text(queue, index, &text).await
            });
        }
        cx.notify();
    }

    // ── undo / rewind (frontend-api §5) ──────────────────────────────────────

    /// Whether the `nth` user prompt has a file checkpoint (its turn changed files).
    #[must_use]
    pub fn has_checkpoint(&self, nth: usize) -> bool {
        self.checkpoints.contains(&nth)
    }

    /// Rewinds to before the `nth` user prompt: restores the files its turn and later turns
    /// changed and, with `conversation`, forks the session before it and puts the prompt
    /// back in the composer.
    pub fn rewind_at(&mut self, nth: usize, conversation: bool, cx: &mut Context<Self>) {
        if self.model.is_busy() {
            self.toast("Stop the run first", cx);
            return;
        }
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
                .map(str::to_owned)
                .ok_or_else(|| ClientError::Protocol("message not found".into()))?;
            backend
                .rewind(
                    &entry,
                    RewindOptions {
                        files: true,
                        conversation,
                    },
                )
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |pane, cx| match result {
                Ok(Ok(r)) if r.cancelled => pane.toast("An extension cancelled the rewind", cx),
                Ok(Ok(r)) => {
                    if let Some(text) = r.text.filter(|_| conversation) {
                        pane.set_composer_text(&text, cx);
                    }
                    let n = r.restored.len() + r.deleted.len();
                    let mut msg = if n == 0 {
                        if conversation {
                            "Rewound the conversation".to_owned()
                        } else {
                            "No file changes to undo".to_owned()
                        }
                    } else {
                        format!("Restored {n} file{}", if n == 1 { "" } else { "s" })
                    };
                    if !r.failed.is_empty() {
                        let _ = write!(msg, " · {} failed", r.failed.len());
                    }
                    pane.toast(&msg, cx);
                    pane.load_checkpoints(cx);
                }
                Ok(Err(e)) => pane.toast(&e.to_string(), cx),
                Err(e) => pane.toast(&e.to_string(), cx),
            });
        })
        .detach();
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
        let key = event.keystroke.key.as_str();
        if self.composer_key(key, event.keystroke.modifiers, window, cx) {
            cx.stop_propagation();
        }
    }

    /// The editor's own key bindings (↑ ↓ Tab Esc) run as actions before key-down
    /// listeners, so the composer intercepts those actions in the capture phase and routes
    /// them here (completion menu, history, work tray, abort).
    pub(crate) fn composer_action(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_key(key, gpui_kit::Modifiers::default(), window, cx) {
            cx.stop_propagation();
        }
    }

    /// Composer keys; returns whether the key was handled.
    #[allow(clippy::too_many_lines, reason = "one arm per composer key")]
    fn composer_key(
        &mut self,
        key: &str,
        modifiers: gpui_kit::Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = self.composer.read(cx).value().to_string();
        let session = self.model.id.clone();
        let rows = self.completion.rows.len();
        if rows > 0 {
            match key {
                "down" => self.completion.active = (self.completion.active + 1) % rows,
                "up" => self.completion.active = (self.completion.active + rows - 1) % rows,
                "tab" => self.accept_completion(window, cx),
                "escape" => self.completion = Completion::default(),
                _ => return false,
            }
            cx.notify();
            return true;
        }
        if key == "v" && crate::components::secondary(&modifiers) && self.paste_image(cx) {
            return true;
        }
        if key == "up" && crate::components::secondary(&modifiers) {
            self.focus_timeline(window, cx);
            return true;
        }
        let card = self
            .interaction
            .as_ref()
            .filter(|v| !v.read(cx).is_modal())
            .cloned();
        if let Some(card) = card
            && ((key == "up" && text.is_empty()) || (key == "tab" && modifiers.shift))
        {
            card.update(cx, |v, cx| v.focus_card(window, cx));
            return true;
        }
        match key {
            "up" if text.is_empty() || self.drafts.history(&session).is_browsing() => {
                if let Some(prev) = self.drafts.history(&session).older().map(str::to_owned) {
                    self.composer
                        .update(cx, |s, cx| s.set_value(prev, window, cx));
                    return true;
                }
                false
            }
            "down" if self.drafts.history(&session).is_browsing() => {
                if let Some(next) = self.drafts.history(&session).newer().map(str::to_owned) {
                    self.composer
                        .update(cx, |s, cx| s.set_value(next, window, cx));
                    return true;
                }
                false
            }
            "down" if text.is_empty() => {
                // Consumable intent: the work tray opens when an extension takes it; when
                // nobody does, ask the tray service directly (GA-21).
                let backend = Arc::clone(&self.backend);
                let task = Tokio::spawn(cx, async move {
                    backend
                        .editor_event(EditorEvent {
                            event_id: "down".into(),
                            intent: EditorIntent::EmptyArrowDown,
                            text: String::new(),
                            token_id: None,
                        })
                        .await
                });
                cx.spawn(async move |this, cx| {
                    let consumed = task.await;
                    tracing::debug!(?consumed, "emptyArrowDown");
                    if let Ok(Ok(false)) = consumed {
                        let _ = this.update(cx, |pane, cx| pane.open_tray(None, None, cx));
                    }
                })
                .detach();
                true
            }
            "enter" if modifiers.alt => {
                self.submit(SubmitKey::FollowUp, cx);
                true
            }
            "escape" if self.model.is_busy() && text.is_empty() => {
                self.abort(cx);
                true
            }
            _ => false,
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
                // The card takes the composer slot (A2.11), so it takes focus too; its 400 ms
                // arming delay keeps a keystroke meant for the draft from resolving it.
                let typing = false;
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
        crate::components::browser::hub::watch(self, cx);
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
