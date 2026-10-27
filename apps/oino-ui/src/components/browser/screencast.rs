//! The screencast Browser tab (browser-control.md §9.2, option b): the agent's and the user's
//! tabs of this session drawn from CDP screencast frames, with mouse / wheel / key / clipboard
//! forwarding, the take-over strip and dialog / handoff cards.
//!
//! Threading (stability.md): service calls that need tokio (`attach_view`, `frontend`) go
//! through [`Tokio`]; stream polling (runtime-agnostic channels) runs on GPUI; JPEG decoding
//! runs on the background executor. Replaced frames are dropped from the GPU atlas.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, CursorStyle, Entity,
    EventEmitter, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, KeyDownEvent,
    KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Pixels, Render, RenderImage, ScrollEvent, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, canvas, div, px,
};
use oino_extension_api::browser::BrowserHost;
use oino_extension_api::browser::protocol::{
    BrowserInput, BrowserScope, BrowserViewEvent, FrontendRequest, MouseButton as PageButton,
    MouseEventType, TabOwner,
};

use super::input::{self, CursorKind, KeyAction, Mods};
use super::model::{BrowserModel, Effect, Fit, Strip};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::empty::empty_state;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};
use crate::rt::Tokio;

/// Emitted to the wrapper.
#[derive(Clone, Debug)]
pub enum CastEvent {
    /// Title / URL changed.
    Changed,
}

