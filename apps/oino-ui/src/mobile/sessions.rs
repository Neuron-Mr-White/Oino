//! M1 Sessions (design.md §6.1): app bar with the aggregate Unicrab glyph, search and
//! settings; "Sessions" title with Display; the Needs-you card; workspace sections with
//! swipe (Pin, Delete) and long-press (Rename, Fork, Copy id) rows; pull to refresh; the
//! New session pill.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollEvent, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use crate::components::badge::{Capsule, DotKind, capsule, dot};
use crate::components::button::Variant;
use crate::components::unicrab::unicrab;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt};
use crate::design::tokens::touch;
use crate::design::{TextRole, radius, size, space};
use crate::mobile::gesture::{long_press, swipe_row};
use crate::mobile::shell::{MobileShell, Page, SettingsPage, Sheet, now_ms, pill_btn};
use crate::model::interaction::{self, InteractionKind};
use crate::model::sessions::{LiveStatus, SessionEntry, age, workspace_label};

impl MobileShell {
    /// Sessions waiting for the user: `(session id, title, action label)`.
    fn needs_you(&self, cx: &Context<Self>) -> Vec<(String, String, &'static str)> {
        self.panes
            .iter()
            .filter_map(|p| {
                let p = p.entity.read(cx);
                let dialog = p.model.ui.dialog()?;
                let verb = match interaction::kind(dialog) {
                    InteractionKind::Approval => "Approve",
                    InteractionKind::Question => "Answer",
                    InteractionKind::PlanReview => "Review",
                    _ => "Open",
                };
                Some((p.model.id.clone(), p.model.title(), verb))
            })
            .collect()
    }

