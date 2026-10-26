//! `WorkspaceHeader` (design.md §4.8): title (click to rename), project chip, branch chip,
//! `⋯` menu, run capsule, Unicrab when the session needs you, frame toggles.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use oino_client::PiCommand;

use crate::components::badge::{Capsule, capsule};
use crate::components::button::{Btn, BtnSize};
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::unicrab::unicrab;
use crate::design::brand::CrabState;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::sessions::workspace_label;
use crate::views::session_pane::{PaneEvent, PaneMenu, SessionPane, SideTab};

/// Shell state the header reflects.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent visual flags of one element"
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HeaderState {
    /// Sidebar collapsed (show the re-open button).
    pub sidebar_collapsed: bool,
    /// Side frame open.
    pub side_open: bool,
    /// Bottom frame open.
    pub bottom_open: bool,
    /// Top-most pane of a split (drag region, frame toggles).
    pub primary: bool,
    /// One of several split panes (shows a close button).
    pub closable: bool,
    /// Client-side chrome: this header carries the window controls (right-most frame).
    pub controls: bool,
    /// Client-side chrome: the side frame's tab bar carries the window controls.
    pub controls_side: bool,
    /// The window is maximized (restore icon).
    pub maximized: bool,
    /// Client-side chrome: headers are window drag regions.
    pub drag: bool,
}

impl SessionPane {
    pub(crate) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.model.title();
        let state =
            cx.new(|cx| InputState::new(window, cx).default_value(SharedString::from(title)));
        state.update(cx, |s, cx| s.focus(window, cx));
        cx.subscribe(&state, |this, input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => {
                let name = input.read(cx).value().trim().to_owned();
                this.rename = None;
                if !name.is_empty() {
                    this.send(PiCommand::SetSessionName { name }.into(), cx);
                }
                cx.notify();
            }
            InputEvent::Blur => {
                this.rename = None;
                cx.notify();
            }
            _ => {}
        })
        .detach();
        self.rename = Some(state);
        cx.notify();
    }

    /// The header.
    #[allow(clippy::too_many_lines, reason = "header layout with its menus")]
    pub(crate) fn render_header(
        &self,
        state: HeaderState,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self.model.title();
        let compact = self.width < px(crate::model::composer::COMPRESS_BELOW + size::SIDEBAR);
        let project = workspace_label(&self.model.cwd);
        let branch = self.status.branch.clone();
        let close = {
            let e = cx.entity().downgrade();
            move |_: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |p, cx| {
                    p.menu = None;
                    cx.notify();
                });
            }
        };
        let e = cx.entity().downgrade();
        let id = self.model.id.clone();
        let file = self.model.file.clone();
        let more = popover(
            Btn::icon_only("hdr-more", IconName::Ellipsis, "More")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::More, cx))),
            self.menu == Some(PaneMenu::More),
            Placement::BelowStart,
            || {
                let act =
                    |label: &'static str,
                     glyph: IconName,
                     f: fn(&mut SessionPane, &mut Context<SessionPane>)| {
                        let e = e.clone();
                        MenuEntry::item(label, move |_, cx| {
                            let _ = e.update(cx, |p, cx| {
                                p.menu = None;
                                f(p, cx);
                            });
                        })
                        .icon(glyph)
                    };
                let id = id.clone();
                let file = file.clone();
                menu(
                    "header-more",
                    t,
                    size::MENU_MIN * 1.2,
                    vec![
                        MenuEntry::item("Copy session id", move |_, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(id.clone()));
                        })
                        .icon(IconName::Copy),
                        MenuEntry::item("Copy file path", move |_, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                file.clone().unwrap_or_default(),
                            ));
                        })
                        .icon(IconName::FileText),
                        act(
                            "Export HTML",
                            IconName::ExternalLink,
                            SessionPane::export_html,
                        ),
                        MenuEntry::Separator,
                        act("Fork", IconName::GitBranch, SessionPane::fork_session),
                        act("Tree view", IconName::ListTree, |p, cx| {
                            p.open_side(SideTab::Tree, cx);
                        })
                        .keys("⌘⇧T"),
                        act("Compact now", IconName::Archive, SessionPane::compact),
                    ],
                )
                .into_any_element()
            },
            close,
        );
        let run = if self.model.ui.needs_attention() {
            Some(Capsule::Waiting)
        } else {
            match self.model.run {
                oino_core::events::RunState::Running => Some(Capsule::Running),
                oino_core::events::RunState::Compacting => Some(Capsule::Compacting),
                oino_core::events::RunState::Retrying => Some(Capsule::Retrying),
                _ => None,
            }
        };
        let title_el: AnyElement = match &self.rename {
            Some(input) => div()
                .w(px(size::SIDEBAR))
                .child(Input::new(input))
                .into_any_element(),
            None => div()
                .id("hdr-title")
                .min_w(px(size::CONTROL_LG * 2.0))
                .flex_shrink(1.0)
                .truncate()
                .px(px(space::S1))
                .rounded(px(radius::R_MD))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover()))
                .text_role(t, TextRole::Base)
                .weight_medium()
                .text_color(if self.focused_pane {
                    t.fg()
                } else {
                    t.fg_subtle()
                })
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.start_rename(window, cx)),
                )
                .child(SharedString::from(title))
                .into_any_element(),
        };
        let bar = div()
            .id("pane-header")
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(size::HEADER))
            .px(px(space::S3))
            .gap(px(space::S1))
            .border_b_1()
            .border_color(t.border());
        let bar = if state.drag {
            crate::components::window_chrome::drag_region(bar, std::rc::Rc::clone(&self.drag_armed))
        } else {
            bar
        };
        bar.when(state.sidebar_collapsed && state.primary, |el| {
            el.child(
                Btn::icon_only("hdr-sidebar", IconName::PanelLeft, "Show sidebar")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleSidebar))),
            )
        })
        .child(title_el)
        .when(!compact, |el| {
            el.child(
                Btn::new("hdr-project")
                    .label(SharedString::from(project))
                    .icon(IconName::Folder)
                    .size(BtnSize::Sm)
                    .tooltip(SharedString::from(self.model.cwd.clone()), None),
            )
        })
        .when_some(branch.filter(|_| !compact), |el, b| {
            el.child(
                Btn::new("hdr-branch")
                    .label(SharedString::from(b))
                    .icon(IconName::GitBranch)
                    .size(BtnSize::Sm),
            )
        })
        .child(more)
        .child(div().flex_1())
        .when_some(run, |el, k| el.child(capsule(k, cx)))
        .when(self.model.ui.needs_attention(), |el| {
            el.child(unicrab(CrabState::NeedsYou, size::CRAB.0, cx))
        })
        .when(state.primary, |el| {
            el.child(
                Btn::icon_only("hdr-bottom", IconName::PanelBottom, "Bottom frame")
                    .selected(state.bottom_open)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleBottom))),
            )
            .child(
                Btn::icon_only("hdr-side", IconName::PanelRight, "Side pane")
                    .selected(state.side_open)
                    .tooltip("Side pane", Some("⌘J"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleSide))),
            )
        })
        .when(state.closable, |el| {
            el.child(
                Btn::icon_only("hdr-close-pane", IconName::X, "Close pane")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ClosePane))),
            )
        })
        .when(state.controls, |el| {
            el.child(crate::components::window_chrome::window_controls(
                t,
                state.maximized,
            ))
        })
        .into_any_element()
    }
}
