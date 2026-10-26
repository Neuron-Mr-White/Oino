//! [`MobileShell`]: the phone window root.
//!
//! One screen at a time (sessions → conversation, settings) plus at most one bottom sheet
//! (model + thinking, session actions) or full-screen side sheet (diff, tool output,
//! `sidePane` extension panels). Sessions keep running in the background when another screen
//! is shown, as on desktop.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, FocusHandle,
    Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px,
};
use oino_ai::ThinkingLevel;

use crate::design::theme;
use crate::host::{AppSettings, ExtensionInfo, OpenRequest, OpenedSession, SessionHost, ThemePref};
use crate::mobile::MobilePlatform;
use crate::model::sessions::{LiveStatus, SessionEntry, SessionList, SessionSummary, age};
use crate::model::ui_state::{NotifyKind, UiEffect};
use crate::rt::Tokio;
use crate::views::session_pane::{PaneEvent, SessionPane, SideTab};

/// Height of the app bar.
const BAR: f32 = 52.;
/// Minimum touch target.
const TOUCH: f32 = 44.;

const LEVELS: [ThinkingLevel; 6] = [
    ThinkingLevel::Off,
    ThinkingLevel::Minimal,
    ThinkingLevel::Low,
    ThinkingLevel::Medium,
    ThinkingLevel::High,
    ThinkingLevel::Xhigh,
];

/// The visible screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// Session list (home).
    Sessions,
    /// One session's conversation (pane key).
    Session(u64),
    /// Settings.
    Settings,
}

/// A bottom sheet over the conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sheet {
    /// Model list + thinking level.
    Model,
    /// Session actions (new, compact, panels).
    Actions,
}

struct Pane {
    key: u64,
    entity: Entity<SessionPane>,
    _subscriptions: Vec<Subscription>,
}

/// The phone window root view.
pub struct MobileShell {
    host: Arc<dyn SessionHost>,
    settings: AppSettings,
    sessions: SessionList,
    panes: Vec<Pane>,
    next_key: u64,
    screen: Screen,
    sheet: Option<Sheet>,
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

impl MobileShell {
    /// The root view; starts on the session list.
    pub fn new(host: Arc<dyn SessionHost>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = host.app_settings();
        theme::install(&settings, Some(window), cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions"));
        let subscriptions = vec![cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, input, event, _, cx| match event {
                InputEvent::Change => {
                    let query = input.read(cx).value().to_string();
                    this.sessions.set_query(&query);
                    cx.notify();
                }
                InputEvent::Focus => (MobilePlatform::get(cx).show_keyboard)(),
                InputEvent::Blur => (MobilePlatform::get(cx).hide_keyboard)(),
                InputEvent::PressEnter { .. } => {}
            },
        )];
        let shell = Self {
            host,
            settings,
            sessions: SessionList::new(),
            panes: Vec::new(),
            next_key: 0,
            screen: Screen::Sessions,
            sheet: None,
            extensions: Vec::new(),
            search,
            opening: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        shell.refresh_sessions(cx);
        shell
    }

    fn pane(&self, key: u64) -> Option<&Entity<SessionPane>> {
        self.panes.iter().find(|p| p.key == key).map(|p| &p.entity)
    }

    fn active_pane(&self) -> Option<&Entity<SessionPane>> {
        match self.screen {
            Screen::Session(key) => self.pane(key),
            Screen::Sessions | Screen::Settings => None,
        }
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

    // ── sessions ─────────────────────────────────────────────────────────────

    fn refresh_sessions(&self, cx: &mut Context<Self>) {
        let task = Tokio::spawn(cx, self.host.list_sessions());
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(list)) = task.await {
                let _ = this.update(cx, |shell, cx| {
                    shell.sessions.set_sessions(list);
                    shell.sync_open_sessions(cx);
                    cx.notify();
                });
            }
        })
        .detach();
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
            self.sessions.set_live(&model.id, pane.live());
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
        // The composer owns the pane's focus handle: raise / drop the soft keyboard with it.
        let focus = entity.read(cx).focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&entity, window, move |this, pane, event, window, cx| {
                this.on_pane_event(key, pane, event, window, cx);
            }),
            cx.on_focus_in(&focus, window, |_, _, cx| {
                (MobilePlatform::get(cx).show_keyboard)();
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
    }

    fn show_session(&mut self, key: u64, cx: &mut Context<Self>) {
        self.screen = Screen::Session(key);
        self.sheet = None;
        if let Some(pane) = self.pane(key) {
            let id = pane.read(cx).model.id.clone();
            if self.sessions.live(&id) == LiveStatus::Unread {
                self.sessions.set_live(&id, LiveStatus::Idle);
            }
        }
        cx.notify();
    }

    fn open_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self
            .panes
            .iter()
            .find(|p| p.entity.read(cx).model.id == id)
            .map(|p| p.key)
        {
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

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.active_pane().map_or_else(
            || self.host.info().default_cwd,
            |p| PathBuf::from(p.read(cx).model.cwd.clone()),
        );
        self.open(OpenRequest::New { cwd }, window, cx);
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
                let visible = self.screen == Screen::Session(key);
                let status = if *status == LiveStatus::Idle
                    && self.sessions.live(&id) == LiveStatus::Running
                    && !visible
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
            PaneEvent::PickModel => {
                self.sheet = Some(Sheet::Model);
                cx.notify();
            }
            PaneEvent::Effect(effect) => self.perform(effect.clone(), window, cx),
            PaneEvent::Toast(text) => {
                window.push_notification((NotificationType::Info, text.clone()), cx);
            }
            PaneEvent::ShowSide
            | PaneEvent::ShowBottom
            | PaneEvent::Focused
            | PaneEvent::ToggleSidebar
            | PaneEvent::ToggleSide
            | PaneEvent::ToggleBottom
            | PaneEvent::ClosePane => cx.notify(),
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
            UiEffect::OpenSettings { .. } => self.open_settings(cx),
            _ => {}
        }
    }