    fn needs_you_card(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let waiting = self.needs_you(cx);
        if waiting.is_empty() {
            return None;
        }
        let n = waiting.len();
        let eyes = crate::components::unicrab::needs_you_eyes(size::ICON, t.waiting_fg(), cx);
        Some(
            div()
                .mx(px(space::S4))
                .mt(px(space::S3))
                .flex()
                .flex_col()
                .rounded(px(radius::R_CARD))
                .bg(t.waiting_surface())
                .border_1()
                .border_color(t.waiting_fg().opacity(0.25))
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .px(px(space::S4))
                        .pt(px(space::S3))
                        .pb(px(space::S1))
                        .child(eyes)
                        .child(
                            div()
                                .text_role(t, TextRole::Sm)
                                .weight_medium()
                                .text_color(t.waiting_fg())
                                .child(SharedString::from(format!("Needs you ({n})"))),
                        ),
                )
                .children(waiting.into_iter().map(|(id, title, verb)| {
                    let open = id.clone();
                    div()
                        .id(ElementId::Name(format!("m-need-{id}").into()))
                        .flex()
                        .items_center()
                        .gap(px(space::S3))
                        .min_h(px(touch::TARGET + space::S1))
                        .px(px(space::S4))
                        .cursor_pointer()
                        .active(|s| s.bg(t.hover()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.open_session(&open, window, cx);
                        }))
                        .child(dot(t, DotKind::Waiting))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_role(t, TextRole::Base)
                                .text_color(t.fg())
                                .child(SharedString::from(title)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .text_role(t, TextRole::Sm)
                                .weight_medium()
                                .text_color(t.waiting_fg())
                                .child(verb)
                                .child(icon(IconName::ChevronRight, size::ICON_MD, t.waiting_fg())),
                        )
                }))
                .child(div().h(px(space::S2)))
                .into_any_element(),
        )
    }

    fn session_row(
        &mut self,
        entry: &SessionEntry,
        width: gpui_kit::Pixels,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.summary.id.clone();
        let title = if entry.summary.title.is_empty() {
            "Untitled".to_owned()
        } else {
            entry.summary.title.clone()
        };
        let when = age(now_ms(), entry.summary.modified_ms);
        let sub = format!(
            "{} · {} message{}",
            workspace_label(&entry.summary.cwd),
            entry.summary.message_count,
            if entry.summary.message_count == 1 {
                ""
            } else {
                "s"
            }
        );
        let trailing: AnyElement = match entry.status {
            LiveStatus::Running => capsule(Capsule::Running, cx).into_any_element(),
            LiveStatus::Attention => capsule(Capsule::Waiting, cx).into_any_element(),
            LiveStatus::Idle | LiveStatus::Unread => div()
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtlest())
                .child(SharedString::from(when))
                .into_any_element(),
        };
        let lead = match entry.status {
            LiveStatus::Running => dot(t, DotKind::Running),
            LiveStatus::Attention => dot(t, DotKind::Waiting),
            LiveStatus::Unread => dot(t, DotKind::Unread),
            LiveStatus::Idle => div().size(px(size::DOT)).flex_shrink_0(),
        };
        let open = id.clone();
        let menu = id.clone();
        let press = long_press(
            Rc::clone(&self.press),
            Rc::new({
                let entity = cx.entity().downgrade();
                move |_, _, cx| {
                    let menu = menu.clone();
                    let _ = entity.update(cx, |this, cx| {
                        (crate::mobile::MobilePlatform::get(cx).haptic)();
                        this.open_sheet(Sheet::RowMenu(menu), cx);
                    });
                }
            }),
        );
        let content = div()
            .id(ElementId::Name(format!("m-row-{id}").into()))
            .relative()
            .flex()
            .items_center()
            .gap(px(space::S3))
            .w_full()
            .h(px(touch::ROW))
            .px(px(space::S4))
            .bg(t.frame())
            .cursor_pointer()
            .active(|s| s.bg(t.hover()))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_session(&open, window, cx);
            }))
            .child(div().w(px(touch::DOT)).flex().justify_center().child(lead))
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
                            .when(entry.pinned, TextExt::weight_medium)
                            .text_color(t.fg())
                            .child(SharedString::from(title.clone())),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(sub)),
                    ),
            )
            .child(trailing)
            .child(press)
            .into_any_element();
        let pin_id = id.clone();
        let del_id = id.clone();
        let path = entry.summary.path.clone();
        let pinned = entry.pinned;
        let action = |key: &str, glyph: IconName, label: &'static str, bg, fg| {
            div()
                .id(ElementId::Name(format!("m-swipe-{key}-{id}").into()))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(space::S1))
                .w(px(touch::SWIPE_ACTION))
                .h(px(touch::ROW))
                .bg(bg)
                .text_color(fg)
                .text_role(t, TextRole::Sm)
                .weight_medium()
                .cursor_pointer()
                .child(icon(glyph, size::ICON, fg))
                .child(label)
        };
        let actions = vec![
            action(
                "pin",
                IconName::Pin,
                if pinned { "Unpin" } else { "Pin" },
                t.secondary(),
                t.fg(),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.sessions.toggle_pin(&pin_id);
                if let Some(h) = this.swipes.get(&pin_id) {
                    crate::mobile::gesture::close_row(h);
                }
                cx.notify();
            }))
            .into_any_element(),
            action(
                "del",
                IconName::Trash,
                "Delete",
                t.deep(),
                gpui_kit::white(),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.confirm_delete(&del_id, &title, path.clone(), cx);
            }))
            .into_any_element(),
        ];
        let handle = self.swipes.entry(id.clone()).or_default().clone();
        swipe_row(
            ElementId::Name(format!("m-swipe-{id}").into()),
            width,
            content,
            actions,
            &handle,
        )
    }

    /// Asks before deleting a session file.
    pub(crate) fn confirm_delete(
        &mut self,
        id: &str,
        title: &str,
        path: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = path else {
            return;
        };
        let id = id.to_owned();
        self.open_sheet(
            Sheet::Confirm(crate::mobile::shell::Confirm {
                title: "Delete session?".into(),
                body: SharedString::from(format!(
                    "\u{201c}{title}\u{201d} and its session file are deleted. This cannot be undone."
                )),
                action: "Delete".into(),
                run: Rc::new(move |this, _, cx| {
                    this.panes.retain(|p| p.entity.read(cx).model.id != id);
                    match std::fs::remove_file(&path) {
                        Ok(()) => {
                            this.sessions.remove(&id);
                            this.toast(
                                crate::components::toast::ToastKind::Info,
                                None,
                                "Session deleted",
                                cx,
                            );
                        }
                        Err(error) => this.toast(
                            crate::components::toast::ToastKind::Error,
                            None,
                            format!("Deleting the session: {error}"),
                            cx,
                        ),
                    }
                }),
            }),
            cx,
        );
    }

    pub(crate) fn render_sessions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let crab = self.crab(cx);
        let glyph = div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .child(unicrab(crab, touch::CRAB, cx))
            .when(self.opening, |el| {
                el.child(spinner(
                    crate::design::ticker::Ticker::frame(cx),
                    size::ICON_MD,
                    t.fg_subtle(),
                ))
            })
            .into_any_element();
        let bar = Self::app_bar(
            &t,
            None,
            glyph,
            vec![
                Self::icon_btn(
                    "m-search",
                    IconName::Search,
                    "Search",
                    cx.listener(|this, _, _, cx| this.push(Page::Search, cx)),
                ),
                Self::icon_btn(
                    "m-settings",
                    IconName::Settings,
                    "Settings",
                    cx.listener(|this, _, _, cx| this.open_settings(SettingsPage::Root, cx)),
                ),
            ],
        );
        let heading = div()
            .flex()
            .items_end()
            .px(px(space::S4))
            .pt(px(space::S1))
            .pb(px(space::S1))
            .child(
                div()
                    .flex_1()
                    .text_role(&t, TextRole::Display)
                    .weight_semibold()
                    .text_color(t.fg())
                    .child("Sessions"),
            )
            .child(
                crate::components::button::Btn::new("m-display")
                    .label("Display")
                    .icon(IconName::SlidersHorizontal)
                    .variant(Variant::Ghost)
                    .size(crate::components::button::BtnSize::Touch)
                    .on_click(cx.listener(|this, _, _, cx| this.open_sheet(Sheet::Display, cx))),
            );
        let width = window.viewport_size().width;
        let mut list: Vec<AnyElement> = Vec::new();
        list.extend(self.needs_you_card(&t, cx));
        let pinned = self.sessions.pinned();
        if !pinned.is_empty() {
            list.push(Self::section(&t, "Pinned"));
            for e in &pinned {
                list.push(self.session_row(e, width, &t, cx));
            }
        }
        for group in self.sessions.groups() {
            let cwd = group.cwd.clone();
            list.push(
                div()
                    .flex()
                    .items_center()
                    .pr(px(space::S1))
                    .child(div().flex_1().child(Self::section(&t, group.label.clone())))
                    .child(
                        div().pt(px(space::S3)).child(
                            crate::components::button::Btn::icon_only(
                                ElementId::Name(format!("m-new-in-{cwd}").into()),
                                IconName::Plus,
                                "New session here",
                            )
                            .size(crate::components::button::BtnSize::Touch)
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.new_session(Some(cwd.clone()), window, cx);
                                },
                            )),
                        ),
                    )
                    .into_any_element(),
            );
            for e in group.sessions.iter().filter(|e| !e.pinned) {
                list.push(self.session_row(e, width, &t, cx));
            }
        }
        if self.sessions.is_empty() && !self.refreshing {
            list.push(
                crate::components::empty::empty_state(
                    &t,
                    IconName::MessageSquareDashed,
                    "No sessions yet",
                    Some("Tap New session to start.".into()),
                    Vec::new(),
                )
                .into_any_element(),
            );
        }
        list.push(div().h(px(touch::FAB + space::S8)).into_any_element());
        let pull = self.pull.progress();
        let refreshing = self.refreshing;
        let scroll = self.list_scroll.clone();
        let fab = div()
            .absolute()
            .right(px(space::S4))
            .bottom(px(space::S4))
            .child(
                pill_btn(
                    "m-new",
                    "New session",
                    Some(IconName::SquarePen),
                    Variant::Primary,
                )
                .loading(self.opening)
                .on_click(cx.listener(|this, _, window, cx| this.new_session(None, window, cx))),
            )
            .rounded(px(radius::R_FULL))
            .shadow(t.e3());
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(heading)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("m-sessions")
                            .size_full()
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .track_scroll(&scroll)
                            .on_scroll(cx.listener(move |this, e: &ScrollEvent, _, cx| {
                                let y = this.list_scroll.offset().y;
                                if this.pull.feed(e, y, px(touch::ROW)) {
                                    this.pull_refresh(cx);
                                } else if this.pull.distance > 0.0 {
                                    cx.notify();
                                }
                            }))
                            .child(div().flex().flex_col().w_full().children(list)),
                    )
                    .when(pull > 0.0 || refreshing, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(space::S2))
                                .left_0()
                                .right_0()
                                .flex()
                                .justify_center()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(size::CONTROL_LG))
                                        .rounded(px(radius::R_FULL))
                                        .bg(t.popover())
                                        .shadow(t.e2())
                                        .opacity(if refreshing { 1.0 } else { pull })
                                        .child(spinner(
                                            if refreshing {
                                                crate::design::ticker::Ticker::frame(cx)
                                            } else {
                                                #[allow(
                                                    clippy::cast_possible_truncation,
                                                    clippy::cast_sign_loss,
                                                    reason = "0–12 steps"
                                                )]
                                                let step = (pull * 12.0) as u32;
                                                step
                                            },
                                            size::ICON,
                                            t.primary(),
                                        )),
                                ),
                        )
                    })
                    .child(fab),
            )
            .into_any_element()
    }
}