/// The screencast view of one session.
pub struct ScreencastView {
    host: Arc<dyn BrowserHost>,
    workspace: String,
    /// View model.
    pub model: BrowserModel,
    url: Entity<InputState>,
    focus: FocusHandle,
    frame: Option<Arc<RenderImage>>,
    view_id: Option<u64>,
    view_task: Option<Task<()>>,
    session_task: Option<Task<()>>,
    area: Rc<Cell<Bounds<Pixels>>>,
    cursor: CursorStyle,
    last_move: Instant,
    last_probe: Instant,
    shown: bool,
    notice: Option<String>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<CastEvent> for ScreencastView {}

fn mods(m: gpui_kit::Modifiers) -> Mods {
    Mods {
        control: m.control,
        alt: m.alt,
        shift: m.shift,
        platform: m.platform,
    }
}

fn page_button(b: MouseButton) -> Option<PageButton> {
    match b {
        MouseButton::Left => Some(PageButton::Left),
        MouseButton::Right => Some(PageButton::Right),
        MouseButton::Middle => Some(PageButton::Middle),
        MouseButton::Navigate(_) => None,
    }
}

/// JPEG → BGRA GPU image.
fn decode(jpeg: &[u8]) -> Option<Arc<RenderImage>> {
    let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?;
    let mut rgba = img.to_rgba8();
    for px in rgba.pixels_mut() {
        px.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

impl ScreencastView {
    /// A view of `session`'s tabs; opens `url` as a user tab first when given.
    pub fn new(
        host: Arc<dyn BrowserHost>,
        session: &str,
        workspace: &str,
        url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search or enter an address"));
        let sub = cx.subscribe_in(&input, window, |this, input, e: &InputEvent, _, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                let typed = input.read(cx).value().to_string();
                this.goto(&super::normalise_url(&typed), cx);
            }
        });
        let release = cx.on_release(|this: &mut Self, cx: &mut App| {
            if let Some(img) = this.frame.take() {
                cx.drop_image(img, None);
            }
            if let Some(id) = this.view_id.take() {
                this.fire(FrontendRequest::ViewDetach { view_id: id }, cx);
            }
        });
        let mut this = Self {
            host,
            workspace: workspace.to_owned(),
            model: BrowserModel::new(session),
            url: input,
            focus: cx.focus_handle(),
            frame: None,
            view_id: None,
            view_task: None,
            session_task: None,
            area: Rc::new(Cell::new(Bounds::default())),
            cursor: CursorStyle::Arrow,
            last_move: Instant::now(),
            last_probe: Instant::now(),
            shown: true,
            notice: None,
            _subs: vec![sub, release],
        };
        this.start(url, window, cx);
        this
    }

    fn session(&self) -> String {
        self.model.session.clone()
    }

    fn start(&mut self, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let scope = BrowserScope::session(self.session(), self.workspace.clone());
        let status = self.host.status(Some(&scope));
        let reveal = super::hub::BrowserHub::take_reveal(cx, &self.model.session);
        let _ = self
            .model
            .apply(&BrowserViewEvent::Tabs { tabs: status.tabs });
        if let Some(tab) = reveal {
            let _ = self.model.select(&tab);
        }
        let mut events = self.host.subscribe(&self.model.session);
        self.session_task = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(e) = events.next().await {
                let alive = this.update_in(cx, |v, window, cx| v.on_event(&e, window, cx));
                if alive.is_err() {
                    break;
                }
            }
        }));
        match url.filter(|u| u != "about:blank") {
            Some(url) => self.open_user_tab(url, cx),
            None => {
                if let Some(tab) = self.model.current.clone() {
                    self.attach(&tab, cx);
                }
            }
        }
        self.sync_url(window, cx);
    }

    fn request(&self, req: FrontendRequest, cx: &App) -> Task<Result<serde_json::Value, String>> {
        let host = Arc::clone(&self.host);
        let session = self.session();
        let ws = self.workspace.clone();
        let task = Tokio::spawn(
            cx,
            async move { host.frontend(Some(&session), &ws, req).await },
        );
        cx.background_spawn(async move { task.await.unwrap_or_else(|e| Err(e.to_string())) })
    }

    fn fire(&self, req: FrontendRequest, cx: &App) {
        self.request(req, cx).detach();
    }

    fn input(&self, input: BrowserInput, cx: &App) {
        if let Some(tab) = self.model.current.clone() {
            self.fire(FrontendRequest::Input { tab, input }, cx);
        }
    }

    /// Opens `url` in a new user tab of this session and shows it.
    pub fn open_user_tab(&mut self, url: String, cx: &mut Context<Self>) {
        let task = self.request(FrontendRequest::TabOpen { url, reveal: true }, cx);
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |v, cx| match r {
                Ok(info) => {
                    if let Some(id) = info.get("id").and_then(|i| i.as_str()) {
                        v.show(id, cx);
                    }
                }
                Err(e) => {
                    v.notice = Some(e);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Navigates the shown tab (or opens a user tab when there is none).
    pub fn goto(&mut self, url: &str, cx: &mut Context<Self>) {
        if self.model.current.is_some() {
            self.input(
                BrowserInput::Goto {
                    url: url.to_owned(),
                },
                cx,
            );
        } else {
            self.open_user_tab(url.to_owned(), cx);
        }
    }

    fn show(&mut self, tab: &str, cx: &mut Context<Self>) {
        if let Effect::Show(t) = self.model.select(tab) {
            self.attach(&t, cx);
        }
        cx.emit(CastEvent::Changed);
        cx.notify();
    }

    fn detach(&mut self, cx: &mut Context<Self>) {
        self.view_task = None;
        if let Some(id) = self.view_id.take() {
            self.fire(FrontendRequest::ViewDetach { view_id: id }, cx);
        }
    }

    fn attach(&mut self, tab: &str, cx: &mut Context<Self>) {
        self.detach(cx);
        if !self.shown {
            return;
        }
        let host = Arc::clone(&self.host);
        let session = self.session();
        let tab = tab.to_owned();
        let attach = Tokio::spawn(
            cx,
            async move { host.attach_view(Some(&session), &tab, None) },
        );
        self.view_task = Some(cx.spawn(async move |this, cx| {
            let Ok(Ok(mut view)) = attach.await else {
                return;
            };
            let id = view.view_id;
            if this.update(cx, |v, _| v.view_id = Some(id)).is_err() {
                return;
            }
            while let Some(e) = view.events.next().await {
                if let BrowserViewEvent::Frame { jpeg, seq, .. } = &e {
                    let data = Arc::clone(&jpeg.0);
                    let seq = *seq;
                    let img = cx.background_spawn(async move { decode(&data) }).await;
                    let alive = this.update(cx, |v, cx| {
                        let _ = v.model.apply(&e);
                        if let Some(img) = img
                            && let Some(old) = v.frame.replace(img)
                        {
                            cx.drop_image(old, None);
                        }
                        v.fire(FrontendRequest::ViewAck { view_id: id, seq }, cx);
                        cx.notify();
                    });
                    if alive.is_err() {
                        break;
                    }
                }
            }
        }));
    }

    fn on_event(&mut self, e: &BrowserViewEvent, window: &mut Window, cx: &mut Context<Self>) {
        match self.model.apply(e) {
            Effect::Show(tab) | Effect::Reveal(tab) => self.attach(&tab, cx),
            Effect::Blank => {
                self.detach(cx);
                if let Some(old) = self.frame.take() {
                    cx.drop_image(old, None);
                }
            }
            Effect::Repaint => {}
        }
        if matches!(
            e,
            BrowserViewEvent::Page { .. }
                | BrowserViewEvent::Tabs { .. }
                | BrowserViewEvent::Reveal { .. }
                | BrowserViewEvent::Closed { .. }
        ) {
            self.sync_url(window, cx);
            cx.emit(CastEvent::Changed);
        }
        cx.notify();
    }

    fn sync_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.model.page.url.clone();
        let focused = self.url.read(cx).focus_handle(cx).is_focused(window);
        if !focused && self.url.read(cx).value() != url.as_str() {
            self.url.update(cx, |s, cx| s.set_value(url, window, cx));
        }
    }

    /// Shows / hides (hidden views stop their screencast).
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        if shown {
            if let Some(tab) = self.model.current.clone() {
                self.attach(&tab, cx);
            }
        } else {
            self.detach(cx);
        }
    }

    fn page_point(&self, p: gpui_kit::Point<Pixels>) -> Option<(f64, f64)> {
        let b = self.area.get();
        let css = self.model.frame_css;
        let fit = Fit::new(
            (
                f64::from(f32::from(b.origin.x)),
                f64::from(f32::from(b.origin.y)),
                f64::from(f32::from(b.size.width)),
                f64::from(f32::from(b.size.height)),
            ),
            css,
        );
        fit.to_page((f64::from(f32::from(p.x)), f64::from(f32::from(p.y))), css)
    }

    fn mouse(
        &mut self,
        event: MouseEventType,
        p: gpui_kit::Point<Pixels>,
        b: MouseButton,
        clicks: usize,
        m: gpui_kit::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let (Some((x, y)), Some(button)) = (self.page_point(p), page_button(b)) else {
            return;
        };
        self.input(
            BrowserInput::Mouse {
                event,
                x,
                y,
                button,
                click_count: u32::try_from(clicks).unwrap_or(1),
                modifiers: mods(m).bits(),
            },
            cx,
        );
    }

    fn on_move(&mut self, e: &MouseMoveEvent, cx: &mut Context<Self>) {
        let pressed = e.pressed_button.is_some();
        if !pressed && self.last_move.elapsed() < Duration::from_millis(33) {
            return;
        }
        self.last_move = Instant::now();
        let Some((x, y)) = self.page_point(e.position) else {
            return;
        };
        self.input(
            BrowserInput::Mouse {
                event: MouseEventType::Move,
                x,
                y,
                button: e.pressed_button.and_then(page_button).unwrap_or_default(),
                click_count: 0,
                modifiers: mods(e.modifiers).bits(),
            },
            cx,
        );
        if self.last_probe.elapsed() > Duration::from_millis(100)
            && let Some(tab) = self.model.current.clone()
        {
            self.last_probe = Instant::now();
            let task = self.request(FrontendRequest::Cursor { tab, x, y }, cx);
            cx.spawn(async move |this, cx| {
                if let Ok(v) = task.await {
                    let _ = this.update(cx, |v2, cx| {
                        let style = match input::cursor_kind(v.as_str().unwrap_or("default")) {
                            CursorKind::Pointer => CursorStyle::PointingHand,
                            CursorKind::Text => CursorStyle::IBeam,
                            CursorKind::Grab => CursorStyle::OpenHand,
                            CursorKind::Default => CursorStyle::Arrow,
                        };
                        if v2.cursor != style {
                            v2.cursor = style;
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
    }

    fn on_key(&mut self, e: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match input::key_down(&k.key, k.key_char.as_deref(), mods(k.modifiers)) {
            KeyAction::Send(events) => {
                for ev in events {
                    self.input(ev, cx);
                }
            }
            KeyAction::Paste => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.input(BrowserInput::ClipboardPaste { text }, cx);
                }
            }
            KeyAction::Copy { cut } => {
                if let Some(tab) = self.model.current.clone() {
                    let task = self.request(FrontendRequest::CopySelection { tab }, cx);
                    cx.spawn(async move |this, cx| {
                        if let Ok(v) = task.await
                            && let Some(text) = v.as_str().filter(|t| !t.is_empty())
                        {
                            let text = text.to_owned();
                            let _ = this.update(cx, |v, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                                if cut {
                                    v.input(
                                        BrowserInput::Key {
                                            event: oino_extension_api::browser::protocol::KeyEventType::Down,
                                            key: "Backspace".into(),
                                            text: None,
                                            modifiers: 0,
                                        },
                                        cx,
                                    );
                                }
                            });
                        }
                    })
                    .detach();
                }
            }
            KeyAction::Ignore => return,
        }
        cx.stop_propagation();
    }

    fn on_key_up(&mut self, e: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        self.input(
            input::key_up(&k.key, k.key_char.as_deref(), mods(k.modifiers)),
            cx,
        );
        cx.stop_propagation();
    }

    fn takeover(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(tab) = self.model.current.clone() {
            self.fire(FrontendRequest::Takeover { tab, on }, cx);
        }
    }

    fn tab_strip(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .id("br-tabs")
            .flex()
            .items_center()
            .gap(px(space::S1))
            .px(px(space::S2))
            .h(px(size::CONTROL_LG))
            .overflow_x_scroll()
            .border_b_1()
            .border_color(t.border());
        for tab in &self.model.tabs {
            let id = tab.id.clone();
            let active = self.model.current.as_deref() == Some(tab.id.as_str());
            let agent = matches!(tab.owner, TabOwner::Agent { .. });
            let label = BrowserModel::label(tab);
            let close_id = id.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("br-tab-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .px(px(space::S2))
                    .py(px(space::HALF))
                    .rounded(px(crate::design::radius::R_MD))
                    .max_w(px(180.))
                    .when(active, |el| el.bg(t.selected()))
                    .hover(|el| el.bg(t.hover()))
                    .cursor_pointer()
                    .text_role(t, TextRole::Sm)
                    .when(agent, |el| {
                        el.child(crate::design::icons::icon(IconName::Bot, 12., t.working()))
                    })
                    .child(div().truncate().child(label))
                    .child(
                        Btn::icon_only(
                            SharedString::from(format!("br-x-{id}")),
                            IconName::WindowClose,
                            "Close tab",
                        )
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.fire(
                                FrontendRequest::TabClose {
                                    tab: close_id.clone(),
                                },
                                cx,
                            );
                        })),
                    )
                    .on_click(cx.listener(move |v, _, _, cx| v.show(&id, cx))),
            );
        }
        row = row.child(
            Btn::icon_only("br-new", IconName::Plus, "New tab")
                .size(BtnSize::Sm)
                .on_click(cx.listener(|v, _, _, cx| v.open_user_tab("about:blank".into(), cx))),
        );
        row.into_any_element()
    }

    fn nav_bar(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let has = self.model.current.is_some();
        let hist = |id: &'static str,
                    glyph: IconName,
                    tip: &'static str,
                    action: &'static str,
                    enabled: bool| {
            Btn::icon_only(id, glyph, tip)
                .size(BtnSize::Sm)
                .disabled(!enabled)
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.input(
                        BrowserInput::History {
                            action: action.into(),
                        },
                        cx,
                    );
                }))
        };
        let external = self.model.page.url.clone();
        div()
            .flex()
            .items_center()
            .gap(px(space::S1))
            .h(px(size::CONTROL_LG + space::S1))
            .px(px(space::S2))
            .border_b_1()
            .border_color(t.border())
            .child(hist(
                "br-back",
                IconName::ArrowLeft,
                "Back",
                "back",
                has && self.model.page.can_back,
            ))
            .child(hist(
                "br-fwd",
                IconName::ArrowRight,
                "Forward",
                "forward",
                has && self.model.page.can_forward,
            ))
            .child(hist(
                "br-reload",
                IconName::RotateCw,
                "Reload",
                "reload",
                has,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_role(t, TextRole::Sm)
                    .child(Input::new(&self.url)),
            )
            .child(
                Btn::icon_only(
                    "br-ext",
                    IconName::ExternalLink,
                    "Open in the system browser",
                )
                .size(BtnSize::Sm)
                .disabled(external.is_empty() || external == "about:blank")
                .on_click(move |_, _, cx| cx.open_url(&external)),
            )
            .into_any_element()
    }

    fn strip(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (dot, text, action, on) = match self.model.strip() {
            Strip::None => return None,
            Strip::AgentDriving => (t.working(), "Agent is using this tab", "Take over", true),
            Strip::UserControl => (t.amber(), "You have control", "Give back", false),
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(space::S2))
                .px(px(space::S3))
                .py(px(space::S1))
                .bg(OinoTheme::tint(dot, 0.12))
                .border_b_1()
                .border_color(t.border())
                .text_role(t, TextRole::Sm)
                .child(div().size(px(8.)).rounded_full().bg(dot))
                .child(div().flex_1().child(text))
                .child(
                    Btn::new(if on { "br-takeover" } else { "br-giveback" })
                        .label(action)
                        .size(BtnSize::Sm)
                        .variant(Variant::Outline)
                        .on_click(cx.listener(move |v, _, _, cx| v.takeover(on, cx))),
                )
                .into_any_element(),
        )
    }

    fn cards(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let card = |title: String, body: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .m(px(space::S3))
                .p(px(space::S3))
                .rounded(px(crate::design::radius::R_CARD))
                .bg(t.card())
                .border_1()
                .border_color(t.card_border())
                .shadow(t.e3())
                .child(div().text_role(t, TextRole::Base).child(title))
                .child(
                    div()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(body),
                )
        };
        if let Some(message) = &self.model.handoff {
            let tab = self.model.current.clone().unwrap_or_default();
            let tab2 = tab.clone();
            return Some(
                card("Agent needs you".into(), message.clone())
                    .child(
                        div()
                            .flex()
                            .gap(px(space::S2))
                            .child(
                                Btn::new("br-handoff-done")
                                    .label("Done")
                                    .variant(Variant::Primary)
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |v, _, _, cx| {
                                        v.fire(
                                            FrontendRequest::HandoffDone {
                                                tab: tab.clone(),
                                                done: true,
                                            },
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Btn::new("br-handoff-cancel")
                                    .label("Cancel")
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |v, _, _, cx| {
                                        v.fire(
                                            FrontendRequest::HandoffDone {
                                                tab: tab2.clone(),
                                                done: false,
                                            },
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(d) = &self.model.dialog {
            let tab = self.model.current.clone().unwrap_or_default();
            let tab2 = tab.clone();
            let cancel = d.kind != "alert";
            return Some(
                card(format!("Page {}", d.kind), d.message.clone())
                    .child(
                        div()
                            .flex()
                            .gap(px(space::S2))
                            .child(
                                Btn::new("br-dialog-ok")
                                    .label("OK")
                                    .variant(Variant::Primary)
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |v, _, _, cx| {
                                        v.fire(
                                            FrontendRequest::DialogAnswer {
                                                tab: tab.clone(),
                                                accept: true,
                                                text: None,
                                            },
                                            cx,
                                        );
                                    })),
                            )
                            .when(cancel, |el| {
                                el.child(
                                    Btn::new("br-dialog-cancel")
                                        .label("Cancel")
                                        .size(BtnSize::Sm)
                                        .on_click(cx.listener(move |v, _, _, cx| {
                                            v.fire(
                                                FrontendRequest::DialogAnswer {
                                                    tab: tab2.clone(),
                                                    accept: false,
                                                    text: None,
                                                },
                                                cx,
                                            );
                                        })),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        None
    }

    fn canvas_area(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let area = Rc::clone(&self.area);
        let frame = self.frame.clone();
        let css = self.model.frame_css;
        let dim = self.model.page.loading;
        let bg = t.frame();
        div()
            .id("br-canvas")
            .flex_1()
            .min_h_0()
            .relative()
            .bg(bg)
            .track_focus(&self.focus)
            .key_context("BrowserCanvas")
            .cursor(self.cursor)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, e: &MouseDownEvent, window, cx| {
                    window.focus(&v.focus, cx);
                    v.mouse(
                        MouseEventType::Down,
                        e.position,
                        e.button,
                        e.click_count,
                        e.modifiers,
                        cx,
                    );
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|v, e: &MouseDownEvent, _, cx| {
                    v.mouse(
                        MouseEventType::Down,
                        e.position,
                        e.button,
                        e.click_count,
                        e.modifiers,
                        cx,
                    );
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, e: &MouseUpEvent, _, cx| {
                    v.mouse(
                        MouseEventType::Up,
                        e.position,
                        e.button,
                        e.click_count,
                        e.modifiers,
                        cx,
                    );
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|v, e: &MouseUpEvent, _, cx| {
                    v.mouse(
                        MouseEventType::Up,
                        e.position,
                        e.button,
                        e.click_count,
                        e.modifiers,
                        cx,
                    );
                }),
            )
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| v.on_move(e, cx)))
            .on_scroll(cx.listener(|v, e: &ScrollEvent, _, cx| {
                let d = e.delta.pixel_delta(px(16.));
                let Some((x, y)) = v.page_point(e.position) else {
                    return;
                };
                let scale = Fit::new(
                    (
                        0.0,
                        0.0,
                        f64::from(f32::from(v.area.get().size.width)),
                        f64::from(f32::from(v.area.get().size.height)),
                    ),
                    v.model.frame_css,
                )
                .scale;
                v.input(
                    BrowserInput::Wheel {
                        x,
                        y,
                        dx: -f64::from(f32::from(d.x)) / scale,
                        dy: -f64::from(f32::from(d.y)) / scale,
                        modifiers: mods(e.modifiers).bits(),
                    },
                    cx,
                );
            }))
            .child(
                canvas(
                    move |bounds, _, _| area.set(bounds),
                    move |bounds, (), window, _| {
                        let Some(img) = frame else { return };
                        let b = (
                            f64::from(f32::from(bounds.origin.x)),
                            f64::from(f32::from(bounds.origin.y)),
                            f64::from(f32::from(bounds.size.width)),
                            f64::from(f32::from(bounds.size.height)),
                        );
                        let fit = Fit::new(b, css);
                        #[allow(clippy::cast_possible_truncation, reason = "window pixels")]
                        let draw = Bounds::new(
                            gpui_kit::point(px(fit.x as f32), px(fit.y as f32)),
                            gpui_kit::size(
                                px((css.0 * fit.scale) as f32),
                                px((css.1 * fit.scale) as f32),
                            ),
                        );
                        let _ = window.paint_image(
                            draw,
                            draw,
                            gpui_kit::Corners::default(),
                            img,
                            0,
                            dim,
                        );
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

impl Render for ScreencastView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let _ = window;
        let body = if self.model.current.is_none() {
            let line = match self.model.closed.as_deref() {
                Some("idle") => {
                    "The browser stopped after being idle. Open a page to start it again."
                }
                Some("crash") => "The browser closed unexpectedly. Open a page to restart it.",
                _ => "Pages you or the agent open in this conversation appear here.",
            };
            div()
                .flex_1()
                .p(px(space::S4))
                .child(empty_state(
                    &t,
                    IconName::Globe,
                    "No browser tab",
                    Some(line.into()),
                    Vec::new(),
                ))
                .into_any_element()
        } else {
            self.canvas_area(&t, cx)
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.tab_strip(&t, cx))
            .child(self.nav_bar(&t, cx))
            .children(self.strip(&t, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(body)
                    .children(
                        self.cards(&t, cx)
                            .map(|c| div().absolute().bottom_0().left_0().right_0().child(c)),
                    ),
            )
            .when_some(self.notice.clone(), |el, n| {
                el.child(
                    div()
                        .px(px(space::S3))
                        .py(px(space::S1))
                        .text_role(&t, TextRole::Sm)
                        .text_color(t.danger())
                        .child(n),
                )
            })
            .when_some(self.model.download.clone(), |el, d| {
                el.child(
                    div()
                        .px(px(space::S3))
                        .py(px(space::S1))
                        .text_role(&t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(format!("Download: {d}")),
                )
            })
    }
}