    // ── settings ─────────────────────────────────────────────────────────────

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.screen = Screen::Settings;
        self.sheet = None;
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

    fn set_theme(&mut self, pref: ThemePref, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme = pref;
        theme::install(&self.settings, Some(window), cx);
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

    // ── navigation ───────────────────────────────────────────────────────────

    /// Back: sheet → side sheet → screen. Returns whether anything was popped.
    fn back(&mut self, cx: &mut Context<Self>) -> bool {
        if self.sheet.take().is_some() {
            cx.notify();
            return true;
        }
        if let Some(pane) = self.active_pane().cloned()
            && pane.read(cx).side.is_open()
        {
            pane.update(cx, |p, cx| {
                p.side.close();
                cx.notify();
            });
            return true;
        }
        if self.screen == Screen::Sessions {
            return false;
        }
        self.screen = Screen::Sessions;
        self.sync_open_sessions(cx);
        self.refresh_sessions(cx);
        cx.notify();
        true
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.back(cx) {
            window.blur(cx);
            cx.stop_propagation();
        }
    }

    // ── rendering ────────────────────────────────────────────────────────────

    fn icon_button(id: &'static str, icon: IconName) -> Button {
        Button::new(id)
            .ghost()
            .icon(icon)
            .min_w(px(TOUCH))
            .h(px(TOUCH))
    }

    fn app_bar(
        leading: Option<Button>,
        title: AnyElement,
        actions: Vec<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(BAR))
            .flex_none()
            .px_1()
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .children(leading)
            .child(div().flex_1().min_w_0().px_2().child(title))
            .children(actions)
            .into_any_element()
    }

    fn status_dot(status: LiveStatus, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let color = match status {
            LiveStatus::Idle => theme.muted_foreground.opacity(0.35),
            LiveStatus::Running => theme.success,
            LiveStatus::Attention => theme.warning,
            LiveStatus::Unread => theme.info,
        };
        div()
            .w(px(10.))
            .h(px(10.))
            .flex_none()
            .rounded_full()
            .bg(color)
            .into_any_element()
    }

