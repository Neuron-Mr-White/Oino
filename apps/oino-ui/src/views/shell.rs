//! The window root (design.md §3.1): independent frames — sidebar, conversation panes (split
//! ≤ 4), side frame, bottom frame — with 4 px resize gaps; routes for Settings, Board,
//! Extensions and onboarding; command palette, toasts, modals and the frame-time overlay.
//!
//! "Closing a pane ≠ stopping a session": sessions keep running in hidden panes.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, PathPromptOptions, Pixels, Render, SharedString, Styled as _,
    Subscription, Task, Window, div, px,
};

use crate::components::header::HeaderState;
use crate::components::palette::{PaletteEvent, PaletteView};
use crate::components::toast::{ToastKind, Toasts, stack};
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
use crate::views::session_pane::{PaneEvent, PaneMenu, SessionPane, SideTab};
use crate::views::settings_page::{SettingsEvent, SettingsSection, SettingsView};
use crate::views::{
    Abort, CloseFind, CommandPalette, FindInConversation, FocusComposer, NewSession, OpenFolder,
    OpenSettings, PickModel, PickThinking, SplitPane, ToggleBottom, ToggleFrameOverlay,
    ToggleSidePane, ToggleSidebar, ToggleStatusPanel, ToggleTheme, TreeView,
};

/// Main-area routes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Session panes.
    Sessions,
    /// Settings page.
    Settings,
    /// Kanboard board (D9).
    Board,
    /// Extensions page (D12).
    Extensions,
    /// First-run onboarding (D13).
    Onboarding,
}

/// Sidebar list mode (design.md §4.7 `ViewSwitcher`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidebarView {
    /// Grouped by workspace.
    #[default]
    Group,
    /// Flat by time.
    Recent,
}

/// Which frame edge is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    Sidebar,
    Side,
    Bottom,
}

pub(crate) struct Pane {
    pub(crate) key: u64,
    pub(crate) entity: Entity<SessionPane>,
    _subscription: Subscription,
}

