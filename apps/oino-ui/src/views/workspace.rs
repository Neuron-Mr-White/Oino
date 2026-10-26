//! The window root (gpui-app §5.1): sidebar (workspaces → sessions), header, the active
//! session pane, side pane, status bar, command palette and settings.
//!
//! "Closing a pane ≠ stopping a session": panes stay alive (and keep streaming) when another
//! session is shown; only the visible pane renders.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, FocusHandle,
    Focusable, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px,
};
use oino_ai::ThinkingLevel;

use crate::host::{AppSettings, ExtensionInfo, OpenRequest, OpenedSession, SessionHost, ThemePref};
use crate::model::palette::{PaletteAction, PaletteItem, PaletteMode};
use crate::model::sessions::{LiveStatus, SessionEntry, SessionList, SessionSummary, age};
use crate::model::ui_state::{NotifyKind, UiEffect};
use crate::rt::Tokio;
use crate::views::palette::{PaletteEvent, PaletteView};
use crate::views::session_pane::{PaneEvent, SessionPane, SideTab};
use crate::views::theme::{self, tone_color};
use crate::views::{
    Abort, CommandPalette, NewSession, OpenFolder, OpenSettings, PickModel, PickThinking,
    ToggleSidePane, ToggleSidebar, ToggleTheme,
};

/// Settings pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsPage {
    General,
    Models,
    Extensions,
    Keybindings,
    About,
}

struct Pane {
    key: u64,
    entity: Entity<SessionPane>,
    _subscription: Subscription,
}

/// The window root view.
pub struct Workspace {
    host: Arc<dyn SessionHost>,
    settings: AppSettings,
    sessions: SessionList,
    panes: Vec<Pane>,
    active: Option<u64>,
    next_key: u64,
    sidebar_open: bool,
    side_open: bool,
    palette: Option<(Entity<PaletteView>, Subscription)>,
    settings_page: Option<SettingsPage>,
    extensions: Vec<ExtensionInfo>,
    search: Entity<InputState>,
    opening: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

fn now_ms() -> i64 {
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
        theme::apply(settings.theme, Some(window), cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions"));
        let subscriptions =
            vec![
                cx.subscribe_in(&search, window, |this: &mut Self, input, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let query = input.read(cx).value().to_string();
                        this.sessions.set_query(&query);
                        cx.notify();
                    }
                }),
            ];
        let mut ws = Self {
            host,
            settings,
            sessions: SessionList::new(),
            panes: Vec::new(),
            active: None,
            next_key: 0,
            sidebar_open: true,
            side_open: true,
            palette: None,
            settings_page: None,
            extensions: Vec::new(),
            search,
            opening: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        ws.refresh_sessions(cx);
        let cwd = ws.host.info().default_cwd;
        ws.open(OpenRequest::New { cwd }, window, cx);
        ws
    }

    fn active_pane(&self) -> Option<&Entity<SessionPane>> {
        let key = self.active?;
        self.panes.iter().find(|p| p.key == key).map(|p| &p.entity)
    }