    fn session_row(entry: &SessionEntry, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let id = entry.summary.id.clone();
        let detail = format!(
            "{} · {} messages",
            age(now_ms(), entry.summary.modified_ms),
            entry.summary.message_count
        );
        h_flex()
            .id(SharedString::from(format!("m-s-{}", entry.summary.id)))
            .w_full()
            .min_h(px(60.))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .active(|s| s.bg(theme.accent))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_session(&id, window, cx);
            }))
            .child(Self::status_dot(entry.status, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(
                        SharedString::from(if entry.summary.title.is_empty() {
                            "Untitled".to_owned()
                        } else {
                            entry.summary.title.clone()
                        }),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(SharedString::from(detail)),
                    ),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(IconName::ChevronRight),
            )
            .into_any_element()
    }

    fn render_sessions(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let title = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Oino"),
            )
            .when(self.opening, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("starting…"),
                )
            })
            .into_any_element();
        let bar = Self::app_bar(
            None,
            title,
            vec![
                Self::icon_button("m-settings", IconName::Settings)
                    .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx)))
                    .into_any_element(),
            ],
            cx,
        );
        let mut list = v_flex().w_full();
        let pinned = self.sessions.pinned();
        if !pinned.is_empty() {
            list = list.child(Self::section("Pinned", cx));
            for entry in &pinned {
                list = list.child(Self::session_row(entry, cx));
            }
        }
        for group in self.sessions.groups() {
            list = list.child(Self::section(&group.label, cx));
            for entry in &group.sessions {
                list = list.child(Self::session_row(entry, cx));
            }
        }
        if self.sessions.is_empty() {
            list = list.child(
                v_flex()
                    .w_full()
                    .py_12()
                    .gap_2()
                    .items_center()
                    .text_color(theme.muted_foreground)
                    .child("No sessions yet")
                    .child(div().text_xs().child("Tap “New session” to start.")),
            );
        }
        let new_session = div().w_full().px_4().py_3().flex_none().child(
            Button::new("m-new")
                .primary()
                .w_full()
                .h(px(TOUCH))
                .icon(IconName::Plus)
                .label("New session")
                .loading(self.opening)
                .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
        );
        v_flex()
            .size_full()
            .child(bar)
            .child(
                div()
                    .px_4()
                    .pt_3()
                    .flex_none()
                    .child(Input::new(&self.search).cleanable(true)),
            )
            .child(
                div()
                    .id("m-sessions")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(list),
            )
            .child(new_session)
            .into_any_element()
    }

    fn section(label: &str, cx: &App) -> AnyElement {
        div()
            .w_full()
            .px_4()
            .pt_4()
            .pb_1()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .child(SharedString::from(label.to_uppercase()))
            .into_any_element()
    }

    fn render_session(
        pane: &Entity<SessionPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let (title, model_label, thinking, busy, side) = {
            let p = pane.read(cx);
            (
                p.model.title(),
                p.model.model_label(),
                p.model.thinking,
                p.model.is_busy(),
                p.side.is_open() && !p.model.ui.needs_attention(),
            )
        };
        let heading = v_flex()
            .id("m-title")
            .min_w_0()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.sheet = Some(Sheet::Model);
                cx.notify();
            }))
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(title)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .truncate()
                            .child(SharedString::from(format!("{model_label} · {thinking}"))),
                    )
                    .child(IconName::ChevronDown),
            )
            .into_any_element();
        let mut actions = Vec::new();
        if busy {
            actions.push(
                Self::icon_button("m-stop", IconName::Square)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.with_pane(cx, SessionPane::abort);
                    }))
                    .into_any_element(),
            );
        }
        actions.push(
            Self::icon_button("m-more", IconName::EllipsisVertical)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sheet = Some(Sheet::Actions);
                    cx.notify();
                }))
                .into_any_element(),
        );
        let back = Self::icon_button("m-back", IconName::ChevronLeft).on_click(cx.listener(
            |this, _, window, cx| {
                window.blur(cx);
                this.screen = Screen::Sessions;
                this.sheet = None;
                this.sync_open_sessions(cx);
                this.refresh_sessions(cx);
                cx.notify();
            },
        ));
        let bar = Self::app_bar(Some(back), heading, actions, cx);
        let side_sheet = if side {
            Some(pane.update(cx, |p, cx| p.render_side(window, cx)))
        } else {
            None
        };
        // The side sheet covers the conversation (not the app bar) and swallows its input.
        // It steps aside while a dialog waits, since the pane renders dialogs beneath it.
        v_flex()
            .size_full()
            .child(bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .relative()
                    .child(pane.clone())
                    .when_some(side_sheet, |el, side| {
                        el.child(
                            div()
                                .id("m-side-sheet")
                                .absolute()
                                .inset_0()
                                .occlude()
                                .bg(theme.background)
                                .child(side),
                        )
                    }),
            )
            .into_any_element()
    }

    fn setting_row(label: &str, detail: &str, control: AnyElement, cx: &App) -> AnyElement {
        h_flex()
            .w_full()
            .min_h(px(56.))
            .px_4()
            .py_2()
            .gap_4()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.5))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().child(SharedString::from(label.to_owned())))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(detail.to_owned())),
                    ),
            )
            .child(control)
            .into_any_element()
    }

    fn settings_appearance(s: &AppSettings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            Self::setting_row(
                "Dark theme",
                "Follow the Oino dark palette",
                Switch::new("m-dark")
                    .checked(s.theme == ThemePref::Dark)
                    .on_click(cx.listener(|this, on: &bool, window, cx| {
                        let pref = if *on {
                            ThemePref::Dark
                        } else {
                            ThemePref::Light
                        };
                        this.set_theme(pref, window, cx);
                    }))
                    .into_any_element(),
                cx,
            ),
            Self::setting_row(
                "Collapse thinking",
                "Hide reasoning blocks until tapped",
                Switch::new("m-think")
                    .checked(s.hide_thinking)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.update_settings(|s| s.hide_thinking = on, cx);
                    }))
                    .into_any_element(),
                cx,
            ),
            Self::setting_row(
                "Expand tool rows",
                "Show tool output inline by default",
                Switch::new("m-tools")
                    .checked(s.expand_tools)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.update_settings(|s| s.expand_tools = on, cx);
                    }))
                    .into_any_element(),
                cx,
            ),
        ]
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let s = self.settings.clone();
        let info = self.host.info();
        let back = Self::icon_button("m-set-back", IconName::ChevronLeft).on_click(cx.listener(
            |this, _, _, cx| {
                this.back(cx);
            },
        ));
        let bar = Self::app_bar(
            Some(back),
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Settings")
                .into_any_element(),
            Vec::new(),
            cx,
        );
        let appearance = Self::settings_appearance(&s, cx);
        let extensions: Vec<AnyElement> = self
            .extensions
            .iter()
            .map(|e| {
                let detail = e.error.clone().unwrap_or_else(|| {
                    format!(
                        "{} · {}{}",
                        e.version,
                        e.kind,
                        if e.bundled { " · bundled" } else { "" }
                    )
                });
                let status = div()
                    .text_xs()
                    .text_color(if e.error.is_some() {
                        theme.danger
                    } else {
                        theme.success
                    })
                    .child(if e.error.is_some() {
                        "failed"
                    } else {
                        "loaded"
                    })
                    .into_any_element();
                Self::setting_row(&e.id, &detail, status, cx)
            })
            .collect();
        let about = vec![
            Self::setting_row("Version", &info.version, div().into_any_element(), cx),
            Self::setting_row("Backend", &info.backend, div().into_any_element(), cx),
            Self::setting_row(
                "Agent directory",
                &info.agent_dir.display().to_string(),
                div().into_any_element(),
                cx,
            ),
        ];
        v_flex()
            .size_full()
            .child(bar)
            .child(
                div()
                    .id("m-settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .w_full()
                            .pb_6()
                            .child(Self::section("Appearance", cx))
                            .children(appearance)
                            .child(Self::section("Extensions", cx))
                            .when(extensions.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px_4()
                                        .py_2()
                                        .text_sm()
                                        .text_color(theme.muted_foreground)
                                        .child("Open a session to load extensions."),
                                )
                            })
                            .children(extensions)
                            .child(Self::section("About", cx))
                            .children(about),
                    ),
            )
            .into_any_element()
    }

    fn sheet_frame(
        id: &'static str,
        title: &str,
        body: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        div()
            .id("m-sheet-scrim")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .justify_end()
            .bg(crate::design::OinoTheme::get(cx).scrim())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.sheet = None;
                cx.notify();
            }))
            .child(
                v_flex()
                    .id(id)
                    .w_full()
                    .max_h(gpui_kit::relative(0.8))
                    .rounded_t_xl()
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_lg()
                    // Taps inside the sheet must not reach the scrim.
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex().w_full().justify_center().pt_2().child(
                            div()
                                .w(px(36.))
                                .h(px(4.))
                                .rounded_full()
                                .bg(theme.muted_foreground.opacity(0.4)),
                        ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_2()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(title.to_owned())),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-body")))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .pb_4()
                            .child(body),
                    ),
            )
            .into_any_element()
    }

    fn sheet_row(
        id: SharedString,
        label: String,
        detail: Option<String>,
        current: bool,
        cx: &App,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let theme = cx.theme();
        h_flex()
            .id(id)
            .w_full()
            .min_h(px(TOUCH + 4.))
            .px_4()
            .gap_3()
            .items_center()
            .active(|s| s.bg(theme.accent))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(SharedString::from(label)))
                    .when_some(detail, |el, d| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(SharedString::from(d)),
                        )
                    }),
            )
            .when(current, |el| {
                el.child(div().text_color(theme.primary).child(IconName::Check))
            })
    }

    fn render_model_sheet(pane: &Entity<SessionPane>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (models, current, thinking) = {
            let p = pane.read(cx);
            (p.models.clone(), p.model.model.clone(), p.model.thinking)
        };
        let chips = h_flex()
            .px_4()
            .py_2()
            .gap_1()
            .flex_wrap()
            .children(LEVELS.iter().map(|l| {
                let level = *l;
                let selected = level == thinking;
                Button::new(SharedString::from(format!("m-think-{level}")))
                    .small()
                    .when(selected, ButtonVariants::primary)
                    .when(!selected, Button::outline)
                    .label(SharedString::from(level.to_string()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.with_pane(cx, |p, cx| p.set_thinking(level, cx));
                    }))
            }));
        let rows: Vec<AnyElement> = models
            .into_iter()
            .map(|m| {
                let is_current = current
                    .as_ref()
                    .is_some_and(|c| c.id == m.id && c.provider == m.provider);
                let label = if m.name.is_empty() {
                    m.id.clone()
                } else {
                    m.name.clone()
                };
                let detail = format!("{} · {}k context", m.provider, m.context_window / 1000);
                let (provider, id) = (m.provider.clone(), m.id.clone());
                Self::sheet_row(
                    SharedString::from(format!("m-model-{provider}-{id}")),
                    label,
                    Some(detail),
                    is_current,
                    cx,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let (provider, id) = (provider.clone(), id.clone());
                    this.with_pane(cx, |p, cx| p.set_model(provider, id, cx));
                    this.sheet = None;
                    cx.notify();
                }))
                .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        let body = v_flex()
            .w_full()
            .child(Self::section("Thinking", cx))
            .child(chips)
            .child(Self::section("Model", cx))
            .when(empty, |el| {
                el.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("No models with credentials. Send /login to add a provider."),
                )
            })
            .children(rows)
            .into_any_element();
        Self::sheet_frame("m-model-sheet", "Model", body, cx)
    }

    fn render_actions_sheet(pane: &Entity<SessionPane>, cx: &mut Context<Self>) -> AnyElement {
        let panels = pane.read(cx).side_panels();
        let mut rows: Vec<AnyElement> = vec![
            Self::sheet_row("m-act-new".into(), "New session".into(), None, false, cx)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.sheet = None;
                    this.new_session(window, cx);
                }))
                .into_any_element(),
            Self::sheet_row(
                "m-act-compact".into(),
                "Compact context".into(),
                Some("Summarise older turns".into()),
                false,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.sheet = None;
                this.with_pane(cx, SessionPane::compact);
            }))
            .into_any_element(),
            Self::sheet_row(
                "m-act-login".into(),
                "Log in to a provider".into(),
                Some("/login".into()),
                false,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.sheet = None;
                this.with_pane(cx, |p, cx| p.run_prompt("/login", cx));
            }))
            .into_any_element(),
            Self::sheet_row("m-act-settings".into(), "Settings".into(), None, false, cx)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_settings(cx)))
                .into_any_element(),
        ];
        if !panels.is_empty() {
            rows.push(Self::section("Panels", cx));
            for (id, title) in panels {
                let panel = id.clone();
                rows.push(
                    Self::sheet_row(
                        SharedString::from(format!("m-panel-{id}")),
                        title,
                        None,
                        false,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.sheet = None;
                        let panel = panel.clone();
                        this.with_pane(cx, |p, cx| {
                            p.side.open(SideTab::Panel(panel));
                            cx.notify();
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
                );
            }
        }
        let body = v_flex().w_full().children(rows).into_any_element();
        Self::sheet_frame("m-actions-sheet", "Session", body, cx)
    }
}

impl Render for MobileShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let hooks = MobilePlatform::get(cx);
        let insets = (hooks.safe_area)();
        let keyboard = (hooks.keyboard_height)();
        let bottom = insets.bottom.max(keyboard);
        let pane = self.active_pane().cloned();
        let body = match (self.screen, pane.as_ref()) {
            (Screen::Session(_), Some(pane)) => Self::render_session(pane, window, cx),
            (Screen::Settings, _) => self.render_settings(cx),
            _ => self.render_sessions(cx),
        };
        let sheet = match (self.sheet, pane.as_ref()) {
            (Some(Sheet::Model), Some(pane)) => Some(Self::render_model_sheet(pane, cx)),
            (Some(Sheet::Actions), Some(pane)) => Some(Self::render_actions_sheet(pane, cx)),
            _ => None,
        };
        v_flex()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .pt(px(insets.top))
            .pb(px(bottom))
            .pl(px(insets.left))
            .pr(px(insets.right))
            // Sheets are absolutely positioned, which ignores the root's padding: keep them
            // inside a box that already excludes the safe areas and the keyboard.
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(body)
                    .children(sheet),
            )
    }
}
