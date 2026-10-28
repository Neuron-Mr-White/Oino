//! [`MobileShell`]: the phone window root (design.md §6).
//!
//! State: a page stack (root = M1 Sessions), at most one bottom sheet, the right drawer flag
//! and the toasts. Open sessions are [`SessionPane`]s that keep running while another page is
//! shown, as on desktop. Back pops menu → sheet → drawer → page.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, BackGestureEvent, BackGesturePhase, ClickEvent,
    ClipboardItem, Context, ElementId, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    px,
};

use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::interaction::InteractionView;
use crate::components::toast::{ToastKind, Toasts};
use crate::design::brand::CrabState;
use crate::design::icons::icon;
use crate::design::theme::{self, OinoTheme, TextExt as _};
use crate::design::tokens::touch;
use crate::design::{TextRole, radius, size, space};
use crate::host::{AppSettings, ExtensionInfo, OpenRequest, OpenedSession, SessionHost, ThemePref};
use crate::mobile::gesture::Pull;
use crate::mobile::{Launch, MobilePlatform, RemoteHost};
use crate::model::palette::Scope;
use crate::model::sessions::{LiveStatus, SessionList, SessionSummary};
use crate::model::ui_state::{NotifyKind, PanelPlacement, UiEffect};
use crate::rt::Tokio;
use crate::views::session_pane::{PaneEvent, SessionPane, SideTab};

/// A settings page (M8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    /// The list of pages.
    Root,
    /// Theme, text size, motion.
    Appearance,
    /// Model and thinking defaults, providers.
    Models,
    /// Conversation display.
    Sessions,
    /// Workflow permissions.
    Permissions,
    /// Extensions list.
    Extensions,
    /// Extension settings hub (namespace list).
    Hub,
    /// One extension settings namespace.
    Namespace(String),
    /// Remote access (M10 hosts).
    Remote,
    /// About.
    About,
}

/// A page on the navigation stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// M1 home.
    Sessions,
    /// M2 conversation of pane `key`.
    Session(u64),
    /// M8.
    Settings(SettingsPage),
    /// M7.
    Search,
    /// M9 board of pane `key`.
    Board(u64),
    /// M10 pairing.
    Pair,
    /// M5 diff page.
    Diff {
        /// Title (path).
        title: String,
        /// Unified patch.
        patch: String,
    },
    /// M5 tool output page.
    Output {
        /// Title (command).
        title: String,
        /// Output text.
        text: String,
    },
}

/// A confirmation sheet.
#[derive(Clone)]
pub(crate) struct Confirm {
    /// Title.
    pub(crate) title: SharedString,
    /// Body.
    pub(crate) body: SharedString,
    /// Destructive action label.
    pub(crate) action: SharedString,
    /// Runs on confirm.
    pub(crate) run: Rc<dyn Fn(&mut MobileShell, &mut Window, &mut Context<MobileShell>)>,
}

/// A bottom sheet (M6).
#[derive(Clone)]
pub(crate) enum Sheet {
    /// Model rows + thinking segments.
    Model,
    /// Session actions.
    Actions,
    /// Expanded status panel content.
    Status,
    /// Rename a session.
    Rename(String),
    /// Confirmation.
    Confirm(Confirm),
    /// Long-press menu of a session row.
    RowMenu(String),
    /// Long-press menu of a board card (task id).
    CardMenu(String),
    /// Session list display options.
    Display,
    /// Edit one extension setting.
    Edit {
        /// Namespace.
        ns: String,
        /// Field key.
        key: String,
        /// Field label.
        label: String,
    },
}

pub(crate) struct Pane {
    pub(crate) key: u64,
    pub(crate) entity: Entity<SessionPane>,
    _subscriptions: Vec<Subscription>,
}

/// The phone window root view.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent view flags (drawer, opening, refreshing, window active)"
)]
pub struct MobileShell {
    pub(crate) host: Arc<dyn SessionHost>,
    pub(crate) settings: AppSettings,
    pub(crate) sessions: SessionList,
    pub(crate) panes: Vec<Pane>,
    next_key: u64,
    pub(crate) stack: Vec<Page>,
    pub(crate) sheet: Option<Sheet>,
    pub(crate) drawer: bool,
    pub(crate) extensions: Vec<ExtensionInfo>,
    pub(crate) opening: bool,
    pub(crate) refreshing: bool,
    focus: FocusHandle,
    pub(crate) toasts: Toasts,
    toast_timer: Option<Task<()>>,
    pub(crate) search: Entity<InputState>,
    pub(crate) scope: Scope,
    pub(crate) rename: Option<Entity<InputState>>,
    pub(crate) pair: Entity<InputState>,
    pub(crate) hosts: Vec<RemoteHost>,
    pub(crate) swipes: HashMap<String, ScrollHandle>,
    pub(crate) pull: Pull,
    pub(crate) list_scroll: ScrollHandle,
    pub(crate) page_scroll: ScrollHandle,
    /// Long presses are ignored while a sheet covers the page.
    pub(crate) press: Rc<Cell<bool>>,
    pub(crate) board_lane: usize,
    pub(crate) hub: crate::mobile::settings::Hub,
    pub(crate) board_scroll: ScrollHandle,
    active: bool,
    /// Last touch-down: programmatic focus (a run settling, a card closing) must not raise
    /// the soft keyboard over the conversation — only a tap on a field does.
    last_tap: Rc<Cell<Option<Instant>>>,
    badge: usize,
    notified: HashSet<String>,
    launch: Option<Launch>,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

fn keyboard_hooks(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<MobileShell>,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        |_, _, event: &InputEvent, _, cx| match event {
            InputEvent::Focus => (MobilePlatform::get(cx).show_keyboard)(),
            InputEvent::Blur => (MobilePlatform::get(cx).hide_keyboard)(),
            _ => {}
        },
    )
}