    fn refresh_sessions(&self, cx: &mut Context<Self>) {
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
            let model = &pane.entity.read(cx).model;
            let existing = self.sessions.get(&model.id).cloned();
            self.sessions.upsert(SessionSummary {
                id: model.id.clone(),
                path: model.file.as_ref().map(PathBuf::from),
                cwd: model.cwd.clone(),
                title: model.title(),
                modified_ms: existing.map_or_else(now_ms, |e| e.modified_ms),
                message_count: model.transcript.len(),
            });
            self.sessions
                .set_live(&model.id, pane.entity.read(cx).live());
        }
    }

    fn open(&mut self, request: OpenRequest, window: &mut Window, cx: &mut Context<Self>) {
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
                    Ok(Err(e)) => {
                        window.push_notification((NotificationType::Error, e.to_string()), cx);
                    }
                    Err(e) => {
                        window.push_notification((NotificationType::Error, e.to_string()), cx);
                    }
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
    }

    fn activate(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(key);
        if let Some(pane) = self.active_pane().cloned() {
            let id = pane.read(cx).model.id.clone();
            if self.sessions.live(&id) == LiveStatus::Unread {
                self.sessions.set_live(&id, LiveStatus::Idle);
            }
            SessionPane::focus(&pane, window, cx);
        }
        cx.notify();
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
                    && self.active != Some(key)
                {
                    LiveStatus::Unread
                } else {
                    *status
                };
                self.sessions.set_live(&id, status);
                cx.notify();
            }
            PaneEvent::Changed => {
                self.sync_open_sessions(cx);
                cx.notify();
            }
            PaneEvent::PickModel => self.open_palette(PaletteMode::Models, "", window, cx),
            PaneEvent::Effect(effect) => self.perform(effect.clone(), window, cx),
        }
    }

    fn perform(&mut self, effect: UiEffect, window: &mut Window, cx: &mut Context<Self>) {
        match effect {
            UiEffect::Notify { message, kind } => {
                let kind = match kind {
                    NotifyKind::Info => NotificationType::Info,
                    NotifyKind::Warning => NotificationType::Warning,
                    NotifyKind::Error => NotificationType::Error,
                };
                window.push_notification((kind, message), cx);
            }
            UiEffect::OsNotify { title, body } => {
                // Native OS notifications need a platform integration (request); a toast is
                // shown meanwhile, also when the window is focused.
                window.push_notification((NotificationType::Info, format!("{title}: {body}")), cx);
            }
            UiEffect::OpenUrl(url) => cx.open_url(&url),
            UiEffect::ClipboardWrite(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            UiEffect::SetTheme(name) => {
                let pref = if name.to_lowercase().contains("light") {
                    ThemePref::Light
                } else {
                    ThemePref::Dark
                };
                self.set_theme(pref, window, cx);
            }
            UiEffect::OpenSettings { .. } => {
                self.settings_page = Some(SettingsPage::General);
                cx.notify();
            }
            _ => {}
        }
    }

    fn set_theme(&mut self, pref: ThemePref, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme = pref;
        theme::apply(pref, Some(window), cx);
        self.host.save_app_settings(&self.settings);
        cx.notify();
    }

    fn update_settings(&mut self, f: impl FnOnce(&mut AppSettings), cx: &mut Context<Self>) {
        f(&mut self.settings);
        self.host.save_app_settings(&self.settings);
        for pane in &self.panes {
            let settings = self.settings.clone();
            pane.entity.update(cx, |p, cx| p.set_settings(settings, cx));
        }
        cx.notify();
    }

    // ── palette ──────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_lines, reason = "one arm per event / method kind")]
    fn palette_items(&self, mode: PaletteMode, cx: &App) -> Vec<PaletteItem> {
        let pane = self.active_pane().map(|p| p.read(cx));
        match mode {
            PaletteMode::Models => {
                let current = pane.and_then(|p| p.model.model.clone());
                pane.map(|p| p.models.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| PaletteItem {
                        current: current
                            .as_ref()
                            .is_some_and(|c| c.id == m.id && c.provider == m.provider),
                        label: if m.name.is_empty() {
                            m.id.clone()
                        } else {
                            m.name.clone()
                        },
                        detail: Some(format!("{} · {}k ctx", m.provider, m.context_window / 1000)),
                        keys: None,
                        group: "model",
                        action: PaletteAction::Model(m.provider.clone(), m.id.clone()),
                    })
                    .collect()
            }
            PaletteMode::Thinking => {
                let current = pane.map(|p| p.model.thinking);
                [
                    ThinkingLevel::Off,
                    ThinkingLevel::Minimal,
                    ThinkingLevel::Low,
                    ThinkingLevel::Medium,
                    ThinkingLevel::High,
                    ThinkingLevel::Xhigh,
                ]
                .into_iter()
                .map(|l| PaletteItem {
                    label: l.to_string(),
                    detail: None,
                    keys: None,
                    group: "thinking",
                    current: current == Some(l),
                    action: PaletteAction::Thinking(l.to_string()),
                })
                .collect()
            }
            PaletteMode::Command => {
                let app = |id: &str, label: &str, keys: &str| PaletteItem {
                    label: label.to_owned(),
                    detail: None,
                    keys: (!keys.is_empty()).then(|| keys.to_owned()),
                    group: "command",
                    current: false,
                    action: PaletteAction::App(id.to_owned()),
                };
                let mut items = vec![
                    app("new-session", "New session", "Ctrl+N"),
                    app("open-folder", "New session in folder…", "Ctrl+O"),
                    app("pick-model", "Select model", "Ctrl+L"),
                    app("pick-thinking", "Thinking level", "Ctrl+T"),
                    app("abort", "Abort run", "Ctrl+."),
                    app("compact", "Compact context", ""),
                    app("toggle-sidebar", "Toggle sidebar", "Ctrl+B"),
                    app("toggle-side", "Toggle side pane", "Ctrl+J"),
                    app("toggle-theme", "Toggle light/dark theme", ""),
                    app("settings", "Settings", "Ctrl+,"),
                    app("refresh", "Refresh session list", ""),
                ];
                if let Some(pane) = pane {
                    items.extend(
                        pane.commands()
                            .iter()
                            .map(|(name, description)| PaletteItem {
                                label: format!("/{name}"),
                                detail: description.clone(),
                                keys: None,
                                group: "slash",
                                current: false,
                                action: PaletteAction::Slash(name.clone()),
                            }),
                    );
                    items.extend(pane.model.ui.shortcuts.iter().map(|s| PaletteItem {
                        label: s.description.clone(),
                        detail: Some(s.id.clone()),
                        keys: Some(s.keys.clone()),
                        group: "extension",
                        current: false,
                        action: PaletteAction::App(format!("shortcut:{}", s.id)),
                    }));
                }
                let active_id = pane.map(|p| p.model.id.clone());
                for group in self.sessions.groups() {
                    items.extend(group.sessions.into_iter().map(|e| PaletteItem {
                        current: active_id.as_deref() == Some(e.summary.id.as_str()),
                        label: e.summary.title.clone(),
                        detail: Some(group.label.clone()),
                        keys: None,
                        group: "session",
                        action: PaletteAction::Session(e.summary.id),
                    }));
                }
                items
            }
        }
    }

    fn open_palette(
        &mut self,
        mode: PaletteMode,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self.palette_items(mode, cx);
        let view = cx.new(|cx| PaletteView::new(mode, items, query, window, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &PaletteEvent, window, cx| {
                this.palette = None;
                if let PaletteEvent::Pick(item) = event {
                    this.run_palette(item.action.clone(), window, cx);
                } else if let Some(pane) = this.active_pane().cloned() {
                    SessionPane::focus(&pane, window, cx);
                }
                cx.notify();
            },
        );
        self.palette = Some((view, subscription));
        cx.notify();
    }

    fn with_pane(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut SessionPane, &mut Context<SessionPane>),
    ) {
        if let Some(pane) = self.active_pane().cloned() {
            pane.update(cx, f);
        }
    }

    fn run_palette(&mut self, action: PaletteAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            PaletteAction::App(id) => self.run_app_action(&id, window, cx),
            PaletteAction::Slash(name) => {
                self.with_pane(cx, |p, cx| p.run_prompt(&format!("/{name}"), cx));
            }
            PaletteAction::Session(id) => self.open_session(&id, window, cx),
            PaletteAction::Model(provider, model) => {
                self.with_pane(cx, |p, cx| p.set_model(provider, model, cx));
            }
            PaletteAction::Thinking(level) => {
                if let Ok(level) = level.parse::<ThinkingLevel>() {
                    self.with_pane(cx, |p, cx| p.set_thinking(level, cx));
                }
            }
        }
        if let Some(pane) = self.active_pane().cloned() {
            SessionPane::focus(&pane, window, cx);
        }
    }

    fn run_app_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "new-session" => self.new_session(window, cx),
            "open-folder" => self.open_folder(window, cx),
            "pick-model" => self.open_palette(PaletteMode::Models, "", window, cx),
            "pick-thinking" => self.open_palette(PaletteMode::Thinking, "", window, cx),
            "abort" => self.with_pane(cx, SessionPane::abort),
            "compact" => self.with_pane(cx, SessionPane::compact),
            "toggle-sidebar" => {
                self.sidebar_open = !self.sidebar_open;
                cx.notify();
            }
            "toggle-side" => {
                self.side_open = !self.side_open;
                cx.notify();
            }
            "toggle-theme" => {
                let next = match self.settings.theme {
                    ThemePref::Dark => ThemePref::Light,
                    ThemePref::Light => ThemePref::Dark,
                };
                self.set_theme(next, window, cx);
            }
            "settings" => {
                self.settings_page = Some(SettingsPage::General);
                self.load_extensions(cx);
                cx.notify();
            }
            "refresh" => self.refresh_sessions(cx),
            other => {
                if let Some(shortcut) = other.strip_prefix("shortcut:") {
                    // `invoke_shortcut` is not in the client command set yet (request).
                    window.push_notification(
                        (
                            NotificationType::Warning,
                            format!("Extension shortcut {shortcut} cannot be invoked yet"),
                        ),
                        cx,
                    );
                }
            }
        }
    }

    fn open_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
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

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.active_pane().map_or_else(
            || self.host.info().default_cwd,
            |p| PathBuf::from(p.read(cx).model.cwd.clone()),
        );
        self.open(OpenRequest::New { cwd }, window, cx);
    }

    fn new_session_in(&mut self, cwd: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open(
            OpenRequest::New {
                cwd: PathBuf::from(cwd),
            },
            window,
            cx,
        );
    }

    #[allow(
        clippy::unused_self,
        reason = "view helper kept as a method for uniform call sites"
    )]
    fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn load_extensions(&self, cx: &mut Context<Self>) {
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

    // ── rendering ────────────────────────────────────────────────────────────

    fn status_dot(status: LiveStatus, cx: &App) -> AnyElement {
        let theme = cx.theme().clone();
        let (color, glyph) = match status {
            LiveStatus::Idle => (theme.muted_foreground.opacity(0.35), "•"),
            LiveStatus::Running => (theme.success, "●"),
            LiveStatus::Attention => (theme.warning, "◆"),
            LiveStatus::Unread => (theme.info, "●"),
        };
        div()
            .w(px(12.))
            .text_xs()
            .text_color(color)
            .child(glyph)
            .into_any_element()
    }

    #[allow(
        clippy::unused_self,
        reason = "view helper kept as a method for uniform call sites"
    )]
    fn session_row(
        &self,
        entry: &SessionEntry,
        active_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let id = entry.summary.id.clone();
        let is_active = active_id == Some(entry.summary.id.as_str());
        h_flex()
            .id(SharedString::from(format!("s-{}", entry.summary.id)))
            .w_full()
            .px_2()
            .py(px(5.))
            .gap_2()
            .rounded_md()
            .cursor_pointer()
            .when(is_active, |el| {
                el.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .hover(|s| s.bg(theme.sidebar_accent))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_session(&id, window, cx);
            }))
            .child(Self::status_dot(entry.status, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(SharedString::from(entry.summary.title.clone())),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(SharedString::from(age(now_ms(), entry.summary.modified_ms))),
            )
            .into_any_element()
    }

    #[allow(
        clippy::too_many_lines,
        reason = "declarative element tree; splitting scatters the layout"
    )]
    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let active_id = self.active_pane().map(|p| p.read(cx).model.id.clone());
        let pinned = self.sessions.pinned();
        let groups = self.sessions.groups();
        let mut list = v_flex().w_full().gap_1();
        if !pinned.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .pt_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Pinned"),
            );
            for entry in &pinned {
                list = list.child(self.session_row(entry, active_id.as_deref(), cx));
            }
        }
        for group in groups {
            let collapsed = self.sessions.is_collapsed(&group.cwd);
            let cwd = group.cwd.clone();
            let new_cwd = group.cwd.clone();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("g-{}", group.cwd)))
                    .w_full()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .gap_1()
                    .cursor_pointer()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.sessions.toggle_group(&cwd);
                        cx.notify();
                    }))
                    .child(if collapsed { "▸" } else { "▾" })
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(SharedString::from(group.label.clone())),
                    )
                    .child(
                        Button::new(SharedString::from(format!("gn-{}", group.cwd)))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip(SharedString::from(format!("New session in {}", group.cwd)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.new_session_in(&new_cwd, window, cx);
                            })),
                    ),
            );
            if !collapsed {
                for entry in group.sessions.iter().take(30) {
                    list = list.child(self.session_row(entry, active_id.as_deref(), cx));
                }
            }
        }
        v_flex()
            .w(px(264.))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(
                v_flex()
                    .p_2()
                    .gap_2()
                    .child(
                        h_flex()
                            .px_1()
                            .justify_between()
                            .child(div().font_weight(FontWeight::BOLD).child("Oino"))
                            .when(self.opening, |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child("opening…"),
                                )
                            }),
                    )
                    .child(
                        Button::new("new-session")
                            .w_full()
                            .outline()
                            .small()
                            .icon(IconName::Plus)
                            .label("New session")
                            .tooltip("Ctrl+N")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("search")
                            .w_full()
                            .ghost()
                            .small()
                            .icon(IconName::Search)
                            .label("Search / Command  Ctrl+K")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_palette(PaletteMode::Command, "", window, cx);
                            })),
                    )
                    .child(Input::new(&self.search).small()),
            )
            .child(
                div()
                    .id("session-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_1()
                    .child(list),
            )
            .child(
                h_flex()
                    .p_2()
                    .gap_1()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(
                        Button::new("open-ext")
                            .ghost()
                            .small()
                            .icon(IconName::LayoutDashboard)
                            .label("Extensions")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_page = Some(SettingsPage::Extensions);
                                this.load_extensions(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .label("Settings")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_page = Some(SettingsPage::General);
                                this.load_extensions(cx);
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let pane = self.active_pane().map(|p| p.read(cx));
        let (title, cwd, run) = pane.map_or_else(
            || ("Oino".to_owned(), String::new(), None),
            |p| {
                let run = match p.model.run {
                    oino_core::events::RunState::Idle => None,
                    oino_core::events::RunState::Running => Some(("running", theme.success)),
                    oino_core::events::RunState::Compacting => Some(("compacting", theme.info)),
                    oino_core::events::RunState::Retrying => Some(("retrying", theme.warning)),
                    oino_core::events::RunState::Waiting => Some(("waiting", theme.info)),
                };
                (p.model.title(), p.model.cwd.clone(), run)
            },
        );
        let attention = pane.map_or(0, |p| p.model.ui.dialog_count());
        h_flex()
            .h(px(40.))
            .px_3()
            .gap_3()
            .border_b_1()
            .border_color(theme.border)
            .when(!self.sidebar_open, |el| {
                el.child(
                    Button::new("show-sidebar")
                        .ghost()
                        .xsmall()
                        .icon(IconName::PanelLeftOpen)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_open = true;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .max_w(px(420.))
                    .child(SharedString::from(title)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(SharedString::from(cwd)),
            )
            .child(div().flex_1())
            .when_some(run, |el, (label, color)| {
                el.child(
                    div()
                        .px_2()
                        .rounded_full()
                        .text_xs()
                        .border_1()
                        .border_color(color.opacity(0.5))
                        .text_color(color)
                        .child(label),
                )
            })
            .when(attention > 0, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(format!("◆ {attention} waiting")),
                )
            })
            .child(
                Button::new("toggle-side")
                    .ghost()
                    .xsmall()
                    .icon(if self.side_open {
                        IconName::PanelRightClose
                    } else {
                        IconName::PanelRightOpen
                    })
                    .tooltip("Side pane (Ctrl+J)")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.side_open = !this.side_open;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let pane = self.active_pane().map(|p| p.read(cx));
        let mut left = h_flex().gap_3().flex_1().min_w_0().overflow_hidden();
        let mut right = h_flex().gap_3();
        if let Some(p) = pane {
            for status in p.model.ui.statuses().into_iter().take(8) {
                left = left.child(
                    div()
                        .whitespace_nowrap()
                        .text_color(tone_color(status.tone, cx))
                        .child(SharedString::from(status.text.clone())),
                );
            }
            for footer in p.model.ui.footers() {
                left = left.child(div().child(SharedString::from(footer.to_lines().join(" · "))));
            }
            if let Some(pct) = p.model.context_percent() {
                let color = match pct {
                    0..70 => theme.muted_foreground,
                    70..90 => theme.warning,
                    _ => theme.danger,
                };
                right = right.child(div().text_color(color).child(format!("ctx {pct}%")));
            }
            let u = p.model.usage;
            if u.input + u.output > 0 {
                right = right.child(format!("↑{} ↓{}", compact(u.input), compact(u.output)));
            }
            if u.cost > 0.0 {
                right = right.child(format!("${:.3}", u.cost));
            }
        }
        let running = self
            .panes
            .iter()
            .filter(|p| p.entity.read(cx).model.is_busy())
            .count();
        if running > 1 {
            right = right.child(
                div()
                    .text_color(theme.success)
                    .child(format!("{running} running")),
            );
        }
        right = right.child(SharedString::from(self.host.info().backend));
        h_flex()
            .h(px(24.))
            .px_3()
            .gap_3()
            .text_xs()
            .text_color(theme.muted_foreground)
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .child(left)
            .child(right)
            .into_any_element()
    }

    fn render_side_pane(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.side_open {
            return None;
        }
        let pane = self.active_pane()?.clone();
        let (has_side, panels) = {
            let p = pane.read(cx);
            (p.side.is_some(), p.side_panels())
        };
        if !has_side && panels.is_empty() {
            return None;
        }
        // One panel already shown: its header is enough.
        let panels = if panels.len() == 1 && has_side {
            Vec::new()
        } else {
            panels
        };
        let theme = cx.theme().clone();
        let show_tabs = !panels.is_empty();
        let tabs = h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .children(panels.into_iter().map(|(id, title)| {
                let pane = pane.clone();
                Button::new(SharedString::from(format!("sp-{id}")))
                    .ghost()
                    .xsmall()
                    .label(SharedString::from(title))
                    .on_click(move |_, _, cx| {
                        pane.update(cx, |p, cx| {
                            p.side = Some(SideTab::Panel(id.clone()));
                            cx.notify();
                        });
                    })
            }));
        let body = pane.update(cx, |p, cx| p.render_side(cx));
        Some(
            v_flex()
                .w(px(440.))
                .h_full()
                .flex_shrink_0()
                .border_l_1()
                .border_color(theme.border)
                .when(show_tabs, |el| el.child(tabs))
                .child(div().flex_1().min_h_0().children(body))
                .into_any_element(),
        )
    }

    fn settings_nav(
        &self,
        page: SettingsPage,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.settings_page == Some(page);
        Button::new(SharedString::from(format!("nav-{label}")))
            .w_full()
            .small()
            .label(label)
            .when(active, ButtonVariants::primary)
            .when(!active, ButtonVariants::ghost)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_page = Some(page);
                cx.notify();
            }))
            .into_any_element()
    }

    fn setting_row(label: &str, description: &str, control: AnyElement, cx: &App) -> AnyElement {
        h_flex()
            .w_full()
            .py_2()
            .gap_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_1()
                    .child(div().text_sm().child(SharedString::from(label.to_owned())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(description.to_owned())),
                    ),
            )
            .child(control)
            .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "one block per settings page")]
    fn render_settings(&self, page: SettingsPage, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let s = self.settings.clone();
        let body: AnyElement = match page {
            SettingsPage::General => v_flex()
                .child(Self::setting_row(
                    "Dark theme",
                    "Light/dark theme from gpui-kit tokens.",
                    Switch::new("set-dark")
                        .checked(s.theme == ThemePref::Dark)
                        .on_click(cx.listener(|this, on: &bool, window, cx| {
                            this.set_theme(if *on { ThemePref::Dark } else { ThemePref::Light }, window, cx);
                        }))
                        .into_any_element(),
                    cx,
                ))
                .child(Self::setting_row(
                    "Collapse thinking",
                    "Hide thinking blocks until clicked (hideThinkingBlock).",
                    Switch::new("set-think")
                        .checked(s.hide_thinking)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            let on = *on;
                            this.update_settings(|s| s.hide_thinking = on, cx);
                        }))
                        .into_any_element(),
                    cx,
                ))
                .child(Self::setting_row(
                    "Expand tool output",
                    "Show tool output under every tool row.",
                    Switch::new("set-tools")
                        .checked(s.expand_tools)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            let on = *on;
                            this.update_settings(|s| s.expand_tools = on, cx);
                        }))
                        .into_any_element(),
                    cx,
                ))
                .child(Self::setting_row(
                    "Compact rows",
                    "Denser transcript.",
                    Switch::new("set-compact")
                        .checked(s.compact)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            let on = *on;
                            this.update_settings(|s| s.compact = on, cx);
                        }))
                        .into_any_element(),
                    cx,
                ))
                .into_any_element(),
            SettingsPage::Models => {
                let active = self.active_pane().map(|p| p.read(cx));
                let models = active.map(|p| p.models.clone()).unwrap_or_default();
                v_flex()
                    .gap_2()
                    .child(div().text_sm().text_color(theme.muted_foreground).child(
                        "Providers are bundled WASM extensions. Log in with /login in the composer, or set API keys in ~/.oino/auth.json.",
                    ))
                    .child(
                        Button::new("login")
                            .outline()
                            .small()
                            .label("Run /login")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_page = None;
                                this.with_pane(cx, |p, cx| p.run_prompt("/login", cx));
                            })),
                    )
                    .child(div().pt_2().text_xs().text_color(theme.muted_foreground).child(format!("{} models available", models.len())))
                    .children(models.into_iter().map(|m| {
                        h_flex()
                            .gap_3()
                            .py_1()
                            .text_sm()
                            .child(div().flex_1().child(SharedString::from(if m.name.is_empty() { m.id.clone() } else { m.name.clone() })))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(SharedString::from(format!("{} · {}k", m.provider, m.context_window / 1000))))
                    }))
                    .into_any_element()
            }
            SettingsPage::Extensions => v_flex()
                .gap_1()
                .child(div().pb_2().text_sm().text_color(theme.muted_foreground).child(
                    "Bundled providers and unipi extensions ship inside Oino Desktop. Packages in ~/.oino/extensions load too.",
                ))
                .children(self.extensions.iter().map(|e| {
                    h_flex()
                        .gap_3()
                        .py_1()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(div().flex_1().text_sm().child(SharedString::from(e.id.clone())))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(SharedString::from(e.version.clone())))
                        .child(div().text_xs().w(px(72.)).child(SharedString::from(e.kind.clone())))
                        .child(div().text_xs().w(px(64.)).text_color(theme.muted_foreground).child(if e.bundled { "bundled" } else { "installed" }))
                        .child(match &e.error {
                            Some(err) => div().text_xs().text_color(theme.danger).child(SharedString::from(err.clone())),
                            None => div().text_xs().text_color(theme.success).child("loaded"),
                        })
                }))
                .into_any_element(),
            SettingsPage::Keybindings => v_flex()
                .gap_1()
                .children(
                    [
                        ("Ctrl+K", "Command palette (> commands, # sessions)"),
                        ("Ctrl+N", "New session"),
                        ("Ctrl+O", "New session in folder"),
                        ("Ctrl+L", "Select model"),
                        ("Ctrl+T", "Thinking level"),
                        ("Ctrl+B", "Toggle sidebar"),
                        ("Ctrl+J", "Toggle side pane"),
                        ("Ctrl+.", "Abort run (also Esc in the composer)"),
                        ("Ctrl+,", "Settings"),
                        ("Enter", "Send · steer while running"),
                        ("Ctrl+Enter", "Queue follow-up"),
                        ("Shift+Enter", "New line"),
                        ("↑ / ↓", "Prompt history (empty composer)"),
                        ("Tab", "Complete slash command"),
                    ]
                    .into_iter()
                    .map(|(k, d)| {
                        h_flex()
                            .gap_3()
                            .py_1()
                            .child(div().w(px(120.)).text_xs().font_family(theme.mono_font_family.clone()).child(k))
                            .child(div().text_sm().child(d))
                    }),
                )
                .into_any_element(),
            SettingsPage::About => {
                let info = self.host.info();
                v_flex()
                    .gap_1()
                    .text_sm()
                    .child(format!("Oino Desktop {}", info.version))
                    .child(format!("Agent dir: {}", info.agent_dir.display()))
                    .child(format!("Backend: {}", info.backend))
                    .into_any_element()
            }
        };
        div()
            .id("settings-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .child(
                h_flex()
                    .id("settings")
                    .w(px(860.))
                    .h(px(600.))
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .w(px(180.))
                            .h_full()
                            .p_2()
                            .gap_1()
                            .bg(theme.sidebar)
                            .border_r_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Settings"),
                            )
                            .child(self.settings_nav(SettingsPage::General, "General", cx))
                            .child(self.settings_nav(
                                SettingsPage::Models,
                                "Models & Providers",
                                cx,
                            ))
                            .child(self.settings_nav(SettingsPage::Extensions, "Extensions", cx))
                            .child(self.settings_nav(SettingsPage::Keybindings, "Keybindings", cx))
                            .child(self.settings_nav(SettingsPage::About, "About", cx)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .child(
                                h_flex().px_4().py_2().justify_end().child(
                                    Button::new("settings-close")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.settings_page = None;
                                            cx.notify();
                                        })),
                                ),
                            )
                            .child(
                                div()
                                    .id("settings-body")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .px_6()
                                    .pb_6()
                                    .child(body),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{}.{}k", n / 1000, (n % 1000) / 100),
        _ => format!("{}.{}M", n / 1_000_000, (n % 1_000_000) / 100_000),
    }
}

