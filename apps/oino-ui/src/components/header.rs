//! `DetailHeader` (design.md A2.7): the 56 px header on top of the visible session. Back
//! (closes the detail; Esc does the same), title 16/600 (click to rename; with the list
//! folded it opens the switch sheet) over a sub-line with the state dot, project and branch;
//! the session strip pill (model · context · cost, opens the model menu), the run capsule,
//! `⋮` menu, browser / bottom / side toggles, the header crab while the list is folded, and
//! the window controls with client-side chrome. Bottom rule `line_soft`.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px, relative,
};
use oino_client::PiCommand;

use crate::components::badge::{Capsule, capsule};
use crate::components::button::Btn;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::sessions::{LiveStatus, project_label};
use crate::views::session_pane::{PaneEvent, PaneMenu, SessionPane, SideTab};

/// Shell state the header reflects.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent visual flags of one element"
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HeaderState {
    /// Sidebar hidden (show the re-open button).
    pub sidebar_collapsed: bool,
    /// Side frame open.
    pub side_open: bool,
    /// Bottom frame open.
    pub bottom_open: bool,
    /// The visible session's header (drag region, frame toggles).
    pub primary: bool,
    /// The session list column is folded: the title opens the switch sheet and the header
    /// carries the crab.
    pub list_folded: bool,
    /// Sessions that need the user (header crab badge).
    pub needs: usize,
    /// Client-side chrome: this header carries the window controls.
    pub controls: bool,
    /// Client-side chrome: the side frame carries the window controls.
    pub controls_side: bool,
    /// The window is maximized (restore icon).
    pub maximized: bool,
    /// Client-side chrome: headers are window drag regions.
    pub drag: bool,
}