impl MobileShell {
    /// The root view; starts on the session list (or where `launch` points).
    pub fn new(
        host: Arc<dyn SessionHost>,
        launch: Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut settings = host.app_settings();
        // Phones read at 15 (design.md §6): the desktop default 14 becomes 15 here.
        if f32::from(settings.ui_font_size) <= crate::design::type_scale::UI_FONT_SIZE {
            settings.ui_font_size = 15;
        }
        Self::install_theme(&settings, window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search Oino"));
        let pair = cx.new(|cx| InputState::new(window, cx).placeholder("oino://host:port?token=…"));
        let mut subscriptions = vec![
            keyboard_hooks(&search, window, cx),
            keyboard_hooks(&pair, window, cx),
            cx.subscribe_in(&search, window, |_, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.settings.theme == ThemePref::System {
                    Self::install_theme(&this.settings, window, cx);
                    cx.notify();
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.active = window.is_window_active();
                if this.active {
                    this.poll_link(window, cx);
                }
            }),
        ];
        subscriptions.shrink_to_fit();
        let hosts = (MobilePlatform::get(cx).load_hosts)();
        let shell = Self {
            host,
            settings,
            sessions: SessionList::new(),
            panes: Vec::new(),
            next_key: 0,
            stack: vec![Page::Sessions],
            sheet: None,
            drawer: false,
            extensions: Vec::new(),
            opening: false,
            refreshing: false,
            focus: cx.focus_handle(),
            toasts: Toasts::default(),
            toast_timer: None,
            search,
            scope: Scope::All,
            rename: None,
            pair,
            hosts,
            swipes: HashMap::new(),
            pull: Pull::default(),
            list_scroll: ScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            press: Rc::new(Cell::new(true)),
            board_lane: 0,
            hub: crate::mobile::settings::Hub::default(),
            board_scroll: ScrollHandle::new(),
            active: true,
            last_tap: Rc::new(Cell::new(None)),
            badge: 0,
            notified: HashSet::new(),
            launch: Some(launch),
            _subscriptions: subscriptions,
        };
        shell.refresh_sessions(cx);
        shell
    }

    fn install_theme(settings: &AppSettings, window: &mut Window, cx: &mut App) {
        theme::install(settings, Some(window), cx);
        let t = OinoTheme::get(cx);
        let [r, g, b, _] = crate::design::tokens::channels(t.p.frame);
        (MobilePlatform::get(cx).system_bars)(
            t.dark,
            (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        );
    }

    // ── panes ────────────────────────────────────────────────────────────────

    pub(crate) fn pane(&self, key: u64) -> Option<&Entity<SessionPane>> {
        self.panes.iter().find(|p| p.key == key).map(|p| &p.entity)
    }

    pub(crate) fn page(&self) -> &Page {
        self.stack.last().unwrap_or(&Page::Sessions)
    }

    /// The pane of the conversation on top (or under the page on top).
    pub(crate) fn active_key(&self) -> Option<u64> {
        self.stack.iter().rev().find_map(|p| match p {
            Page::Session(k) | Page::Board(k) => Some(*k),
            _ => None,
        })
    }

    pub(crate) fn active_pane(&self) -> Option<&Entity<SessionPane>> {
        self.active_key().and_then(|k| self.pane(k))
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

    /// The pending interaction of the visible conversation when it shows as a sheet.
    pub(crate) fn sheet_interaction(&self, cx: &App) -> Option<Entity<InteractionView>> {
        let Page::Session(key) = self.page() else {
            return None;
        };
        let pane = self.pane(*key)?.read(cx);
        pane.interaction
            .as_ref()
            .filter(|v| v.read(cx).touch_sheet())
            .cloned()
    }

    // ── sessions ─────────────────────────────────────────────────────────────

    pub(crate) fn refresh_sessions(&self, cx: &mut Context<Self>) {
        let task = Tokio::spawn(cx, self.host.list_sessions());
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |shell, cx| {
                shell.refreshing = false;
                if let Ok(Ok(list)) = result {
                    shell.sessions.set_sessions(list);
                    shell.sync_open_sessions(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn pull_refresh(&mut self, cx: &mut Context<Self>) {
        self.refreshing = true;
        (MobilePlatform::get(cx).haptic)();
        self.refresh_sessions(cx);
        cx.notify();
    }

    fn sync_open_sessions(&mut self, cx: &App) {
        for pane in &self.panes {
            let pane = pane.entity.read(cx);
            let model = &pane.model;
            let existing = self.sessions.get(&model.id).cloned();
            self.sessions.upsert(SessionSummary {
                id: model.id.clone(),
                path: model.file.as_ref().map(PathBuf::from),
                cwd: model.cwd.clone(),
                title: model.title(),
                modified_ms: existing.map_or_else(now_ms, |e| e.modified_ms),
                message_count: model.transcript.len(),
            });
            let live = pane.live();
            if live != LiveStatus::Idle || self.sessions.live(&model.id) != LiveStatus::Unread {
                self.sessions.set_live(&model.id, live);
            }
        }
    }

    fn open(&mut self, request: OpenRequest, window: &mut Window, cx: &mut Context<Self>) {
        let (OpenRequest::New { cwd } | OpenRequest::Resume { cwd, .. }) = &request;
        self.settings.touch_cwd(&cwd.display().to_string());
        self.host.save_app_settings(&self.settings);
        self.opening = true;
        cx.notify();
        let task = Tokio::spawn(cx, self.host.open(request));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |shell, window, cx| {
                shell.opening = false;
                match result {
                    Ok(Ok(opened)) => shell.add_pane(&opened, window, cx),
                    Ok(Err(e)) => shell.toast(ToastKind::Error, None, e.to_string(), cx),
                    Err(e) => shell.toast(ToastKind::Error, None, e.to_string(), cx),
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
        // The composer owns the pane's focus handle: raise / drop the soft keyboard with it.
        let focus = entity.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&entity, window, move |this, pane, event, window, cx| {
                this.on_pane_event(key, pane, event, window, cx);
            }),
            cx.observe(&entity, |this, _, cx| this.on_pane_changed(cx)),
            cx.on_focus_in(&focus, window, |this, _, cx| {
                let tapped = this
                    .last_tap
                    .get()
                    .is_some_and(|at| at.elapsed() < Duration::from_millis(700));
                if tapped {
                    (MobilePlatform::get(cx).show_keyboard)();
                }
            }),
            cx.on_focus_out(&focus, window, |_, _, _, cx| {
                (MobilePlatform::get(cx).hide_keyboard)();
            }),
        ];
        self.panes.push(Pane {
            key,
            entity,
            _subscriptions: subscriptions,
        });
        self.show_session(key, cx);
        self.sync_open_sessions(cx);
        if let Some(launch) = self.launch.take() {
            self.apply_launch_sheet(&launch, cx);
        }
    }

    pub(crate) fn show_session(&mut self, key: u64, cx: &mut Context<Self>) {
        self.stack.truncate(1);
        self.stack.push(Page::Session(key));
        self.sheet = None;
        self.drawer = false;
        if let Some(pane) = self.pane(key) {
            let id = pane.read(cx).model.id.clone();
            if self.sessions.live(&id) == LiveStatus::Unread {
                self.sessions.set_live(&id, LiveStatus::Idle);
            }
        }
        cx.notify();
    }

    pub(crate) fn open_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.key_of(id, cx) {
            self.show_session(key, cx);
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

    pub(crate) fn key_of(&self, id: &str, cx: &App) -> Option<u64> {
        self.panes
            .iter()
            .find(|p| p.entity.read(cx).model.id == id)
            .map(|p| p.key)
    }

    pub(crate) fn new_session(
        &mut self,
        cwd: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd = cwd.map_or_else(
            || {
                self.active_pane().map_or_else(
                    || self.host.info().default_cwd,
                    |p| PathBuf::from(p.read(cx).model.cwd.clone()),
                )
            },
            PathBuf::from,
        );
        self.open(OpenRequest::New { cwd }, window, cx);
    }

    /// Runs `f` on the pane of session `id`, opening it first when needed.
    pub(crate) fn with_session(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut SessionPane, &mut Context<SessionPane>) + 'static,
    ) {
        if let Some(pane) = self.key_of(id, cx).and_then(|k| self.pane(k).cloned()) {
            pane.update(cx, f);
            return;
        }
        let Some(summary) = self.sessions.get(id).cloned() else {
            return;
        };
        let Some(path) = summary.path else {
            return;
        };
        let task = Tokio::spawn(
            cx,
            self.host.open(OpenRequest::Resume {
                path,
                cwd: PathBuf::from(summary.cwd),
            }),
        );
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(opened)) = task.await {
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.add_pane(&opened, window, cx);
                    shell.with_pane(cx, f);
                });
            }
        })
        .detach();
    }

    fn on_pane_changed(&mut self, cx: &mut Context<Self>) {
        // Badge = sessions that need you (design.md §6.11).
        let needs = self
            .panes
            .iter()
            .filter(|p| p.entity.read(cx).live() == LiveStatus::Attention)
            .count();
        if needs != self.badge {
            self.badge = needs;
            (MobilePlatform::get(cx).set_badge)(needs);
        }
    }

    /// Aggregate Unicrab state of every open session.
    pub(crate) fn crab(&self, cx: &App) -> CrabState {
        CrabState::aggregate(self.panes.iter().map(|p| p.entity.read(cx).crab()))
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
                let visible = self.page() == &Page::Session(key) && self.active;
                let status = if *status == LiveStatus::Idle
                    && self.sessions.live(&id) == LiveStatus::Running
                    && !visible
                {
                    LiveStatus::Unread
                } else {
                    *status
                };
                if status == LiveStatus::Attention
                    && !self.active
                    && self.notified.insert(id.clone())
                {
                    // A pending interaction while backgrounded → local notification (§6.3).
                    let p = pane.read(cx);
                    let title = p.model.title();
                    let body = p
                        .model
                        .ui
                        .dialog()
                        .map_or_else(|| "Oino needs you".to_owned(), |d| d.title.clone());
                    (MobilePlatform::get(cx).notify)(notification_id(&id), &title, &body);
                }
                if status != LiveStatus::Attention {
                    self.notified.remove(&id);
                }
                self.sessions.set_live(&id, status);
                self.on_pane_changed(cx);
                cx.notify();
            }
            PaneEvent::Changed => {
                self.sync_open_sessions(cx);
                cx.notify();
            }
            PaneEvent::PickModel => {
                self.open_sheet(Sheet::Model, cx);
            }
            PaneEvent::Effect(effect) => self.perform(effect.clone(), window, cx),
            PaneEvent::Confirm => {
                // Destructive pane action (rewind, stop an agent / task) → confirm sheet.
                let Some(request) = pane.update(cx, |p, _| p.pending_confirm.take()) else {
                    return;
                };
                let pane = pane.clone();
                self.open_sheet(
                    Sheet::Confirm(Confirm {
                        title: SharedString::from(request.title),
                        body: SharedString::from(request.body),
                        action: SharedString::from(request.action),
                        run: Rc::new(move |_, _, cx| {
                            pane.update(cx, |p, cx| (request.run)(p, cx));
                        }),
                    }),
                    cx,
                );
            }
            PaneEvent::Toast(text) => self.toast(ToastKind::Info, None, text.clone(), cx),
            PaneEvent::ShowSide => self.on_show_side(key, pane, cx),
            PaneEvent::ToggleSide => {
                self.drawer = !self.drawer;
                cx.notify();
            }
            PaneEvent::ShowBottom
            | PaneEvent::Focused
            | PaneEvent::ToggleSidebar
            | PaneEvent::ToggleBottom
            | PaneEvent::ClosePane => cx.notify(),
        }
    }