use gpui_kit::component::button::ButtonVariants;

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let sidebar = self.sidebar_open.then(|| self.render_sidebar(cx));
        let header = self.render_header(cx);
        let side = self.render_side_pane(cx);
        let status = self.render_status_bar(cx);
        let settings = self
            .settings_page
            .map(|page| self.render_settings(page, cx));
        let main: AnyElement = match self.active_pane() {
            Some(pane) => pane.clone().into_any_element(),
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child(if self.opening {
                    "Starting session…"
                } else {
                    "No session"
                })
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &NewSession, window, cx| this.new_session(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_open = !this.sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidePane, _, cx| {
                this.side_open = !this.side_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Abort, _, cx| this.with_pane(cx, SessionPane::abort)))
            .on_action(cx.listener(|this, _: &CommandPalette, window, cx| {
                this.open_palette(PaletteMode::Command, "", window, cx);
            }))
            .on_action(cx.listener(|this, _: &PickModel, window, cx| {
                this.open_palette(PaletteMode::Models, "", window, cx);
            }))
            .on_action(cx.listener(|this, _: &PickThinking, window, cx| {
                this.open_palette(PaletteMode::Thinking, "", window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.settings_page = Some(SettingsPage::General);
                this.load_extensions(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                this.run_app_action("toggle-theme", window, cx);
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .children(sidebar)
                    .child(
                        v_flex().flex_1().min_w_0().h_full().child(header).child(
                            h_flex()
                                .flex_1()
                                .min_h_0()
                                .w_full()
                                .child(div().flex_1().min_w_0().h_full().child(main))
                                .children(side),
                        ),
                    ),
            )
            .child(status)
            .children(settings)
            .when_some(self.palette.as_ref().map(|(p, _)| p.clone()), |el, p| {
                el.child(p)
            })
    }
}