/// The window root view.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent frame visibility flags"
)]
pub struct Workspace {
    pub(crate) host: Arc<dyn SessionHost>,
    pub(crate) settings: AppSettings,
    pub(crate) sessions: SessionList,
    pub(crate) panes: Vec<Pane>,
    /// Visible panes (split, ≤ 4), left to right.
    pub(crate) layout: Vec<u64>,
    pub(crate) focused: Option<u64>,
    next_key: u64,
    pub(crate) route: Route,
    pub(crate) sidebar_open: bool,
    pub(crate) side_open: bool,
    pub(crate) bottom_open: bool,
    pub(crate) sidebar_width: Pixels,
    side_width: Pixels,
    bottom_height: Pixels,
    drag: Option<Drag>,
    pub(crate) sidebar_view: SidebarView,
    pub(crate) sidebar_menu: Option<String>,
    palette: Option<(Entity<PaletteView>, Subscription)>,
    settings_view: Option<(Entity<SettingsView>, Subscription)>,
    pub(crate) extensions: Vec<ExtensionInfo>,
    toasts: Toasts,
    toast_task: Option<Task<()>>,
    pub(crate) opening: bool,
    pub(crate) history: (Vec<String>, usize),
    pub(crate) window_width: Pixels,
    pub(crate) update_available: Option<String>,
    pending_panes: usize,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

impl Workspace {
    /// The root view; lists sessions and opens a new session in the default directory.
    pub fn new(host: Arc<dyn SessionHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = host.app_settings();
        theme::install(&settings, Some(window), cx);
        let probe = cx.default_global::<perf::FrameProbe>();
        probe.log = std::env::var("OINO_FRAME_LOG").is_ok_and(|v| v == "1");
        probe.overlay = std::env::var("OINO_FRAME_OVERLAY").is_ok_and(|v| v == "1");
        let subscriptions =
            vec![
                cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
                    if this.settings.theme == ThemePref::System {
                        theme::install(&this.settings, Some(window), cx);
                        cx.notify();
                    }
                }),
            ];
        let onboarding =
            !settings.onboarded && std::env::var("OINO_DESKTOP_ONBOARDING").is_ok_and(|v| v == "1");
        let mut ws = Self {
            host,
            settings,
            sessions: SessionList::new(),
            panes: Vec::new(),
            layout: Vec::new(),
            focused: None,
            next_key: 0,
            route: if onboarding {
                Route::Onboarding
            } else {
                Route::Sessions
            },
            sidebar_open: true,
            side_open: false,
            bottom_open: false,
            sidebar_width: px(size::SIDEBAR),
            side_width: px(size::SIDE_PANE),
            bottom_height: px(size::BOTTOM),
            drag: None,
            sidebar_view: SidebarView::Group,
            sidebar_menu: None,
            palette: None,
            settings_view: None,
            extensions: Vec::new(),
            toasts: Toasts::default(),
            toast_task: None,
            opening: false,
            history: (Vec::new(), 0),
            window_width: window.viewport_size().width,
            update_available: None,
            pending_panes: 0,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        ws.refresh_sessions(cx);
        ws.load_extensions(cx);
        let cwd = ws.host.info().default_cwd;
        ws.open(OpenRequest::New { cwd: cwd.clone() }, window, cx);
        // Demo / screenshot hooks: extra split panes and an initial surface.
        let panes = std::env::var("OINO_DESKTOP_PANES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1)
            .min(4);
        if panes > 1 {
            ws.pending_panes = panes - 1;
        }
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

    /// Opens a named surface (`palette`, `settings[:section]`, `board`, `extensions`, `side`).
    pub fn open_surface(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match name.split_once(':').unwrap_or((name, "")) {
            ("palette", q) => self.open_palette(q, window, cx),
            ("settings", ns) => self.open_settings((!ns.is_empty()).then_some(ns), window, cx),
            ("board", _) => self.open_board(window, cx),
            ("extensions", _) => self.run_app_action("extensions", window, cx),
            ("side", _) => {
                self.side_open = true;
                cx.notify();
            }
            ("model", _) => self.with_pane(cx, |p, cx| p.toggle_menu(PaneMenu::Model, cx)),
            _ => {}
        }
    }

    pub(crate) fn active_pane(&self) -> Option<&Entity<SessionPane>> {
        let key = self.focused?;
        self.panes.iter().find(|p| p.key == key).map(|p| &p.entity)
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

    /// Open panes are always listed (new sessions have no file until the first message).
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
            self.sessions.set_live(&model.id, p.live());
        }
    }

    pub(crate) fn open(
        &mut self,
        request: OpenRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.panes.push(Pane {
            key,
            entity,
            _subscription: subscription,
        });
        self.activate(key, window, cx);
        self.sync_open_sessions(cx);
        if self.pending_panes > 0 {
            self.pending_panes -= 1;
            self.split(window, cx);
        }
    }

    /// Shows pane `key` (replacing the focused slot of the layout) and focuses it.
    pub(crate) fn activate(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.layout.contains(&key) {
            match self
                .focused
                .and_then(|f| self.layout.iter().position(|k| *k == f))
            {
                Some(slot) => self.layout[slot] = key,
                None => self.layout.push(key),
            }
        }
        self.focus_pane(key, window, cx);
        self.route = Route::Sessions;
        if let Some(id) = self.pane_by_key(key).map(|p| p.read(cx).model.id.clone()) {
            let (stack, at) = &mut self.history;
            stack.truncate(*at + usize::from(!stack.is_empty()));
            if stack.last() != Some(&id) {
                stack.push(id);
            }
            *at = stack.len().saturating_sub(1);
        }
        cx.notify();
    }

    fn focus_pane(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.focused = Some(key);
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
                    && self.focused != Some(key)
                {
                    LiveStatus::Unread
                } else {
                    *status
                };
                self.sessions.set_live(&id, status);
                if status == LiveStatus::Attention && !window.is_window_active() {
                    let title = pane
                        .read(cx)
                        .model
                        .ui
                        .dialog()
                        .map(|d| d.title.clone())
                        .unwrap_or_default();
                    self.toast(ToastKind::Warning, Some("Oino needs you".into()), title, cx);
                }
                self.update_title(window, cx);
                cx.notify();
            }
            PaneEvent::Changed => {
                self.sync_open_sessions(cx);
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
            PaneEvent::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                cx.notify();
            }
            PaneEvent::ToggleSide => {
                self.side_open = !self.side_open;
                cx.notify();
            }
            PaneEvent::ClosePane => self.close_pane(key, window, cx),
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
                self.toast(kind, None, message, cx);
            }
            UiEffect::OsNotify { title, body } => {
                // Native notifications need a platform integration; a toast with the title bold.
                self.toast(ToastKind::Info, Some(title.into()), body, cx);
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

    /// Applies a settings change everywhere (theme, panes, persistence).
    pub(crate) fn update_settings(
        &mut self,
        f: impl FnOnce(&mut AppSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        f(&mut self.settings);
        self.host.save_app_settings(&self.settings);
        theme::install(&self.settings, Some(window), cx);
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

    // ── navigation ──────────────────────────────────────────────────────────

    pub(crate) fn open_palette(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            "abort" => self.with_pane(cx, SessionPane::abort),
            "compact" => self.with_pane(cx, SessionPane::compact),
            "fork" => self.with_pane(cx, SessionPane::fork_session),
            "tree" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Tree, cx)),
            "review" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Review, cx)),
            "files" => self.with_pane(cx, |p, cx| p.open_side(SideTab::Files, cx)),
            "toggle-sidebar" => {
                self.sidebar_open = !self.sidebar_open;
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
            "board" => self.open_board(window, cx),
            "extensions" => {
                self.route = Route::Extensions;
                self.load_extensions(cx);
                cx.notify();
            }
            "theme-light" => self.update_settings(|s| s.theme = ThemePref::Light, window, cx),
            "theme-dark" => self.update_settings(|s| s.theme = ThemePref::Dark, window, cx),
            "theme-system" => self.update_settings(|s| s.theme = ThemePref::System, window, cx),
            "settings" => self.open_settings(None, window, cx),
            "refresh" => self.refresh_sessions(cx),
            "split" => self.split(window, cx),
            "check-updates" => self.with_pane(cx, |p, cx| p.run_prompt("/unipi:update", cx)),
            "frame-overlay" => {
                perf::toggle_overlay(cx);
                cx.notify();
            }
            _ => {}
        }
    }

    pub(crate) fn open_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self
            .panes
            .iter()
            .find(|p| p.entity.read(cx).model.id == id)
            .map(|p| p.key)
        {
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
            || self.host.info().default_cwd,
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

    /// ⌘\: a new pane next to the focused one (≤ 4).
    pub(crate) fn split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.len() >= 4 {
            self.toast(ToastKind::Info, None, "At most four panes", cx);
            return;
        }
        // A free slot: the new pane is appended instead of replacing the focused one.
        self.focused = None;
        self.new_session(window, cx);
    }

    /// Closes pane `key` from the layout (its session keeps running).
    pub(crate) fn close_pane(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout.len() <= 1 {
            return;
        }
        self.layout.retain(|k| *k != key);
        if self.focused == Some(key)
            && let Some(first) = self.layout.first().copied()
        {
            self.focus_pane(first, window, cx);
        }
        cx.notify();
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
            Some(ns) => SettingsSection::Extension(ns.to_owned()),
            None => SettingsSection::Appearance,
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
        let sub = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Back => {
                    this.route = Route::Sessions;
                    this.settings_view = None;
                    if let Some(pane) = this.active_pane().cloned() {
                        SessionPane::focus(&pane, window, cx);
                    }
                    cx.notify();
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
            },
        );
        self.settings_view = Some((view, sub));
        self.route = Route::Settings;
        cx.notify();
    }

    pub(crate) fn open_board(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let has = self
            .active_pane()
            .is_some_and(|p| p.read(cx).model.ui.panel(kanban::BOARD_PANEL).is_some());
        if !has {
            self.with_pane(cx, |p, cx| p.run_prompt("/unipi:kanboard", cx));
        }
        self.route = Route::Board;
        cx.notify();
    }

    pub(crate) fn history_step(&mut self, dir: isize, window: &mut Window, cx: &mut Context<Self>) {
        let (stack, at) = &self.history;
        let Some(next) = at.checked_add_signed(dir).filter(|n| *n < stack.len()) else {
            return;
        };
        let id = stack[next].clone();
        self.history.1 = next;
        if let Some(key) = self
            .panes
            .iter()
            .find(|p| p.entity.read(cx).model.id == id)
            .map(|p| p.key)
        {
            self.layout.retain(|k| *k != key);
            self.focus_pane(key, window, cx);
            if let Some(slot) = self.layout.first_mut() {
                *slot = key;
            } else {
                self.layout.push(key);
            }
            cx.notify();
        }
    }

    // ── rendering ───────────────────────────────────────────────────────────

    fn frame(&self, t: &OinoTheme) -> gpui_kit::Div {
        div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(t.frame())
            .border_1()
            .border_color(t.border())
            .rounded(px(if cfg!(target_os = "windows") {
                radius::R_MD
            } else {
                radius::R_XL
            }))
    }

    fn handle(
        &self,
        t: &OinoTheme,
        drag: Drag,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.drag == Some(drag);
        let line = div()
            .when(vertical, |el| el.w(px(space::HALF)).h_full())
            .when(!vertical, |el| el.h(px(space::HALF)).w_full())
            .rounded(px(radius::R_FULL))
            .when(active, |el| el.bg(t.resize_handle()));
        div()
            .id(SharedString::from(format!("handle-{drag:?}")))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .when(vertical, |el| {
                el.w(px(size::FRAME_GAP))
                    .h_full()
                    .py(px(radius::R_XL))
                    .cursor_col_resize()
            })
            .when(!vertical, |el| {
                el.h(px(size::FRAME_GAP))
                    .w_full()
                    .px(px(radius::R_XL))
                    .cursor_row_resize()
            })
            .hover(|s| s.bg(gpui_kit::transparent_black()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.drag = Some(drag);
                    cx.notify();
                }),
            )
            .child(line)
            .into_any_element()
    }

    fn on_drag_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };
        let vp = window.viewport_size();
        match drag {
            Drag::Sidebar => {
                self.sidebar_width = e.position.x.clamp(px(size::SIDEBAR), vp.width * 0.5);
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

    #[allow(clippy::too_many_lines, reason = "the frame layout")]
    fn render_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let width = window.viewport_size().width;
        let narrow = width < px(breakpoint::NARROW);
        let sidebar_visible = self.sidebar_open && !narrow;
        let mut row = div().flex().flex_row().flex_1().min_w_0().h_full();
        let layout = self.layout.clone();
        let multi = layout.len() > 1;
        for (i, key) in layout.iter().enumerate() {
            let Some(pane) = self.pane_by_key(*key).cloned() else {
                continue;
            };
            let focused = self.focused == Some(*key);
            let state = HeaderState {
                sidebar_collapsed: !sidebar_visible,
                side_open: self.side_open,
                bottom_open: self.bottom_open,
                primary: i == 0 || focused,
                closable: multi,
            };
            pane.update(cx, |p, _| p.header_state = state);
            if i > 0 {
                row = row.child(div().w(px(size::FRAME_GAP)).flex_shrink_0());
            }
            let k = *key;
            row = row.child(
                self.frame(&t)
                    .id(SharedString::from(format!("pane-{k}")))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .when(multi && focused, |el| el.border_color(t.border_hover()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            if this.focused != Some(k) {
                                this.focus_pane(k, window, cx);
                                cx.notify();
                            }
                        }),
                    )
                    // Cached: a pane re-renders only when it was notified (streaming in
                    // one pane does not rebuild the others, design.md §2.11).
                    .child(
                        pane.clone()
                            .cached(gpui_kit::StyleRefinement::default().size_full()),
                    ),
            );
        }
        if layout.is_empty() {
            row = row.child(
                self.frame(&t)
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_role(&t, TextRole::Base)
                    .text_color(t.fg_subtle())
                    .child(if self.opening {
                        "Starting session…"
                    } else {
                        "No session"
                    }),
            );
        }
        let side = self
            .side_open
            .then(|| self.active_pane().cloned())
            .flatten()
            .map(|pane| {
                pane.update(cx, |p, cx| {
                    if !p.side.is_open() {
                        p.side.open(SideTab::Review);
                    }
                    p.render_side(window, cx)
                })
            });
        let bottom = self
            .bottom_open
            .then(|| self.active_pane().cloned())
            .flatten()
            .map(|pane| pane.update(cx, |p, cx| p.render_bottom(window, cx)));
        let center = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(row)
            .when_some(bottom, |el, b| {
                el.child(self.handle(&t, Drag::Bottom, false, cx)).child(
                    self.frame(&t)
                        .h(self.bottom_height)
                        .flex_shrink_0()
                        .child(b),
                )
            });
        let side_fits = width
            - self.side_width
            - if sidebar_visible {
                self.sidebar_width
            } else {
                px(space::S0)
            }
            >= px(size::CONVERSATION_MIN);
        div()
            .flex()
            .flex_row()
            .size_full()
            .child(center)
            .when_some(side.filter(|_| side_fits || narrow), |el, s| {
                if narrow {
                    el.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .w(px(size::SIDE_PANE_MIN + space::S10))
                            .p(px(size::WINDOW_INSET))
                            .child(self.frame(&t).size_full().shadow(t.e2()).child(s)),
                    )
                } else {
                    el.child(self.handle(&t, Drag::Side, true, cx)).child(
                        self.frame(&t)
                            .w(self.side_width)
                            .flex_shrink_0()
                            .h_full()
                            .child(s),
                    )
                }
            })
            .into_any_element()
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    #[allow(clippy::too_many_lines, reason = "root layout and action wiring")]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        perf::begin(cx);
        let t = OinoTheme::get(cx);
        self.window_width = window.viewport_size().width;
        let narrow = self.window_width < px(breakpoint::NARROW);
        let sidebar = (self.sidebar_open
            && !narrow
            && !matches!(self.route, Route::Onboarding | Route::Settings))
        .then(|| self.render_sidebar(window, cx));
        let main: AnyElement = match self.route {
            Route::Sessions => self.render_sessions(window, cx),
            Route::Settings => match &self.settings_view {
                Some((view, _)) => self
                    .frame(&t)
                    .size_full()
                    .child(view.clone())
                    .into_any_element(),
                None => self.render_sessions(window, cx),
            },
            Route::Board => self.render_board(window, cx),
            Route::Extensions => self.render_extensions(cx),
            Route::Onboarding => self.render_onboarding(window, cx),
        };
        let modal = self
            .active_pane()
            .cloned()
            .and_then(|p| p.update(cx, |p, cx| p.render_modal(window, cx)));
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
        let toasts = stack(&t, &self.toasts, on_dismiss, on_hover);
        let inset = if cfg!(target_os = "windows") {
            size::WINDOW_INSET_WINDOWS
        } else {
            size::WINDOW_INSET
        };
        let frames = div()
            .absolute()
            .top(px(inset))
            .left(px(inset))
            .right(px(inset))
            .bottom(px(inset))
            .flex()
            .flex_row()
            .children(sidebar.map(|s| {
                div()
                    .flex()
                    .flex_row()
                    .h_full()
                    .child(
                        div()
                            .w(self.sidebar_width)
                            .h_full()
                            .flex_shrink_0()
                            .child(s),
                    )
                    .child(self.handle(&t, Drag::Sidebar, true, cx))
            }))
            .child(div().flex_1().min_w_0().h_full().child(main));
        div()
            .id("root")
            .relative()
            .size_full()
            .bg(t.window())
            .text_color(t.fg())
            .font_family(t.ui_font())
            .text_role(&t, TextRole::Base)
            .track_focus(&self.focus)
            .on_mouse_move(cx.listener(Self::on_drag_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &NewSession, window, cx| this.new_session(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
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
            .on_action(cx.listener(|this, _: &PickModel, _, cx| {
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
            .on_action(cx.listener(|this, _: &SplitPane, window, cx| this.split(window, cx)))
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
            .child(frames)
            .children(modal)
            .when_some(self.palette.as_ref().map(|(p, _)| p.clone()), |el, p| {
                el.child(p)
            })
            .child(toasts)
            .children(perf::overlay(cx))
            .child(perf::end_marker())
    }
}