    /// A side tab was opened: diffs and outputs become full-screen pages (M5), the kanboard
    /// becomes the board pager (M9), everything else opens the right drawer (M4).
    fn on_show_side(&mut self, key: u64, pane: &Entity<SessionPane>, cx: &mut Context<Self>) {
        let current = pane.read(cx).side.current().cloned();
        let page = match current {
            Some(SideTab::Diff { title, patch }) => Some(Page::Diff { title, patch }),
            Some(SideTab::Output { title, text }) => Some(Page::Output { title, text }),
            Some(SideTab::Panel(id)) if id == crate::model::kanban::BOARD_PANEL => {
                Some(Page::Board(key))
            }
            _ => None,
        };
        if let Some(page) = page {
            self.page_scroll = ScrollHandle::new();
            if matches!(page, Page::Output { .. }) {
                self.page_scroll.scroll_to_bottom();
            }
            pane.update(cx, |p, _| {
                let active = p.side.active;
                p.side.close_at(active);
            });
            self.sheet = None;
            self.drawer = false;
            if self.page() != &page {
                self.stack.push(page);
            }
        } else if self.page() == &Page::Session(key) {
            self.drawer = true;
        }
        cx.notify();
    }

    pub(crate) fn perform(
        &mut self,
        effect: UiEffect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match effect {
            UiEffect::Notify { message, kind } => {
                let kind = match kind {
                    NotifyKind::Info => ToastKind::Info,
                    NotifyKind::Warning => ToastKind::Warning,
                    NotifyKind::Error => ToastKind::Error,
                };
                self.toast(kind, None, message, cx);
            }
            UiEffect::OsNotify { title, body } => {
                if self.active {
                    self.toast(ToastKind::Info, Some(title.into()), body, cx);
                } else {
                    let id = notification_id(&format!("{title}\n{body}"));
                    (MobilePlatform::get(cx).notify)(id, &title, &body);
                }
            }
            UiEffect::OpenUrl(url) => self.open_url(&url, cx),
            UiEffect::ClipboardWrite(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.toast(ToastKind::Success, None, "Copied", cx);
            }
            UiEffect::SetTheme(name) => {
                let pref = if name.to_lowercase().contains("light") {
                    ThemePref::Light
                } else {
                    ThemePref::Dark
                };
                self.set_theme(pref, window, cx);
            }
            UiEffect::OpenSettings { namespace } => {
                let page = namespace.map_or(SettingsPage::Hub, SettingsPage::Namespace);
                self.open_settings(page, cx);
            }
            _ => {}
        }
    }