/// Session strip text: `model · ctx 12 % · $0.004` (A2.7, the compressed footer summary).
#[must_use]
pub fn strip_text(model: &str, context: Option<u8>, cost: f64) -> String {
    let mut parts = vec![model.to_owned()];
    if let Some(c) = context {
        parts.push(format!("ctx {c}%"));
    }
    if cost > 0.0 {
        parts.push(if cost < 0.01 {
            format!("${cost:.3}")
        } else {
            format!("${cost:.2}")
        });
    }
    parts.join(" · ")
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

    /// The detail header.
    #[allow(clippy::too_many_lines, reason = "header layout with its menus")]
    pub(crate) fn render_header(
        &self,
        state: HeaderState,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = self.model.title();
        let compact = self.width < px(crate::model::composer::COMPRESS_BELOW + size::SIDEBAR);
        let project = project_label(&self.model.cwd);
        let branch = self.status.branch.clone();
        let live = self.live();
        let dot = match live {
            LiveStatus::Attention => t.brand(),
            LiveStatus::Running => t.working(),
            LiveStatus::Unread => t.success(),
            LiveStatus::Idle => t.fg_subtlest(),
        };
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
            Btn::icon_only("hdr-more", IconName::EllipsisVertical, "More")
                .size(crate::components::button::BtnSize::Xl)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::More, cx))),
            self.menu == Some(PaneMenu::More),
            Placement::BelowEnd,
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
                let rename = {
                    let e = e.clone();
                    MenuEntry::item("Rename", move |window, cx| {
                        let _ = e.update(cx, |p, cx| {
                            p.menu = None;
                            p.start_rename(window, cx);
                        });
                    })
                    .icon(IconName::TextCursorInput)
                };
                let id = id.clone();
                let file = file.clone();
                let reveal = file.clone();
                menu(
                    "header-more",
                    t,
                    size::MENU_MIN * 1.2,
                    vec![
                        rename,
                        act("Fork", IconName::GitFork, SessionPane::fork_session),
                        act("Tree", IconName::ListTree, |p, cx| {
                            p.open_side(SideTab::Tree, cx);
                        })
                        .keys("⌘⇧T"),
                        act("Compact now", IconName::Archive, SessionPane::compact),
                        act(
                            "Export HTML",
                            IconName::ExternalLink,
                            SessionPane::export_html,
                        ),
                        MenuEntry::Separator,
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
                        MenuEntry::item("Reveal file", move |_, cx| {
                            if let Some(f) = &reveal {
                                cx.reveal_path(std::path::Path::new(f));
                            }
                        })
                        .icon(IconName::FolderSearch)
                        .disabled(self.model.file.is_none()),
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
        let folded = state.list_folded;
        let title_el: AnyElement = match &self.rename {
            Some(input) => div()
                .w(px(size::SIDEBAR))
                .child(Input::new(input))
                .into_any_element(),
            None => div()
                .id("hdr-title")
                .flex()
                .flex_col()
                .min_w(px(size::CONTROL_LG * 2.0))
                .flex_shrink(1.0)
                .px(px(space::S1_5))
                .rounded(px(radius::R_ROW))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover()))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if folded && e.click_count() == 1 {
                        window.dispatch_action(Box::new(crate::views::SwitchSession), cx);
                    } else {
                        this.start_rename(window, cx);
                    }
                }))
                .tooltip(crate::components::tooltip::tip(
                    if folded { "Switch session" } else { "Rename" },
                    folded.then_some("⌘O".into()),
                ))
                .child(
                    div()
                        .truncate()
                        .text_role(t, TextRole::Lg)
                        .weight_semibold()
                        .text_color(t.fg())
                        .child(SharedString::from(title)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S1_5))
                        .min_w_0()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtlest())
                        .child(
                            div()
                                .flex_shrink_0()
                                .size(px(size::DOT))
                                .rounded(px(radius::R_FULL))
                                .bg(dot),
                        )
                        .child(div().truncate().child(SharedString::from(match &branch {
                            Some(b) => format!("{project} · {b}"),
                            None => project.clone(),
                        }))),
                )
                .into_any_element(),
        };
        let strip = strip_text(
            &self.model.model_label(),
            self.model.context_percent(),
            self.model.usage.cost,
        );
        let bar = div()
            .id("pane-header")
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .h(px(size::DETAIL_HEAD))
            .pl(px(space::S2))
            .pr(px(space::S3))
            .gap(px(space::S1))
            .border_b_1()
            .border_color(t.line_soft());
        let bar = if state.drag {
            crate::components::window_chrome::drag_region(bar, std::rc::Rc::clone(&self.drag_armed))
        } else {
            bar
        };
        bar.when(state.sidebar_collapsed && state.primary, |el| {
            el.child(
                Btn::icon_only("hdr-sidebar", IconName::PanelLeft, "Show sidebar")
                    .size(crate::components::button::BtnSize::Xl)
                    .tooltip("Show sidebar", Some("⌘B"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleSidebar))),
            )
        })
        .when(!state.sidebar_collapsed || !state.primary, |el| {
            el.child(
                Btn::icon_only("hdr-back", IconName::ChevronLeft, "Close")
                    .size(crate::components::button::BtnSize::Xl)
                    .tooltip("Close", Some("esc"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ClosePane))),
            )
        })
        .child(title_el)
        .child(div().flex_1().min_w(px(space::S2)))
        .when(!compact, |el| {
            el.child(
                div()
                    .id("hdr-strip")
                    .flex()
                    .items_center()
                    .flex_shrink(1.0)
                    .min_w_0()
                    .max_w(relative(0.46))
                    .h(px(size::STRIP_PILL))
                    .px(px(space::S3))
                    .rounded(px(radius::R_FULL))
                    .cursor_pointer()
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .hover(|s| s.bg(t.hover()))
                    .tooltip(crate::components::tooltip::tip(
                        "Model and usage",
                        Some("⌘M".into()),
                    ))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::PickModel)))
                    .child(div().truncate().child(SharedString::from(strip))),
            )
        })
        .when_some(run, |el, k| el.child(capsule(k, cx)))
        .child(more)
        .when(state.primary, |el| {
            el.child(
                Btn::icon_only("hdr-bottom", IconName::PanelBottom, "Terminal")
                    .size(crate::components::button::BtnSize::Xl)
                    .selected(state.bottom_open)
                    .tooltip("Terminal", Some("⌘`"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleBottom))),
            )
            .child(
                Btn::icon_only("hdr-side", IconName::PanelRight, "Side pane")
                    .size(crate::components::button::BtnSize::Xl)
                    .selected(state.side_open)
                    .tooltip("Side pane", Some("⌘J"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::ToggleSide))),
            )
        })
        .when(folded && state.needs > 0, |el| {
            el.child(crate::components::page_header::header_crab(
                crate::design::brand::CrabState::NeedsYou,
                state.needs,
                cx,
            ))
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

#[cfg(test)]
mod tests {
    #[test]
    fn strip_text_compresses_the_footer() {
        assert_eq!(super::strip_text("Flash", Some(3), 0.0), "Flash · ctx 3%");
        assert_eq!(super::strip_text("Flash", None, 0.0042), "Flash · $0.004");
        assert_eq!(
            super::strip_text("Pro", Some(80), 1.5),
            "Pro · ctx 80% · $1.50"
        );
    }
}
