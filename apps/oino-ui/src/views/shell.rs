//! The window root (design.md A2.1 L1–L2, A2.8): flush columns on `bg` separated by 1 px
//! `line_soft` rules — the 224 px nav sidebar, then for Sessions the session list column
//! (400, resizable 280–560) and the detail (one visible session, its side pane and bottom
//! frame) — and the Board, Extensions, Settings and onboarding pages. Command palette,
//! switch sheet (Ctrl+O), toasts, result cards, dialogs and the frame-time overlay.
//!
//! One session is visible at a time. Other open sessions keep running in the background
//! (warm sessions); hidden idle ones beyond [`MAX_WARM_SESSIONS`] shut down and reopen from
//! the list. There are no split panes (A2.18).

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, PathPromptOptions, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    linear_color_stop, linear_gradient, px,
};

use crate::components::header::HeaderState;
use crate::components::palette::{PaletteEvent, PaletteView};
use crate::components::toast::{ToastKind, Toasts, is_long, stack};
use crate::design::brand::CrabState;
use crate::design::perf;
use crate::design::theme::{self, OinoTheme, TextExt as _};
use crate::design::tokens::breakpoint;
use crate::design::{TextRole, radius, size, space};
use crate::host::{AppSettings, ExtensionInfo, OpenRequest, OpenedSession, SessionHost, ThemePref};
use crate::model::kanban;
use crate::model::palette::PaletteAction;
use crate::model::sessions::{LiveStatus, SessionList, SessionSummary};
use crate::model::ui_state::{NotifyKind, UiEffect};
use crate::rt::Tokio;
use crate::views::session_pane::{BottomTab, PaneEvent, PaneMenu, SessionPane, SideTab};
use crate::views::settings_page::{SettingsEvent, SettingsSection, SettingsView};
use crate::views::{
    Abort, CloseFind, CommandPalette, FindInConversation, FocusComposer, GoBoard, GoCron,
    GoExtensions, GoSessions, NewSession, OpenFolder, OpenSettings, PickModel, PickThinking,
    SwitchSession, ToggleBottom, ToggleFrameOverlay, ToggleSidePane, ToggleSidebar,
    ToggleStatusPanel, ToggleTheme, TreeView,
};

/// Main-area routes (the sidebar nav).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Session list and the visible session.
    Sessions,
    /// Settings page.
    Settings,
    /// Kanboard board (D9).
    Board,
    /// Extensions page (D12).
    Extensions,
    /// Automations — the cron view (§8 row 49).
    Cron,
    /// First-run onboarding (D13).
    Onboarding,
}

/// Which splitter is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    List,
    Side,
    Bottom,
}

/// One open (live) session: its view, focus stamp and event subscription.
pub(crate) struct LiveSession {
    pub(crate) key: u64,
    pub(crate) entity: Entity<SessionPane>,
    /// Focus order stamp (least recently used hidden idle sessions are released first).
    last_used: u64,
    _subscription: Subscription,
    _topics: Option<Task<()>>,
}

/// Open sessions kept warm (the visible one and running / waiting ones never count against
/// it). Every in-process session holds a core session with its extension state, so hidden
/// idle sessions beyond this are shut down; they reopen from the list.
pub const MAX_WARM_SESSIONS: usize = 8;

/// The sessions to release so at most `max` stay: hidden (`!visible`), idle ones, least
/// recently used first. Facts are `(key, last_used, visible, idle)`.
#[must_use]
pub fn sessions_to_release(sessions: &[(u64, u64, bool, bool)], max: usize) -> Vec<u64> {
    let excess = sessions.len().saturating_sub(max);
    let mut idle: Vec<&(u64, u64, bool, bool)> = sessions
        .iter()
        .filter(|(_, _, visible, idle)| !visible && *idle)
        .collect();
    idle.sort_by_key(|(_, used, _, _)| *used);
    idle.into_iter()
        .take(excess)
        .map(|(key, ..)| *key)
        .collect()
}

/// Bus topics every session subscribes to (frontend-api.md §2): workflow mode chip, status,
/// bg tasks, kanboard, fusion, footer, and core git.
pub const TOPICS: [&str; 2] = ["unipi:*", "oino:git"];

/// The switch sheet (Ctrl+O, A2.8): a filter field and the session rows.
pub(crate) struct SwitchSheet {
    pub(crate) input: Entity<gpui_kit::component::input::InputState>,
    pub(crate) cursor: usize,
    pub(crate) _subscription: Subscription,
}

/// The window root view.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent column visibility flags"
)]
pub struct AppShell {
    pub(crate) host: Arc<dyn SessionHost>,
    pub(crate) settings: AppSettings,
    pub(crate) sessions: SessionList,
    /// Open sessions (one visible, the others warm in the background).
    pub(crate) panes: Vec<LiveSession>,
    /// The visible session.
    pub(crate) focused: Option<u64>,
    /// The detail column shows [`Self::focused`] (Back / Esc closes it to the placeholder).
    pub(crate) detail_open: bool,
    next_key: u64,
    /// Focus stamps of [`LiveSession::last_used`].
    use_counter: u64,
    pub(crate) route: Route,
    pub(crate) sidebar_open: bool,
    /// The session list column (Ctrl+B folds it before the sidebar).
    pub(crate) list_open: bool,
    pub(crate) side_open: bool,
    pub(crate) bottom_open: bool,
    pub(crate) list_width: Pixels,
    side_width: Pixels,
    bottom_height: Pixels,
    drag: Option<Drag>,
    pub(crate) sidebar_menu: Option<String>,
    pub(crate) palette: Option<(Entity<PaletteView>, Subscription)>,
    pub(crate) switch: Option<SwitchSheet>,
    pub(crate) settings_view: Option<(Entity<SettingsView>, Subscription)>,
    pub(crate) extensions: Vec<ExtensionInfo>,
    toasts: Toasts,
    toast_task: Option<Task<()>>,
    /// A multi-line notify shown as a result card (GA-24): `(title, body)`.
    pub(crate) result_card: Option<(SharedString, SharedString)>,
    pub(crate) opening: bool,
    pub(crate) history: (Vec<String>, usize),
    pub(crate) window_width: Pixels,
    pub(crate) update_available: Option<String>,
    /// A list action waiting for its session to open (rename / fork).
    pub(crate) pending_side: Option<(String, SideAction)>,
    /// Project section `⋯` menu open for this cwd.
    pub(crate) group_menu: Option<String>,
    /// Extensions page row `⋮` menu open for this id.
    pub(crate) ext_menu: Option<String>,
    /// Sections showing every session (others show [`SECTION_ROWS`]).
    pub(crate) list_expanded: HashSet<String>,
    /// The needs-you drawer (header crab / strip ▾) is open.
    pub(crate) needs_drawer: bool,
    /// A destructive action waiting for confirmation.
    pub(crate) confirm: Option<Confirm>,
    /// Keyboard focus of the session list and its cursor row.
    pub(crate) sidebar_focus: FocusHandle,
    pub(crate) sidebar_cursor: Option<usize>,
    pub(crate) sidebar_kb: bool,
    /// Press state of the sidebar brand-row drag region.
    pub(crate) drag_armed: Rc<std::cell::Cell<bool>>,
    /// Extensions page README dialog `(title, markdown)`.
    pub(crate) ext_readme: Option<(String, String)>,
    /// D9 board toolbar / cursor / overlay state.
    pub(crate) board_ui: crate::views::board::BoardUi,
    /// Automations (cron) view state.
    pub(crate) cron_ui: crate::views::cron::CronUi,
    /// The mascot window (opt-in) and its bubble.
    pub(crate) mascot: Option<gpui_kit::WindowHandle<crate::views::mascot::MascotView>>,
    pub(crate) mascot_bubble: Option<(String, Instant)>,
    /// Extension shortcut collisions already explained once (`ctrl+b` while typing).
    shortcut_notes: HashSet<String>,
    /// The side frame was laid out this frame (browser visibility).
    side_shown: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Former name of [`AppShell`] (kept so batches landing in parallel compile; A2.18 renames).
pub type Workspace = AppShell;

/// Rows a project section shows before "Show all".
pub const SECTION_ROWS: usize = 6;

/// A list action applied once its session is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideAction {
    /// Start the header rename.
    Rename,
    /// Clone the session into a new one.
    Fork,
}