    /// Opens `url` in the in-app browser (design.md A1.4 mobile), else externally.
    pub(crate) fn open_url(&self, url: &str, cx: &mut Context<Self>) {
        let dark = OinoTheme::get(cx).dark;
        if !(MobilePlatform::get(cx).open_browser)(url, dark) {
            cx.open_url(url);
        }
    }

    pub(crate) fn toast(
        &mut self,
        kind: ToastKind,
        title: Option<SharedString>,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.toasts.push(kind, title, message);
        self.ensure_toast_timer(cx);
        cx.notify();
    }

    /// Starts the expiry sweep if it is not running (see the desktop shell).
    pub(crate) fn ensure_toast_timer(&mut self, cx: &mut Context<Self>) {
        if self.toast_timer.is_some() {
            return;
        }
        self.toast_timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let keep = this
                    .update(cx, |shell, cx| {
                        let now = Instant::now();
                        let changed = shell.toasts.expire(now);
                        let reaped = shell.toasts.reap(now);
                        if changed || reaped {
                            cx.notify();
                        }
                        let keep = shell.toasts.has_timed();
                        if !keep {
                            shell.toast_timer = None;
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

    // ── settings ─────────────────────────────────────────────────────────────

    pub(crate) fn open_settings(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.sheet = None;
        self.drawer = false;
        if !matches!(self.page(), Page::Settings(_)) {
            self.stack.push(Page::Settings(SettingsPage::Root));
        }
        if page != SettingsPage::Root {
            self.stack.push(Page::Settings(page));
        }
        self.load_hub(cx);
        let task = Tokio::spawn(cx, self.host.extensions());
        cx.spawn(async move |this, cx| {
            if let Ok(list) = task.await {
                let _ = this.update(cx, |shell, cx| {
                    shell.extensions = list;
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn set_theme(
        &mut self,
        pref: ThemePref,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings.theme = pref;
        Self::install_theme(&self.settings, window, cx);
        self.host.save_app_settings(&self.settings);
        cx.notify();
    }

    pub(crate) fn update_settings(
        &mut self,
        f: impl FnOnce(&mut AppSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        f(&mut self.settings);
        self.host.save_app_settings(&self.settings);
        Self::install_theme(&self.settings, window, cx);
        for pane in &self.panes {
            let settings = self.settings.clone();
            pane.entity.update(cx, |p, cx| p.set_settings(settings, cx));
        }
        cx.notify();
    }

    // ── navigation ───────────────────────────────────────────────────────────

    pub(crate) fn push(&mut self, page: Page, cx: &mut Context<Self>) {
        self.sheet = None;
        self.drawer = false;
        if page == Page::Search {
            self.scope = Scope::All;
        }
        self.stack.push(page);
        cx.notify();
    }

    pub(crate) fn open_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        if matches!(sheet, Sheet::Rename(_) | Sheet::Edit { .. }) {
            self.rename = None;
        }
        self.sheet = Some(sheet);
        cx.notify();
    }

    pub(crate) fn close_sheet(&mut self, cx: &mut Context<Self>) {
        self.sheet = None;
        self.rename = None;
        cx.notify();
    }

    /// Back: menu / sheet → drawer → overlay panel → page. Returns whether anything was
    /// popped.
    pub(crate) fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        for handle in self.swipes.values() {
            crate::mobile::gesture::close_row(handle);
        }
        if self.sheet.is_some() {
            self.close_sheet(cx);
            return true;
        }
        // A blocking card in a sheet: Back cancels small dialogs only (design.md §6.3); a
        // pending approval / question / plan stays pending and Back leaves the page (the
        // session keeps waiting and shows under Needs you).
        if let Some(view) = self.sheet_interaction(cx)
            && view.read(cx).touch_dismissible()
        {
            view.update(cx, InteractionView::dismiss);
            return true;
        }
        if self.drawer {
            self.drawer = false;
            cx.notify();
            return true;
        }
        if let Some((key, id)) = self.overlay_panel(cx) {
            if let Some(p) = self.pane(key).cloned() {
                p.update(cx, |p, cx| p.close_panel(&id, cx));
            }
            return true;
        }
        if self.stack.len() <= 1 {
            return false;
        }
        window.blur(cx);
        self.stack.pop();
        if self.page() == &Page::Sessions {
            self.sync_open_sessions(cx);
            self.refresh_sessions(cx);
        }
        cx.notify();
        true
    }

    /// The overlay / fullscreen panel of the visible conversation (rendered as an M5 page).
    pub(crate) fn overlay_panel(&self, cx: &App) -> Option<(u64, String)> {
        let Page::Session(key) = self.page() else {
            return None;
        };
        let pane = self.pane(*key)?.read(cx);
        pane.model
            .ui
            .panels()
            .find(|p| {
                matches!(
                    p.placement,
                    PanelPlacement::Overlay | PanelPlacement::Fullscreen
                )
            })
            .map(|p| (*key, p.id.clone()))
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.back(window, cx) {
            window.blur(cx);
            cx.stop_propagation();
        }
    }

    fn on_back_gesture(
        &mut self,
        e: &BackGestureEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if e.phase == BackGesturePhase::Committed {
            self.back(window, cx);
        }
    }

    // ── launch & deep links ──────────────────────────────────────────────────

    fn poll_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = (MobilePlatform::get(cx).take_link)() else {
            return;
        };
        if let Some(launch) = Launch::from_link(&link) {
            if launch.open.as_deref() == Some("pair") {
                self.pair
                    .update(cx, |s, cx| s.set_value(link.clone(), window, cx));
            }
            self.apply_launch(launch, window, cx);
        }
    }

    /// Applies a launch request once the session list is known.
    fn apply_launch(&mut self, launch: Launch, window: &mut Window, cx: &mut Context<Self>) {
        let open = launch.open.clone().unwrap_or_default();
        let (what, arg) = open.split_once(':').unwrap_or((open.as_str(), ""));
        let mut pending = Some(launch.clone());
        match what {
            "new" => self.new_session(None, window, cx),
            "resume" => {
                let first = self
                    .sessions
                    .groups()
                    .into_iter()
                    .flat_map(|g| g.sessions)
                    .next();
                if let Some(e) = first {
                    self.open_session(&e.summary.id, window, cx);
                }
            }
            "board" => {
                self.new_session(None, window, cx);
                pending = Some(Launch {
                    open: Some("board".into()),
                    ..launch.clone()
                });
            }
            "settings" => {
                let page = match arg {
                    "appearance" => SettingsPage::Appearance,
                    "models" => SettingsPage::Models,
                    "sessions" => SettingsPage::Sessions,
                    "permissions" => SettingsPage::Permissions,
                    "extensions" => SettingsPage::Extensions,
                    "hub" => SettingsPage::Hub,
                    "remote" => SettingsPage::Remote,
                    "about" => SettingsPage::About,
                    "" => SettingsPage::Root,
                    ns => SettingsPage::Namespace(ns.to_owned()),
                };
                self.open_settings(page, cx);
                pending = None;
            }
            "search" => {
                if let Some(q) = &launch.query {
                    self.search
                        .update(cx, |s, cx| s.set_value(q.clone(), window, cx));
                }
                self.push(Page::Search, cx);
                pending = None;
            }
            "hosts" => {
                self.open_settings(SettingsPage::Remote, cx);
                pending = None;
            }
            "pair" => {
                self.open_settings(SettingsPage::Remote, cx);
                self.push(Page::Pair, cx);
                pending = None;
            }
            _ => {
                if launch.menu
                    && let Some(e) = self.first_session()
                {
                    self.open_sheet(Sheet::RowMenu(e), cx);
                }
                if launch.swipe
                    && let Some(e) = self.first_session()
                {
                    let handle = self.swipes.entry(e).or_default().clone();
                    crate::mobile::gesture::open_row(&handle, 2);
                }
                pending = None;
            }
        }
        self.launch = pending;
        cx.notify();
    }

    fn first_session(&self) -> Option<String> {
        self.sessions
            .groups()
            .into_iter()
            .flat_map(|g| g.sessions)
            .map(|e| e.summary.id)
            .next()
    }

    fn apply_launch_sheet(&mut self, launch: &Launch, cx: &mut Context<Self>) {
        if launch.open.as_deref() == Some("board")
            && let Some(key) = self.active_key()
        {
            self.open_board(key, false, cx);
        }
        match launch.sheet.as_deref() {
            Some("model") => self.sheet = Some(Sheet::Model),
            Some("actions") => self.sheet = Some(Sheet::Actions),
            Some("status") => self.sheet = Some(Sheet::Status),
            _ => {}
        }
        if launch.home {
            self.stack.truncate(1);
            self.sync_open_sessions(cx);
        }
        if launch.drawer {
            self.with_pane(cx, |p, cx| {
                if !p.side.is_open() {
                    p.side.open(SideTab::Review);
                }
                cx.notify();
            });
            self.drawer = true;
        }
        cx.notify();
    }

    /// Opens the board page for pane `key`, asking the extension for the board when needed.
    pub(crate) fn open_board(&mut self, key: u64, ask: bool, cx: &mut Context<Self>) {
        if ask && let Some(pane) = self.pane(key).cloned() {
            let has = pane
                .read(cx)
                .model
                .ui
                .panel(crate::model::kanban::BOARD_PANEL)
                .is_some();
            if !has {
                pane.update(cx, |p, cx| p.run_prompt("/kanboard", cx));
            }
        }
        self.sheet = None;
        self.drawer = false;
        if self.page() != &Page::Board(key) {
            self.stack.push(Page::Board(key));
        }
        cx.notify();
    }

    // ── shared rendering ─────────────────────────────────────────────────────

    /// App bar (design.md §6.1/§6.2): 52 high, leading, title, trailing actions.
    pub(crate) fn app_bar(
        t: &OinoTheme,
        leading: Option<AnyElement>,
        title: AnyElement,
        actions: Vec<AnyElement>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .w_full()
            .h(px(touch::APP_BAR))
            .px(px(space::S1))
            .gap(px(space::S1))
            .bg(t.frame())
            .children(leading)
            .child(div().flex_1().min_w_0().px(px(space::S2)).child(title))
            .children(actions)
            .into_any_element()
    }

    /// A 44 pt icon button.
    pub(crate) fn icon_btn(
        id: impl Into<ElementId>,
        glyph: IconName,
        label: &'static str,
        f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        Btn::icon_only(id, glyph, label)
            .size(BtnSize::Touch)
            .on_click(f)
            .into_any_element()
    }

    /// The back button of a page header.
    pub(crate) fn back_btn(cx: &mut Context<Self>) -> AnyElement {
        Self::icon_btn(
            "m-back",
            IconName::ChevronLeft,
            "Back",
            cx.listener(|this, _, window, cx| {
                this.back(window, cx);
            }),
        )
    }

    /// A page title (detail header, design.md §6.5): `Lg` 500, truncated, optional sub-line.
    pub(crate) fn title(
        t: &OinoTheme,
        title: impl Into<SharedString>,
        sub: Option<String>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                div()
                    .truncate()
                    .text_role(t, TextRole::Lg)
                    .weight_medium()
                    .text_color(t.fg())
                    .child(title.into()),
            )
            .when_some(sub, |el, s| {
                el.child(
                    div()
                        .truncate()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(s)),
                )
            })
            .into_any_element()
    }

    /// A section label row (uppercase, muted).
    pub(crate) fn section(t: &OinoTheme, label: impl Into<SharedString>) -> AnyElement {
        div()
            .px(px(space::S4))
            .pt(px(space::S4))
            .pb(px(space::S1))
            .child(crate::components::badge::section_label(t, label))
            .into_any_element()
    }

    /// A 56-high list row: optional leading icon, title, sub-line, trailing element.
    pub(crate) fn list_row(
        t: &OinoTheme,
        id: impl Into<ElementId>,
        glyph: Option<IconName>,
        title: impl Into<SharedString>,
        sub: Option<String>,
        trailing: Option<AnyElement>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id.into())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::S3))
            .w_full()
            .min_h(px(touch::ROW))
            .px(px(space::S4))
            .cursor_pointer()
            .active(|s| s.bg(t.hover()))
            .when_some(glyph, |el, g| {
                el.child(icon(g, size::ICON_LG, t.fg_subtle()))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_role(t, TextRole::Base)
                            .text_color(t.fg())
                            .child(title.into()),
                    )
                    .when_some(sub, |el, s| {
                        el.child(
                            div()
                                .truncate()
                                .text_role(t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .child(SharedString::from(s)),
                        )
                    }),
            )
            .children(trailing)
    }

    /// A chevron for navigable rows.
    pub(crate) fn chevron(t: &OinoTheme) -> AnyElement {
        icon(IconName::ChevronRight, size::ICON, t.fg_subtlest()).into_any_element()
    }

    fn render_toasts(&self, t: &OinoTheme, top: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let visible = self.toasts.visible();
        if visible.is_empty() {
            return None;
        }
        Some(
            div()
                .absolute()
                .top(px(top + touch::APP_BAR))
                .left(px(space::S3))
                .right(px(space::S3))
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .children(visible.iter().map(|toast| {
                    let (glyph, c) = match toast.kind {
                        ToastKind::Info => (IconName::Info, t.info()),
                        ToastKind::Success => (IconName::Check, t.success()),
                        ToastKind::Warning => (IconName::TriangleAlert, t.warning()),
                        ToastKind::Error => (IconName::CircleX, t.danger()),
                    };
                    let id = toast.id;
                    div()
                        .id(ElementId::NamedInteger("m-toast".into(), id))
                        .flex()
                        .items_start()
                        .gap(px(space::S3))
                        .py(px(space::S3))
                        .px(px(space::S4))
                        .rounded(px(radius::R_2XL))
                        .bg(t.toast())
                        .border_1()
                        .border_color(t.popover_border())
                        .shadow(t.e3())
                        .occlude()
                        // Swipe up (or tap) dismisses.
                        .on_scroll(cx.listener(move |this, e: &gpui_kit::ScrollEvent, _, cx| {
                            let dy = f32::from(e.delta.pixel_delta(px(size::ICON)).y);
                            if dy < -space::S2 {
                                this.toasts.dismiss(id);
                                this.ensure_toast_timer(cx);
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.toasts.dismiss(id);
                            this.ensure_toast_timer(cx);
                            cx.notify();
                        }))
                        .child(div().pt(px(space::HALF)).child(icon(glyph, size::ICON, c)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(space::HALF))
                                .text_role(t, TextRole::Base)
                                .text_color(t.fg())
                                .when_some(toast.title.clone(), |el, title| {
                                    el.child(div().weight_medium().child(title))
                                })
                                .child(div().line_clamp(3).child(toast.message.clone())),
                        )
                }))
                .into_any_element(),
        )
    }
}

/// A stable notification id for `key`.
fn notification_id(key: &str) -> i32 {
    let h = key.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    i32::try_from(h & 0x7fff_ffff).unwrap_or(1)
}

impl Render for MobileShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.sessions.is_empty()
            && let Some(launch) = self.launch.take()
        {
            self.apply_launch(launch, window, cx);
        }
        let t = OinoTheme::get(cx);
        let hooks = MobilePlatform::get(cx);
        let insets = (hooks.safe_area)();
        let keyboard = (hooks.keyboard_height)();
        let bottom = insets.bottom.max(keyboard);
        let page = self.page().clone();
        let body = match &page {
            Page::Sessions => self.render_sessions(window, cx),
            Page::Session(key) => match self.pane(*key).cloned() {
                Some(pane) => match self.overlay_panel(cx) {
                    Some((_, id)) => self.render_panel_page(&pane, &id, window, cx),
                    None => self.render_conversation(&pane, window, cx),
                },
                None => self.render_sessions(window, cx),
            },
            Page::Settings(p) => self.render_settings(p, window, cx),
            Page::Search => self.render_search(window, cx),
            Page::Board(key) => match self.pane(*key).cloned() {
                Some(pane) => self.render_board(&pane, window, cx),
                None => self.render_sessions(window, cx),
            },
            Page::Pair => self.render_pair(window, cx),
            Page::Diff { title, patch } => self.render_diff_page(title, patch, cx),
            Page::Output { title, text } => self.render_output_page(title, text, cx),
        };
        let drawer = match (&page, self.drawer) {
            (Page::Session(key), true) => self
                .pane(*key)
                .cloned()
                .map(|pane| self.render_drawer(&pane, window, cx)),
            _ => None,
        };
        let sheet = self.render_sheet(window, cx);
        self.press.set(sheet.is_none() && drawer.is_none());
        let toasts = self.render_toasts(&t, insets.top, cx);
        div()
            .id("m-root")
            .size_full()
            .relative()
            .bg(t.frame())
            .text_color(t.fg())
            .font_family(t.ui_font())
            .text_role(&t, TextRole::Base)
            .track_focus(&self.focus)
            // Capture: Back (= `escape` on Android) navigates before the pane sees it.
            .capture_key_down(cx.listener(Self::on_key))
            .capture_any_mouse_down({
                let tap = Rc::clone(&self.last_tap);
                move |_, _, _| tap.set(Some(Instant::now()))
            })
            .on_back_gesture(cx.listener(Self::on_back_gesture))
            .pt(px(insets.top))
            .pb(px(bottom))
            .pl(px(insets.left))
            .pr(px(insets.right))
            // Overlays are absolutely positioned, which ignores the root's padding: keep them
            // inside a box that already excludes the safe areas and the keyboard.
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(body)
                    .children(drawer)
                    .children(sheet),
            )
            .children(toasts)
    }
}

/// Button helper re-exported to the screen modules.
pub(crate) fn pill_btn(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<IconName>,
    variant: Variant,
) -> Btn {
    let b = Btn::new(id)
        .label(label)
        .variant(variant)
        .size(BtnSize::Touch);
    match glyph {
        Some(g) => b.icon(g),
        None => b,
    }
}