/// Confirmation dialog for an important action (app-convention).
#[derive(Clone)]
pub(crate) struct Confirm {
    pub title: SharedString,
    pub body: SharedString,
    pub action: SharedString,
    pub run: Rc<dyn Fn(&mut AppShell, &mut Window, &mut Context<AppShell>)>,
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

impl AppShell {
    /// The root view; lists sessions and reopens the last session (GA-11), else starts one in
    /// the default directory.
    pub fn new(host: Arc<dyn SessionHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = host.app_settings();
        theme::install(&settings, Some(window), cx);
        let probe = cx.default_global::<perf::FrameProbe>();
        probe.log = std::env::var("OINO_FRAME_LOG").is_ok_and(|v| v == "1");
        probe.overlay = std::env::var("OINO_FRAME_OVERLAY").is_ok_and(|v| v == "1");
        let shell = cx.entity().downgrade();
        let main_window = window.window_handle();
        let subscriptions = vec![
            cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
                if this.settings.theme == ThemePref::System {
                    theme::install(&this.settings, Some(window), cx);
                    cx.notify();
                }
            }),
            // Extension shortcuts (`setShortcuts`, GA-16): matched before key bindings and
            // the composer, so `alt+p`, `shift+down`… reach the extension.
            cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle() != main_window {
                    return;
                }
                let _ = shell.update(cx, |this, cx| this.on_keystroke(event, window, cx));
            }),
        ];
        // D13 onboarding (`OINO_DESKTOP_ONBOARDING=1`, also with the demo backend).
        let onboarding = std::env::var("OINO_DESKTOP_ONBOARDING").is_ok_and(|v| v == "1");
        let list_width = px(f32::from(settings.list_width).clamp(size::LIST_MIN, size::LIST_MAX));
        let mut ws = Self {
            host,
            settings,
            sessions: SessionList::new(),
            panes: Vec::new(),
            focused: None,
            detail_open: true,
            next_key: 0,
            use_counter: 0,
            route: if onboarding {
                Route::Onboarding
            } else {
                Route::Sessions
            },
            sidebar_open: true,
            list_open: true,
            side_open: false,
            bottom_open: false,
            list_width,
            side_width: px(size::SIDE_PANE),
            bottom_height: px(size::BOTTOM),
            drag: None,
            sidebar_menu: None,
            palette: None,
            switch: None,
            settings_view: None,
            extensions: Vec::new(),
            toasts: Toasts::default(),
            toast_task: None,
            result_card: None,
            opening: false,
            history: (Vec::new(), 0),
            window_width: window.viewport_size().width,
            update_available: None,
            pending_side: None,
            group_menu: None,
            ext_menu: None,
            list_expanded: HashSet::new(),
            needs_drawer: false,
            confirm: None,
            sidebar_focus: cx.focus_handle(),
            sidebar_cursor: None,
            sidebar_kb: false,
            drag_armed: Rc::default(),
            board_ui: crate::views::board::BoardUi::default(),
            cron_ui: crate::views::cron::CronUi::default(),
            ext_readme: None,
            mascot: None,
            mascot_bubble: None,
            shortcut_notes: HashSet::new(),
            side_shown: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        let root = cx.entity_id();
        crate::design::ticker::Ticker::set_root(cx, root);
        ws.refresh_sessions(cx);
        ws.load_extensions(cx);
        // After the main window exists (opening a window inside its build is not allowed).
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, Self::sync_mascot);
        })
        .detach();
        ws.open(ws.startup_request(), window, cx);
        if let Some(config) = crate::views::soak::SoakConfig::get(cx) {
            crate::views::soak::start(config, window, cx);
        }
        // Opt-in automation server (`OINO_DESKTOP_AUTOMATION`), docs/dev/v1/gui-driver.md.
        crate::automation::install(cx, window);
        if let Ok(open) = std::env::var("OINO_DESKTOP_OPEN") {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(2500))
                    .await;
                let _ = this.update_in(cx, |ws, window, cx| ws.open_surface(&open, window, cx));
            })
            .detach();
        }
        ws
    }

    /// The first session: the last one shown when its file still exists (GA-11), else a new
    /// session in the default folder.
    fn startup_request(&self) -> OpenRequest {
        let fresh =
            std::env::var("OINO_DESKTOP_NEW").is_ok_and(|v| v == "1") || !self.settings.reopen_last;
        if !fresh
            && let Some(path) = self.settings.last_session.as_ref().map(PathBuf::from)
            && path.is_file()
        {
            let cwd = self
                .settings
                .last_session_cwd
                .clone()
                .map_or_else(|| self.host.info().default_cwd, PathBuf::from);
            return OpenRequest::Resume { path, cwd };
        }
        OpenRequest::New {
            cwd: self.default_cwd(),
        }
    }

    /// Folder new sessions start in: Settings → General, else the host default.
    pub(crate) fn default_cwd(&self) -> PathBuf {
        self.settings
            .default_cwd
            .as_ref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| self.host.info().default_cwd)
    }

    /// Opens a named surface (`palette`, `settings[:section]`, `board`, `extensions`, `side`,
    /// `switch`, …; screenshot and evidence hooks, design.md §10.1 P9).
    pub fn open_surface(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match name.split_once(':').unwrap_or((name, "")) {
            ("palette", q) => self.open_palette(q, window, cx),
            ("switch", _) => self.open_switch(window, cx),
            ("settings", ns) => self.open_settings((!ns.is_empty()).then_some(ns), window, cx),
            ("board", _) => self.open_board(window, cx),
            ("cron", _) => self.open_cron(window, cx),
            ("extensions", _) => self.run_app_action("extensions", window, cx),
            ("model", _) => self.with_pane(cx, |p, cx| p.toggle_menu(PaneMenu::Model, cx)),
            ("needs", _) => {
                self.needs_drawer = true;
                cx.notify();
            }
            ("placeholder", _) => {
                self.detail_open = false;
                cx.notify();
            }
            ("result", _) => {
                self.show_result(
                    "Memory status".into(),
                    "migration: needed — run /memory repair\nrecords: 128 (12 pinned)\nlast \
                     dream: 2h ago\nindex: 4.1 MB, healthy\nsidecar: running (pid 4123)"
                        .into(),
                    cx,
                );
            }
            ("bottom", tab) => {
                self.bottom_open = true;
                let tab = tab.to_owned();
                self.with_pane(cx, move |p, cx| {
                    if p.bottom.0.is_empty() {
                        p.bottom.0 = vec![BottomTab::Output, BottomTab::Tasks];
                    }
                    p.bottom.1 = match tab.as_str() {
                        "tasks" => 1,
                        _ => 0,
                    };
                    cx.notify();
                });
                cx.notify();
            }
            ("terminal", _) => {
                self.bottom_open = true;
                cx.notify();
            }
            ("browser", url) => {
                let url = url.to_owned();
                self.side_open = true;
                self.with_pane(cx, move |p, cx| p.open_link(&url, cx));
            }
            ("side", tab) => {
                self.side_open = true;
                let tab = match tab {
                    "files" => Some(SideTab::Files),
                    "tree" => Some(SideTab::Tree),
                    "plan" => Some(SideTab::Plan),
                    "launcher" => None,
                    _ => Some(SideTab::Review),
                };
                self.with_pane(cx, move |p, cx| {
                    if let Some(tab) = tab {
                        p.open_side(tab, cx);
                    } else {
                        p.side.close();
                        cx.notify();
                    }
                });
                cx.notify();
            }
            ("find", q) => {
                let q = q.to_owned();
                if let Some(p) = self.active_pane().cloned() {
                    p.update(cx, |p, cx| {
                        p.open_find(window, cx);
                        if let Some(f) = &p.find {
                            f.input.update(cx, |s, cx| s.set_value(q, window, cx));
                        }
                    });
                }
            }
            ("composer", text) => {
                let text = text.to_owned();
                self.with_pane(cx, move |p, cx| p.set_composer_text(&text, cx));
            }
            ("toast", _) => {
                self.toast(
                    ToastKind::Info,
                    Some("Answered on another device".into()),
                    "The approval was resolved in the TUI",
                    cx,
                );
                self.toast(
                    ToastKind::Warning,
                    None,
                    "Context at 84 % — compaction suggested",
                    cx,
                );
                self.toast(
                    ToastKind::Error,
                    Some("Provider error".into()),
                    "429 rate limited; retrying in 4 s",
                    cx,
                );
            }
            ("session-menu", _) => {
                let id = self.active_pane().map(|p| p.read(cx).model.id.clone());
                self.sidebar_menu = id;
                cx.notify();
            }
            ("delete", _) => {
                if let Some(s) = self
                    .sessions
                    .groups()
                    .into_iter()
                    .flat_map(|g| g.sessions)
                    .find(|e| e.summary.path.is_some())
                {
                    self.confirm_delete(&s.summary.id, &s.summary.title, s.summary.path.clone());
                    cx.notify();
                }
            }
            // Perf hook: synthetic keystrokes into the composer (keystroke → paint latency).
            ("type", _) => {
                if let Some(p) = self.active_pane().cloned() {
                    SessionPane::focus(&p, window, cx);
                }
                cx.spawn_in(window, async move |_, cx| {
                    const KEYS: [&str; 6] = ["h", "e", "l", "l", "o", "space"];
                    for i in 0..60 {
                        cx.background_executor()
                            .timer(Duration::from_millis(120))
                            .await;
                        let ok = cx.update(|window, cx| {
                            if let Ok(k) = gpui_kit::Keystroke::parse(KEYS[i % KEYS.len()]) {
                                window.dispatch_keystroke(k, cx);
                            }
                        });
                        if ok.is_err() {
                            break;
                        }
                    }
                })
                .detach();
            }
            ("onboarding", _) => {
                self.route = Route::Onboarding;
                cx.notify();
            }
            _ => {}
        }
    }

    pub(crate) fn active_pane(&self) -> Option<&Entity<SessionPane>> {
        let key = self.focused?;
        self.panes.iter().find(|p| p.key == key).map(|p| &p.entity)
    }

    /// What the automation driver's `state` method reports (docs/dev/v1/gui-driver.md §2).
    pub(crate) fn automation_state(&self, cx: &App) -> crate::automation::State {
        let pane = self.active_pane().map(|p| p.read(cx));
        crate::automation::State {
            route: match self.route {
                Route::Sessions => "sessions",
                Route::Settings => "settings",
                Route::Board => "board",
                Route::Extensions => "extensions",
                Route::Cron => "cron",
                Route::Onboarding => "onboarding",
            }
            .to_owned(),
            session: pane.as_ref().map(|p| p.model.id.clone()),
            session_name: pane.as_ref().and_then(|p| p.model.name.clone()),
            dialogs: pane
                .as_ref()
                .map(|p| {
                    p.model
                        .ui
                        .dialogs()
                        .map(|d| crate::automation::state::DialogInfo {
                            id: d.id.clone(),
                            method: d.method.clone(),
                            title: d.title.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            panes: self.panes.len(),
            busy: pane.as_ref().is_some_and(|p| p.model.is_busy()),
            open: [
                (self.palette.is_some(), "palette"),
                (self.switch.is_some(), "switch"),
                (self.side_open, "side"),
                (self.bottom_open, "bottom"),
                (self.settings_view.is_some(), "settings-view"),
            ]
            .iter()
            .filter(|(open, _)| *open)
            .map(|(_, name)| (*name).to_owned())
            .collect(),
        }
    }

    pub(crate) fn pane_by_key(&self, key: u64) -> Option<&Entity<SessionPane>> {
        self.panes.iter().find(|p| p.key == key).map(|p| &p.entity)
    }

    pub(crate) fn refresh_sessions(&self, cx: &mut Context<Self>) {
        let task = Tokio::spawn(cx, self.host.list_sessions());
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(list)) = task.await {
                let _ = this.update(cx, |ws, cx| {
                    ws.sessions.set_sessions(list);
                    ws.sync_open_sessions(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Open sessions are always listed (new sessions have no file until the first message).
    fn sync_open_sessions(&mut self, cx: &App) {
        for pane in &self.panes {
            let p = pane.entity.read(cx);
            let model = &p.model;
            let existing = self.sessions.get(&model.id).cloned();
            self.sessions.upsert(SessionSummary {
                id: model.id.clone(),
                path: model.file.as_ref().map(PathBuf::from),
                cwd: model.cwd.clone(),
                title: model.title(),
                modified_ms: existing.map_or_else(now_ms, |e| e.modified_ms),
                message_count: model.transcript.len(),
            });
            let live = p.live();
            // Done-unseen stays until the session is opened (A2.9).
            if !(live == LiveStatus::Idle && self.sessions.live(&model.id) == LiveStatus::Unread) {
                self.sessions.set_live(&model.id, live);
            }
        }
    }

    pub(crate) fn open(
        &mut self,
        request: OpenRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The Sessions screen shows at request time (a session that finishes opening later
        // never pulls the user off another page; onboarding stays up at first run).
        if self.route != Route::Onboarding {
            self.route = Route::Sessions;
            self.settings_view = None;
        }
        if let OpenRequest::Resume { path, .. } = &request {
            let path = path.display().to_string();
            if let Some(key) = self
                .panes
                .iter()
                .find(|p| p.entity.read(cx).model.file.as_deref() == Some(path.as_str()))
                .map(|p| p.key)
            {
                self.activate(key, window, cx);
                return;
            }
        }
        let (OpenRequest::New { cwd } | OpenRequest::Resume { cwd, .. }) = &request;
        self.settings.touch_cwd(&cwd.display().to_string());
        self.host.save_app_settings(&self.settings);
        self.opening = true;
        cx.notify();
        let task = Tokio::spawn(cx, self.host.open(request));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |ws, window, cx| {
                ws.opening = false;
                match result {
                    Ok(Ok(opened)) => ws.add_pane(&opened, window, cx),
                    Ok(Err(e)) => ws.toast(ToastKind::Error, None, e.to_string(), cx),
                    Err(e) => ws.toast(ToastKind::Error, None, e.to_string(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn add_pane(&mut self, opened: &OpenedSession, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let entity = cx.new(|cx| SessionPane::new(opened, settings, window, cx));
        let key = self.next_key;
        self.next_key += 1;
        let subscription =
            cx.subscribe_in(&entity, window, move |this, pane, event, window, cx| {
                this.on_pane_event(key, pane, event, window, cx);
            });
        let topics = Some(Self::subscribe_topics(&entity, cx));
        self.panes.push(LiveSession {
            key,
            entity,
            last_used: 0,
            _subscription: subscription,
            _topics: topics,
        });
        self.activate(key, window, cx);
        self.release_idle_panes(cx);
        self.sync_open_sessions(cx);
        self.run_pending_side(window, cx);
    }

    /// Subscribes the session to [`TOPICS`] (`_oino/topics`) and stores the sticky snapshot;
    /// later publishes arrive as `OinoEvent::Topic` through the pump. The subscription moves
    /// to replacement sessions in core, so this runs once per backend.
    fn subscribe_topics(pane: &Entity<SessionPane>, cx: &mut Context<Self>) -> Task<()> {
        use oino_client::FrontendApi as _;
        let backend = Arc::clone(pane.read(cx).backend());
        let task = Tokio::spawn(cx, async move { backend.topics(&TOPICS, &[]).await });
        let pane = pane.downgrade();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(values)) = task.await else {
                return;
            };
            let _ = pane.update(cx, |p, cx| {
                let mut changed = false;
                for v in values {
                    changed |= p.model.set_topic(&v.topic, v.data);
                }
                if changed {
                    cx.notify();
                }
            });
        })
    }

    /// Shows session `key` in the detail column and focuses it.
    pub(crate) fn activate(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        perf::mark(cx, "session switch");
        self.detail_open = true;
        self.focus_pane(key, window, cx);
        if let Some(id) = self.pane_by_key(key).map(|p| p.read(cx).model.id.clone()) {
            let (stack, at) = &mut self.history;
            stack.truncate(*at + usize::from(!stack.is_empty()));
            if stack.last() != Some(&id) {
                stack.push(id);
            }
            *at = stack.len().saturating_sub(1);
        }
        self.remember_last(cx);
        cx.notify();
    }

    /// Persists the visible session as the one to reopen at startup (GA-11).
    fn remember_last(&mut self, cx: &App) {
        let Some((file, cwd)) = self
            .active_pane()
            .map(|p| p.read(cx))
            .and_then(|p| p.model.file.clone().map(|f| (f, p.model.cwd.clone())))
        else {
            return;
        };
        if self.settings.last_session.as_deref() != Some(file.as_str()) {
            self.settings.last_session = Some(file);
            self.settings.last_session_cwd = Some(cwd);
            self.host.save_app_settings(&self.settings);
        }
    }

    /// Shuts down hidden idle sessions beyond [`MAX_WARM_SESSIONS`] (least recently used
    /// first).
    pub(crate) fn release_idle_panes(&mut self, cx: &mut Context<Self>) {
        let facts: Vec<(u64, u64, bool, bool)> = self
            .panes
            .iter()
            .map(|p| {
                let visible = self.focused == Some(p.key);
                let idle = p.entity.read(cx).live() == LiveStatus::Idle;
                (p.key, p.last_used, visible, idle)
            })
            .collect();
        // `OINO_DESKTOP_MAX_PANES` overrides the cap (soak comparisons).
        let max = std::env::var("OINO_DESKTOP_MAX_PANES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(MAX_WARM_SESSIONS);
        for key in sessions_to_release(&facts, max) {
            self.drop_pane(key, cx);
        }
    }

    /// Removes session `key` entirely and shuts it down (session deleted, released by
    /// [`Self::release_idle_panes`], or `ctx.shutdown()`). It stays listed when it has a file.
    pub(crate) fn drop_pane(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(at) = self.panes.iter().position(|p| p.key == key) else {
            return;
        };
        let pane = self.panes.remove(at);
        if self.focused == Some(key) {
            self.focused = None;
            self.detail_open = false;
        }
        let backend = Arc::clone(pane.entity.read(cx).backend());
        let id = pane.entity.read(cx).model.id.clone();
        if pane.entity.read(cx).model.file.is_none() {
            self.sessions.remove(&id);
        } else {
            self.sessions.set_live(&id, LiveStatus::Idle);
        }
        drop(pane);
        tracing::debug!(session = %id, "released a session");
        Tokio::detach(cx, async move { backend.shutdown().await });
        cx.notify();
    }

    pub(crate) fn focus_pane(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.focused = Some(key);
        self.use_counter += 1;
        let stamp = self.use_counter;
        if let Some(p) = self.panes.iter_mut().find(|p| p.key == key) {
            p.last_used = stamp;
        }
        for pane in &self.panes {
            let focused = pane.key == key;
            pane.entity.update(cx, |p, cx| p.set_focused(focused, cx));
        }
        if let Some(pane) = self.active_pane().cloned() {
            let id = pane.read(cx).model.id.clone();
            if self.sessions.live(&id) == LiveStatus::Unread {
                self.sessions.set_live(&id, LiveStatus::Idle);
            }
            SessionPane::focus(&pane, window, cx);
        }
    }

    /// Shows the next open session (soak driver: background sessions stream while another
    /// is visible).
    pub(crate) fn cycle_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let at = self
            .focused
            .and_then(|f| self.panes.iter().position(|p| p.key == f))
            .unwrap_or(0);
        if let Some(next) = self
            .panes
            .get((at + 1) % self.panes.len().max(1))
            .map(|p| p.key)
        {
            self.activate(next, window, cx);
        }
    }

    fn on_pane_event(
        &mut self,
        key: u64,
        pane: &Entity<SessionPane>,
        event: &PaneEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PaneEvent::Live(status) => {
                let id = pane.read(cx).model.id.clone();
                let status = if *status == LiveStatus::Idle
                    && self.sessions.live(&id) == LiveStatus::Running
                    && (self.focused != Some(key) || !self.detail_open)
                {
                    LiveStatus::Unread
                } else {
                    *status
                };
                self.sessions.set_live(&id, status);
                if self.settings.mascot_window {
                    let p = pane.read(cx);
                    let title = p.model.title();
                    match status {
                        LiveStatus::Attention => {
                            let what = p
                                .model
                                .ui
                                .dialog()
                                .map(|d| d.title.clone())
                                .unwrap_or_default();
                            self.mascot_say(format!("{title} needs you: {what}"), cx);
                        }
                        LiveStatus::Unread => self.mascot_say(format!("{title} finished"), cx),
                        _ => self.sync_mascot(cx),
                    }
                }
                if status == LiveStatus::Attention && !window.is_window_active() {
                    let title = pane
                        .read(cx)
                        .model
                        .ui
                        .dialog()
                        .map(|d| d.title.clone())
                        .unwrap_or_default();
                    // OS notification while unfocused (`notifyOnAsk`); the toast stays for
                    // when the user comes back.
                    crate::platform::OsNotifier::post(
                        cx,
                        "Oino needs you",
                        if title.is_empty() {
                            "A session is waiting for your answer".to_owned()
                        } else {
                            format!("Oino needs approval: {title}")
                        },
                    );
                    self.toast(ToastKind::Warning, Some("Oino needs you".into()), title, cx);
                }
                self.update_title(window, cx);
                cx.notify();
            }
            PaneEvent::Changed => {
                if pane.read(cx).model.shutdown_requested && !pane.read(cx).model.is_busy() {
                    // `ctx.shutdown()` (frontend-api.md §8): close this session, or the app
                    // when it was the only one.
                    if self.panes.len() <= 1 {
                        cx.quit();
                        return;
                    }
                    self.drop_pane(key, cx);
                    return;
                }
                self.sync_open_sessions(cx);
                if self.focused == Some(key) {
                    self.remember_last(cx);
                }
                cx.notify();
            }
            PaneEvent::PickModel => pane.update(cx, |p, cx| p.toggle_menu(PaneMenu::Model, cx)),
            PaneEvent::Effect(effect) => self.perform(effect.clone(), window, cx),
            PaneEvent::Toast(text) => self.toast(ToastKind::Info, None, text.clone(), cx),
            PaneEvent::ShowSide => {
                self.side_open = true;
                cx.notify();
            }
            PaneEvent::ShowBottom => {
                self.bottom_open = true;
                cx.notify();
            }
            PaneEvent::Focused => self.focus_pane(key, window, cx),
            PaneEvent::ToggleSidebar => self.run_app_action("toggle-sidebar", window, cx),
            PaneEvent::ToggleSide => {
                self.side_open = !self.side_open;
                cx.notify();
            }
            // The detail header's back button: close the detail (the session keeps running).
            PaneEvent::ClosePane => {
                if self.list_fits(window) || !self.sidebar_open {
                    self.detail_open = false;
                } else {
                    self.open_switch(window, cx);
                }
                cx.notify();
            }
            PaneEvent::ToggleBottom => {
                self.bottom_open = !self.bottom_open;
                cx.notify();
            }
        }
    }

    fn update_title(&self, window: &mut Window, cx: &App) {
        let needs = self
            .panes
            .iter()
            .any(|p| p.entity.read(cx).model.ui.needs_attention());
        window.set_window_title(if needs { "● Oino" } else { "Oino" });
    }

    /// Aggregate Unicrab state of every open session.
    pub(crate) fn crab(&self, cx: &App) -> CrabState {
        CrabState::aggregate(self.panes.iter().map(|p| p.entity.read(cx).crab()))
    }

    pub(crate) fn toast(
        &mut self,
        kind: ToastKind,
        title: Option<SharedString>,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.toasts.push(kind, title, message);
        if self.toast_task.is_none() {
            self.toast_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    let keep = this
                        .update(cx, |ws, cx| {
                            if ws.toasts.expire(Instant::now()) {
                                cx.notify();
                            }
                            let keep = ws.toasts.has_timed();
                            if !keep {
                                ws.toast_task = None;
                            }
                            keep
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    /// Opens a multi-line status report as a result card dialog (GA-24).
    pub(crate) fn show_result(
        &mut self,
        title: SharedString,
        body: SharedString,
        cx: &mut Context<Self>,
    ) {
        self.result_card = Some((title, body));
        cx.notify();
    }

    fn perform(&mut self, effect: UiEffect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            UiEffect::Notify { message, kind } => {
                let kind = match kind {
                    NotifyKind::Info => ToastKind::Info,
                    NotifyKind::Warning => ToastKind::Warning,
                    NotifyKind::Error => ToastKind::Error,
                };
                if message.contains("update") && message.contains("available") {
                    self.update_available = Some(message.clone());
                }
                if is_long(&message) {
                    let mut lines = message.lines();
                    let first = lines.next().unwrap_or_default().trim().to_owned();
                    // A short first line ending with ':' is the report's title.
                    let (title, body) = if first.len() <= 60 && message.lines().count() > 1 {
                        (
                            first.trim_end_matches(':').to_owned(),
                            lines.collect::<Vec<_>>().join("\n"),
                        )
                    } else {
                        ("Details".to_owned(), message.clone())
                    };
                    self.show_result(title.into(), body.into(), cx);
                } else {
                    self.toast(kind, None, message, cx);
                }
            }
            UiEffect::OsNotify { title, body } => {
                // Native notification when the window is not in front (or no integration);
                // an in-window toast otherwise.
                let posted = !window.is_window_active()
                    && crate::platform::OsNotifier::post(cx, title.clone(), body.clone());
                if !posted {
                    self.toast(ToastKind::Info, Some(title.into()), body, cx);
                }
            }
            UiEffect::OpenUrl(url) => cx.open_url(&url),
            UiEffect::ClipboardWrite(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            UiEffect::SetTheme(name) => {
                let pref = match name.to_lowercase().as_str() {
                    n if n.contains("light") => Some(ThemePref::Light),
                    n if n.contains("dark") => Some(ThemePref::Dark),
                    "system" => Some(ThemePref::System),
                    _ => None,
                };
                if let Some(pref) = pref {
                    self.update_settings(|s| s.theme = pref, window, cx);
                }
            }
            UiEffect::OpenSettings { namespace } => {
                self.open_settings(namespace.as_deref(), window, cx);
            }
            _ => {}
        }
    }

    /// Applies a settings change everywhere (theme, sessions, persistence).
    pub(crate) fn update_settings(
        &mut self,
        f: impl FnOnce(&mut AppSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        f(&mut self.settings);
        self.host.save_app_settings(&self.settings);
        theme::install(&self.settings, Some(window), cx);
        self.sync_mascot(cx);
        for pane in &self.panes {
            let settings = self.settings.clone();
            pane.entity.update(cx, |p, cx| p.set_settings(settings, cx));
        }
        cx.notify();
    }

    pub(crate) fn with_pane(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut SessionPane, &mut Context<SessionPane>),
    ) {
        if let Some(pane) = self.active_pane().cloned() {
            pane.update(cx, f);
        }
    }

    // ── keys ────────────────────────────────────────────────────────────────

    /// Extension shortcuts (GA-16, frontend-api.md §9). A keystroke matching a `setShortcuts`
    /// entry of the visible session invokes it. A key the app also binds (`ctrl+b`…) goes to
    /// the extension only while the composer has focus, with a one-time note.
    fn on_keystroke(
        &mut self,
        event: &gpui_kit::KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_some() || self.switch.is_some() || self.route != Route::Sessions {
            return;
        }
        let Some(pane) = self.active_pane().cloned() else {
            return;
        };
        let ks = &event.keystroke;
        let hit = pane
            .read(cx)
            .model
            .ui
            .shortcuts
            .iter()
            .find(|s| crate::components::keys::shortcut_matches(&s.keys, ks))
            .map(|s| (s.id.clone(), s.keys.clone(), s.description.clone()));
        let Some((id, keys, description)) = hit else {
            return;
        };
        if crate::components::keys::app_reserved(ks) {
            let typing = Focusable::focus_handle(pane.read(cx), cx).is_focused(window);
            if !typing {
                return;
            }
            if self.shortcut_notes.insert(keys.clone()) {
                let shown = crate::components::badge::platform_keys(&keys);
                self.toast(
                    ToastKind::Info,
                    None,
                    format!(
                        "{shown} runs “{description}” while you type; outside the composer it \
                         is the app shortcut"
                    ),
                    cx,
                );
            }
        }
        pane.update(cx, |p, cx| p.invoke_shortcut(&id, cx));
        cx.stop_propagation();
    }

    // ── navigation ──────────────────────────────────────────────────────────

    pub(crate) fn open_palette(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        perf::mark(cx, "palette open");
        self.switch = None;
        let items = self.palette_items(cx);
        let history = self.settings.palette_history.clone();
        let view = cx.new(|cx| PaletteView::new(items, history, query, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.palette = None;
                match event {
                    PaletteEvent::Pick(action, query) => {
                        let q = query.clone();
                        this.settings.push_palette_history(&q);
                        this.host.save_app_settings(&this.settings);
                        this.run_palette(action.clone(), window, cx);
                    }
                    PaletteEvent::Dismiss => {
                        if let Some(pane) = this.active_pane().cloned() {
                            SessionPane::focus(&pane, window, cx);
                        }
                    }
                }
                cx.notify();
            },
        );
        self.palette = Some((view, subscription));
        cx.notify();
    }

    fn run_palette(&mut self, action: PaletteAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            PaletteAction::App(id) => self.run_app_action(&id, window, cx),
            PaletteAction::Slash(name) => {
                self.with_pane(cx, |p, cx| p.run_prompt(&format!("/{name}"), cx));
            }
            PaletteAction::Shortcut(id) => self.with_pane(cx, |p, cx| p.invoke_shortcut(&id, cx)),
            PaletteAction::Session(id) => self.open_session(&id, window, cx),
            PaletteAction::File(path) => self.with_pane(cx, |p, cx| {
                let text = format!("@{path}");
                p.pending_tokens.push((text.clone(), text, path.clone()));
                cx.notify();
            }),
        }
        if self.route == Route::Sessions
            && let Some(pane) = self.active_pane().cloned()
        {
            SessionPane::focus(&pane, window, cx);
        }
    }

    pub(crate) fn run_app_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "new-session" => self.new_session(window, cx),
            "open-folder" => self.open_folder(window, cx),
            "switch" => self.open_switch(window, cx),
            "abort" => self.with_pane(cx, SessionPane::abort),
            "compact" => self.with_pane(cx, SessionPane::compact),
            "fork" => self.with_pane(cx, SessionPane::fork_session),
            "tree" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Tree, cx)),
            "review" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Review, cx)),
            "files" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Files, cx)),
            "model" => self.with_pane(cx, |p, cx| p.toggle_menu(PaneMenu::Model, cx)),
            // Ctrl+B: fold the list column first, then the sidebar; open both again.
            "toggle-sidebar" => {
                if self.route == Route::Sessions && self.list_open && self.sidebar_open {
                    self.list_open = false;
                } else if self.sidebar_open {
                    self.sidebar_open = false;
                } else {
                    self.sidebar_open = true;
                    self.list_open = true;
                }
                cx.notify();
            }
            "toggle-side" => {
                self.side_open = !self.side_open;
                cx.notify();
            }
            "toggle-bottom" => {
                self.bottom_open = !self.bottom_open;
                cx.notify();
            }
            "sessions" => {
                self.route = Route::Sessions;
                self.settings_view = None;
                self.detail_open |= self.focused.is_some();
                if let Some(p) = self.active_pane().cloned() {
                    SessionPane::focus(&p, window, cx);
                }
                cx.notify();
            }
            "board" => self.open_board(window, cx),
            "cron" => self.open_cron(window, cx),
            "extensions" => {
                self.route = Route::Extensions;
                self.settings_view = None;
                self.load_extensions(cx);
                cx.notify();
            }
            "theme-light" => self.update_settings(|s| s.theme = ThemePref::Light, window, cx),
            "theme-dark" => self.update_settings(|s| s.theme = ThemePref::Dark, window, cx),
            "theme-system" => self.update_settings(|s| s.theme = ThemePref::System, window, cx),
            "settings" => self.open_settings(None, window, cx),
            "shortcuts" => self.open_settings(Some("shortcuts"), window, cx),
            "refresh" => self.refresh_sessions(cx),
            "check-updates" => self.with_pane(cx, |p, cx| p.run_prompt("/update-check", cx)),
            "frame-overlay" => {
                perf::toggle_overlay(cx);
                cx.notify();
            }
            _ => {}
        }
    }

    pub(crate) fn open_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.switch = None;
        self.needs_drawer = false;
        if let Some(key) = self
            .panes
            .iter()
            .find(|p| p.entity.read(cx).model.id == id)
            .map(|p| p.key)
        {
            if self.route != Route::Onboarding {
                self.route = Route::Sessions;
                self.settings_view = None;
            }
            self.activate(key, window, cx);
            return;
        }
        if let Some(summary) = self.sessions.get(id).cloned()
            && let Some(path) = summary.path
        {
            self.open(
                OpenRequest::Resume {
                    path,
                    cwd: PathBuf::from(summary.cwd),
                },
                window,
                cx,
            );
        }
    }

    pub(crate) fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.active_pane().map_or_else(
            || self.default_cwd(),
            |p| PathBuf::from(p.read(cx).model.cwd.clone()),
        );
        self.open(OpenRequest::New { cwd }, window, cx);
    }

    pub(crate) fn new_session_in(
        &mut self,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(
            OpenRequest::New {
                cwd: PathBuf::from(cwd),
            },
            window,
            cx,
        );
    }

    pub(crate) fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Start session here".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(cwd) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |ws, window, cx| {
                    ws.open(OpenRequest::New { cwd }, window, cx);
                });
            }
        })
        .detach();
    }

    pub(crate) fn load_extensions(&self, cx: &mut Context<Self>) {
        let task = Tokio::spawn(cx, self.host.extensions());
        cx.spawn(async move |this, cx| {
            if let Ok(list) = task.await {
                let _ = this.update(cx, |ws, cx| {
                    ws.extensions = list;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn open_settings(
        &mut self,
        namespace: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let section = match namespace {
            Some("models") => SettingsSection::Models,
            Some("shortcuts") => SettingsSection::Shortcuts,
            Some("appearance") => SettingsSection::Appearance,
            Some("sessions") => SettingsSection::Sessions,
            Some("permissions") => SettingsSection::Permissions,
            Some("about") => SettingsSection::About,
            Some("unicrab") => SettingsSection::Unicrab,
            Some("general") | None => SettingsSection::General,
            Some(ns) => SettingsSection::Extension(ns.to_owned()),
        };
        let backend = self.active_pane().map(|p| Arc::clone(p.read(cx).backend()));
        let models = self
            .active_pane()
            .map(|p| p.read(cx).models.clone())
            .unwrap_or_default();
        let shortcuts = self
            .active_pane()
            .map(|p| p.read(cx).model.ui.shortcuts.clone())
            .unwrap_or_default();
        let settings = self.settings.clone();
        let info = self.host.info();
        let extensions = self.extensions.clone();
        let view = cx.new(|cx| {
            SettingsView::new(
                settings, backend, models, shortcuts, extensions, info, section, window, cx,
            )
        });
        let page_focus = Focusable::focus_handle(view.read(cx), cx);
        window.focus(&page_focus, cx);
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Back => {
                    this.run_app_action("sessions", window, cx);
                }
                SettingsEvent::Changed(settings) => {
                    let settings = settings.clone();
                    this.update_settings(|s| *s = settings, window, cx);
                }
                SettingsEvent::Command(command) => {
                    let command = command.clone();
                    this.with_pane(cx, |p, cx| p.run_prompt(&command, cx));
                }
                SettingsEvent::Toast(text) => this.toast(ToastKind::Info, None, text.clone(), cx),
                SettingsEvent::OpenExtensions => {
                    this.settings_view = None;
                    this.run_app_action("extensions", window, cx);
                }
            },
        );
        self.settings_view = Some((view, sub));
        self.route = Route::Settings;
        cx.notify();
    }

    pub(crate) fn open_board(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has = self
            .active_pane()
            .is_some_and(|p| p.read(cx).model.ui.panel(kanban::BOARD_PANEL).is_some());
        if !has {
            self.ensure_board(cx);
        }
        self.settings_view = None;
        self.route = Route::Board;
        // QA-1 repro 4: the board page takes the keyboard when it is shown (the composer
        // handle of the sessions page is not in this tree, so without this every key was
        // dead on the board).
        let focus = self
            .board_ui
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// An app-level overlay (result card, confirm, readme) closed (QA-1): return focus to
    /// the visible route's surface instead of leaving it on the removed dialog.
    pub(crate) fn refocus_after_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some()
            || self.switch.is_some()
            || self.confirm.is_some()
            || self.result_card.is_some()
            || self.ext_readme.is_some()
        {
            return; // another overlay stays on top and holds focus
        }
        if self.route == Route::Sessions
            && let Some(pane) = self.active_pane().cloned()
        {
            pane.update(cx, |p, cx| p.restore_focus(window, cx));
        } else {
            self.repair_focus(window, cx);
        }
        cx.notify();
    }

    /// QA-1 safety net: when no live element of this window holds focus (an overlay or page
    /// closed without restoring it), keystrokes and actions land nowhere. Every render,
    /// move focus back to the visible route's surface. While a surface owns focus it is
    /// contained in the rendered frame, so this is a no-op in the healthy case.
    fn repair_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() || self.switch.is_some() {
            return; // their input is in-tree and holds focus
        }
        if self.focus.contains_focused(window, cx) {
            return;
        }
        match self.route {
            Route::Sessions => {
                if let Some(pane) = self.active_pane().cloned() {
                    pane.update(cx, |p, cx| p.restore_focus(window, cx));
                } else {
                    window.focus(&self.sidebar_focus, cx);
                }
            }
            Route::Board => {
                let focus = self
                    .board_ui
                    .focus
                    .get_or_insert_with(|| cx.focus_handle())
                    .clone();
                window.focus(&focus, cx);
            }
            Route::Settings => {
                if let Some((view, _)) = &self.settings_view {
                    let handle = Focusable::focus_handle(view.read(cx), cx);
                    window.focus(&handle, cx);
                } else {
                    window.focus(&self.focus, cx);
                }
            }
            Route::Cron => {
                let focus = self
                    .cron_ui
                    .focus
                    .get_or_insert_with(|| cx.focus_handle())
                    .clone();
                window.focus(&focus, cx);
            }
            Route::Extensions | Route::Onboarding => {
                window.focus(&self.focus, cx);
            }
        }
    }

    // ── rendering ───────────────────────────────────────────────────────────

    /// Whether the list column fits next to the detail (A2.8: the list folds away when
    /// list + chat + side pane do not fit — the chat keeps `CONVERSATION_MIN`, so a
    /// `sidePane` panel is never silently hidden by the list column; QA-5).
    pub(crate) fn list_fits(&self, window: &Window) -> bool {
        let width = f32::from(window.viewport_size().width);
        let sidebar = if self.sidebar_open && width >= breakpoint::NARROW {
            size::SIDEBAR
        } else {
            0.0
        };
        let side = if self.side_open {
            f32::from(self.side_width)
        } else {
            0.0
        };
        width - sidebar - side - f32::from(self.list_width) >= size::CONVERSATION_MIN
    }

    /// A flush splitter: an 8 px hit area with a 1 px `line_soft` rule (2 px `resize_handle`
    /// while dragged or hovered).
    fn splitter(
        &self,
        t: &OinoTheme,
        drag: Drag,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.drag == Some(drag);
        let line = if active {
            t.resize_handle()
        } else {
            t.line_soft()
        };
        let rule = div()
            .absolute()
            .when(vertical, |el| {
                el.top_0()
                    .bottom_0()
                    .left(px((size::SPLITTER - size::HAIRLINE) / 2.0))
                    .w(px(if active { space::HALF } else { size::HAIRLINE }))
            })
            .when(!vertical, |el| {
                el.left_0()
                    .right_0()
                    .top(px((size::SPLITTER - size::HAIRLINE) / 2.0))
                    .h(px(if active { space::HALF } else { size::HAIRLINE }))
            })
            .bg(line);
        div()
            .id(SharedString::from(format!("splitter-{drag:?}")))
            .relative()
            .flex_shrink_0()
            .when(vertical, |el| {
                el.w(px(size::SPLITTER))
                    .mx(-px((size::SPLITTER - size::HAIRLINE) / 2.0))
                    .h_full()
                    .cursor_col_resize()
            })
            .when(!vertical, |el| {
                el.h(px(size::SPLITTER))
                    .my(-px((size::SPLITTER - size::HAIRLINE) / 2.0))
                    .w_full()
                    .cursor_row_resize()
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.drag = Some(drag);
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                // Double-click resets the list column width.
                if drag == Drag::List && e.click_count() == 2 {
                    this.list_width = px(size::LIST);
                    this.settings.list_width = 400;
                    this.host.save_app_settings(&this.settings);
                    let _ = window;
                    cx.notify();
                }
            }))
            .child(rule)
            .into_any_element()
    }

    fn on_drag_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };
        let vp = window.viewport_size();
        match drag {
            Drag::List => {
                let left = if self.sidebar_open {
                    px(size::SIDEBAR)
                } else {
                    px(space::S0)
                };
                self.list_width =
                    (e.position.x - left).clamp(px(size::LIST_MIN), px(size::LIST_MAX));
            }
            Drag::Side => {
                let w = vp.width - e.position.x;
                self.side_width = w.clamp(px(size::SIDE_PANE_MIN), vp.width * 0.6);
            }
            Drag::Bottom => {
                let h = vp.height - e.position.y;
                self.bottom_height = h.clamp(px(size::CONTROL_LG * 3.75), vp.height * 0.6);
            }
        }
        cx.notify();
    }

    /// The detail placeholder (A2.8 / A2.14): `message-square` tile 52, "No session open".
    fn render_placeholder(&self, t: &OinoTheme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .justify_center()
            .gap(px(space::S2_5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(size::PLACEHOLDER_TILE))
                    .rounded(px(radius::R_FLOAT))
                    .bg(t.chip())
                    .child(crate::design::icons::icon(
                        gpui_kit::assets::IconName::MessageSquare,
                        size::ICON_LG,
                        t.fg_subtle(),
                    )),
            )
            .child(
                div()
                    .text_role(t, TextRole::Lg)
                    .weight_semibold()
                    .text_color(t.fg_subtle())
                    .child(if self.opening {
                        "Starting session…"
                    } else {
                        "No session open"
                    }),
            )
            .child(
                div()
                    .max_w(px(size::EMPTY_BODY - space::S5))
                    .text_center()
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtlest())
                    .child(
                        "Pick a session on the left. It keeps running while you look at the \
                         others.",
                    ),
            )
            .into_any_element()
    }

    /// The Sessions screen: list column | detail (session + side pane, bottom frame).
    #[allow(clippy::too_many_lines, reason = "the column layout")]
    fn render_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let width = window.viewport_size().width;
        let narrow = width < px(breakpoint::NARROW);
        let csd = crate::components::window_chrome::client_decorated(window);
        let maximized = window.is_maximized();
        let list_visible = self.list_open && self.list_fits(window);
        let used = if self.sidebar_open && !narrow {
            px(size::SIDEBAR)
        } else {
            px(space::S0)
        } + if list_visible {
            self.list_width
        } else {
            px(space::S0)
        };
        let side_fits = width - self.side_width - used >= px(size::CONVERSATION_MIN);
        let side_visible = self.side_open && side_fits && !narrow && self.detail_open;
        let pane = self.active_pane().cloned().filter(|_| self.detail_open);
        let needs = self.sessions.needs_you().len();
        let detail: AnyElement = match pane.clone() {
            Some(pane) => {
                let state = HeaderState {
                    sidebar_collapsed: !self.sidebar_open || narrow,
                    side_open: self.side_open,
                    bottom_open: self.bottom_open,
                    primary: true,
                    list_folded: !list_visible,
                    needs,
                    controls: csd && !side_visible,
                    controls_side: csd && side_visible,
                    maximized,
                    drag: csd,
                };
                pane.update(cx, |p, _| p.header_state = state);
                // Cached: the session re-renders only when it was notified (streaming in a
                // background session never rebuilds the visible one, design.md §2.11).
                // The automation driver uncaches it: a cached view only re-renders when it is
                // notified, so the element registry would miss a stable card
                // (docs/dev/v1/gui-driver.md §1).
                let detail: AnyElement = if crate::automation::registry::enabled() {
                    pane.clone().into_any_element()
                } else {
                    pane.clone()
                        .cached(gpui_kit::StyleRefinement::default().size_full())
                        .into_any_element()
                };
                div()
                    .id("detail")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(detail)
                    .into_any_element()
            }
            None => self.render_placeholder(&t),
        };
        let side = side_visible.then(|| pane.clone()).flatten().map(|pane| {
            pane.update(cx, |p, cx| {
                if !p.side.is_open() {
                    p.side.open(SideTab::Review);
                }
                p.render_side(window, cx)
            })
        });
        let bottom = (self.bottom_open && self.detail_open)
            .then(|| pane.clone())
            .flatten()
            .map(|pane| pane.update(cx, |p, cx| p.render_bottom(window, cx)));
        let center = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(div().flex().flex_1().min_h_0().child(detail))
            .when_some(bottom, |el, b| {
                el.child(self.splitter(&t, Drag::Bottom, false, cx)).child(
                    div()
                        .flex()
                        .flex_col()
                        .h(self.bottom_height)
                        .flex_shrink_0()
                        .bg(t.card())
                        .child(b),
                )
            });
        self.side_shown = side.is_some();
        let list = list_visible.then(|| self.render_list(window, cx));
        div()
            .flex()
            .flex_row()
            .size_full()
            .when_some(list, |el, l| {
                el.child(
                    div()
                        .flex()
                        .flex_col()
                        .w(self.list_width)
                        .flex_shrink_0()
                        .h_full()
                        .child(l),
                )
                .child(self.splitter(&t, Drag::List, true, cx))
            })
            .child(center)
            .when_some(side, |el, s| {
                el.child(self.splitter(&t, Drag::Side, true, cx)).child(
                    div()
                        .flex()
                        .flex_col()
                        .w(self.side_width)
                        .flex_shrink_0()
                        .h_full()
                        .child(s),
                )
            })
            .into_any_element()
    }

    /// The result card dialog (GA-24). It owns the keyboard while open and returns focus on
    /// close (QA-1 repro 3: the card used to need `Tab Tab Return` before Esc acted).
    fn render_result(
        &self,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (title, body) = self.result_card.clone()?;
        let e = cx.entity().downgrade();
        let restore: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |w, cx| {
            let _ = e.update(cx, |ws, cx| {
                ws.result_card = None;
                ws.refocus_after_overlay(w, cx);
            });
        });
        let focus = crate::components::dialog::owned_focus("result-card", window, cx);
        let copy = body.clone();
        let done = Rc::clone(&restore);
        Some(crate::components::dialog::modal_owned(
            t,
            "result-card",
            title,
            size::DIALOG_MD,
            div()
                .flex()
                .flex_col()
                .gap(px(space::S1))
                .children(body.lines().map(|line| {
                    div()
                        .text_role(t, TextRole::Base)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(line.to_owned()))
                }))
                .into_any_element(),
            vec![
                crate::components::button::Btn::new("result-copy")
                    .label("Copy")
                    .icon(gpui_kit::assets::IconName::Copy)
                    .variant(crate::components::button::Variant::Secondary)
                    .size(crate::components::button::BtnSize::Xl)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.to_string()));
                    })
                    .into_any_element(),
                crate::components::button::Btn::new("result-ok")
                    .label("Done")
                    .variant(crate::components::button::Variant::Primary)
                    .size(crate::components::button::BtnSize::Xl)
                    .on_click(move |_, w, cx| done(w, cx))
                    .into_any_element(),
            ],
            &focus,
            restore,
        ))
    }
}

impl Focusable for AppShell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AppShell {
    #[allow(clippy::too_many_lines, reason = "root layout and action wiring")]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _activity = crate::stability::activity("shell render");
        perf::begin(cx);
        // Publishes the focus handles of the frame that just finished (no-op while
        // automation is off).
        crate::automation::begin_frame(cx);
        self.repair_focus(window, cx);
        let t = OinoTheme::get(cx);
        self.window_width = window.viewport_size().width;
        let narrow = self.window_width < px(breakpoint::NARROW);
        let sidebar = (self.sidebar_open && !narrow && self.route != Route::Onboarding)
            .then(|| self.render_sidebar(window, cx));
        let main: AnyElement = match self.route {
            Route::Sessions => self.render_sessions(window, cx),
            Route::Settings => match &self.settings_view {
                Some((view, _)) => div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(view.clone())
                    .into_any_element(),
                None => self.render_sessions(window, cx),
            },
            Route::Board => self.render_board(window, cx),
            Route::Cron => self.render_cron(window, cx),
            Route::Extensions => self.render_extensions(window, cx),
            Route::Onboarding => self.render_onboarding(window, cx),
        };
        let modal = self
            .active_pane()
            .cloned()
            .filter(|_| self.route == Route::Sessions)
            .and_then(|p| p.update(cx, |p, cx| p.render_modal(window, cx)));
        let confirm = self
            .render_confirm(&t, window, cx)
            .or_else(|| self.render_result(&t, window, cx))
            .or_else(|| self.render_readme(&t, window, cx));
        let switch = self.render_switch(&t, cx);
        // Native browser views cover GPUI content in their bounds: show only the visible
        // session's, and only while nothing is drawn over the side pane.
        let covered = self.palette.is_some()
            || self.switch.is_some()
            || self.confirm.is_some()
            || self.result_card.is_some()
            || self.ext_readme.is_some()
            || self.needs_drawer
            || modal.is_some()
            || self.route != Route::Sessions
            || !self.side_open
            || !self.side_shown;
        let active = self.focused;
        let panes: Vec<(u64, Entity<SessionPane>)> = self
            .panes
            .iter()
            .map(|p| (p.key, p.entity.clone()))
            .collect();
        for (key, pane) in panes {
            pane.update(cx, |p, cx| {
                p.sync_browser(!covered && Some(key) == active, cx);
            });
        }
        let on_dismiss: Rc<dyn Fn(u64, &mut Window, &mut App)> = {
            let e = cx.entity().downgrade();
            Rc::new(move |id, _, cx| {
                let _ = e.update(cx, |ws, cx| {
                    ws.toasts.dismiss(id);
                    cx.notify();
                });
            })
        };
        let on_hover: Rc<dyn Fn(Option<u64>, &mut Window, &mut App)> = {
            let e = cx.entity().downgrade();
            Rc::new(move |id, _, cx| {
                let _ = e.update(cx, |ws, _| ws.toasts.hovered = id);
            })
        };
        let on_more: Rc<dyn Fn(u64, &mut Window, &mut App)> = {
            let e = cx.entity().downgrade();
            Rc::new(move |id, _, cx| {
                let _ = e.update(cx, |ws, cx| {
                    if let Some(toast) = ws.toasts.get(id).cloned() {
                        ws.toasts.dismiss(id);
                        let title = toast.title.unwrap_or_else(|| "Details".into());
                        ws.show_result(title, toast.message, cx);
                    }
                });
            })
        };
        let toasts = stack(&t, &self.toasts, on_dismiss, on_hover, on_more);
        let csd = crate::components::window_chrome::client_decorated(window);
        let floating = csd && crate::components::window_chrome::floating(window);
        // Window controls on pages and the placeholder (a session's detail header or side
        // pane carries them otherwise).
        let controls_overlay = csd
            && match self.route {
                Route::Sessions => !(self.detail_open && self.focused.is_some()),
                Route::Settings => self.settings_view.is_none(),
                Route::Extensions => false,
                Route::Cron => self.cron_ui.edit.is_none(),
                Route::Board | Route::Onboarding => true,
            };
        let columns = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_row()
            .children(sidebar.map(|s| {
                div()
                    .flex()
                    .flex_row()
                    .h_full()
                    .flex_shrink_0()
                    .child(div().w(px(size::SIDEBAR)).h_full().bg(t.sidebar()).child(s))
                    .child(div().w(px(size::HAIRLINE)).h_full().bg(t.line_soft()))
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(main),
            );
        // The client inset is owned by gpui-kit's `Root` (`window_border`), which sets it to
        // its shadow size on every frame and draws the resize band. Setting it here as well made
        // the two fight each frame: the X11 frame extents flipped between 0 and the shadow size,
        // mutter resized the window by 2x the shadow each time, and the window flashed.
        let edges = if floating && window.client_inset().is_none() {
            crate::components::window_chrome::resize_edges(size::WINDOW_INSET)
        } else {
            Vec::new()
        };
        div()
            .id("root")
            .relative()
            .size_full()
            .when(floating && cfg!(target_os = "linux"), |el| {
                el.rounded(px(radius::R_CARD)).overflow_hidden()
            })
            .bg(t.window())
            .text_color(t.fg())
            .font_family(t.ui_font())
            .text_role(&t, TextRole::Base)
            .track_focus(&self.focus)
            .on_mouse_move(cx.listener(Self::on_drag_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if let Some(drag) = this.drag.take() {
                        if drag == Drag::List {
                            let w = f32::from(this.list_width);
                            #[allow(
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss,
                                reason = "clamped to 280–560"
                            )]
                            let w = w.round() as u16;
                            this.settings.list_width = w;
                            this.host.save_app_settings(&this.settings);
                        }
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &NewSession, window, cx| this.new_session(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
            .on_action(cx.listener(|this, _: &SwitchSession, window, cx| {
                this.open_switch(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoSessions, window, cx| {
                this.run_app_action("sessions", window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoCron, window, cx| {
                this.open_cron(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoBoard, window, cx| {
                this.open_board(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoExtensions, window, cx| {
                this.run_app_action("extensions", window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.run_app_action("toggle-sidebar", window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleSidePane, window, cx| {
                this.run_app_action("toggle-side", window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleBottom, window, cx| {
                this.run_app_action("toggle-bottom", window, cx);
            }))
            .on_action(cx.listener(|this, _: &Abort, _, cx| this.with_pane(cx, SessionPane::abort)))
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_palette("", window, cx);
            }))
            .on_action(cx.listener(|this, _: &PickModel, window, cx| {
                // GA-34: from any focus in the session, not only the composer chip.
                if this.route != Route::Sessions {
                    this.run_app_action("sessions", window, cx);
                }
                this.with_pane(cx, |p, cx| p.toggle_menu(PaneMenu::Model, cx));
            }))
            .on_action(cx.listener(|this, _: &PickThinking, _, cx| {
                this.with_pane(cx, |p, cx| p.toggle_menu(PaneMenu::Thinking, cx));
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_settings(None, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                let next = if OinoTheme::get(cx).dark {
                    ThemePref::Light
                } else {
                    ThemePref::Dark
                };
                this.update_settings(|s| s.theme = next, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TreeView, window, cx| {
                this.run_app_action("tree", window, cx);
            }))
            .on_action(cx.listener(|this, _: &FindInConversation, window, cx| {
                if let Some(p) = this.active_pane().cloned() {
                    p.update(cx, |p, cx| p.open_find(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CloseFind, _, cx| {
                this.with_pane(cx, SessionPane::close_find);
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| {
                if let Some(p) = this.active_pane().cloned() {
                    SessionPane::focus(&p, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleStatusPanel, _, cx| {
                this.with_pane(cx, |p, cx| {
                    let now = p.status_expanded_now();
                    p.status_expanded = Some(!now);
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &ToggleFrameOverlay, window, cx| {
                this.run_app_action("frame-overlay", window, cx);
            }))
            // The unipi desktop glow: a 520 px top-left shell tint (A2.2 `glow`).
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(size::GLOW))
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(t.glow(), 0.),
                        linear_color_stop(t.glow().opacity(0.), 1.),
                    )),
            )
            .child(columns)
            .when(controls_overlay, |el| {
                el.child(
                    div()
                        .absolute()
                        .top(px(space::S2))
                        .right(px(space::S2))
                        .child(crate::components::window_chrome::window_controls(
                            &t,
                            window.is_maximized(),
                        )),
                )
            })
            .children(self.render_needs_drawer(&t, cx))
            .children(modal)
            .children(confirm)
            .children(switch)
            .children(edges)
            .when_some(self.palette.as_ref().map(|(p, _)| p.clone()), |el, p| {
                el.child(p)
            })
            .child(toasts)
            .children(perf::overlay(cx))
            .child(perf::end_marker())
            // Publishes the automation element tree once the frame is fully prepainted.
            .child(crate::automation::end_marker())
    }
}

#[cfg(test)]
mod tests {
    use super::sessions_to_release;

    #[test]
    fn releases_hidden_idle_sessions_least_recently_used_first() {
        // (key, last_used, visible, idle)
        let sessions = [
            (0, 5, false, true),
            (1, 1, false, true),
            (2, 0, true, true),
            (3, 2, false, false),
            (4, 3, false, true),
        ];
        assert_eq!(sessions_to_release(&sessions, 3), vec![1, 4]);
        assert_eq!(sessions_to_release(&sessions, 5), Vec::<u64>::new());
        // Visible or running sessions are never released, even over the cap.
        assert_eq!(sessions_to_release(&sessions, 0), vec![1, 4, 0]);
    }
}
