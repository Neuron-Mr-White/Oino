//! The desktop navigation (design.md A2.8, A2.9): the 224 px nav sidebar (brand row, search
//! field, nav items), the Sessions list column (page header, needs-you strip, one section
//! per project with a grouped card of 64 px session rows, attention-first), the switch sheet
//! (Ctrl+O), the needs-you drawer, the delete confirmation and the command-palette items.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, ElementId, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, relative,
};

use crate::automation::Automatable as _;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::kbd::{kbd, legend};
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::page_header::{PageHeader, header_crab};
use crate::components::section_label::{label as section_label, section_row};
use crate::components::unicrab::{oino_mark, unicrab};
use crate::design::brand::CrabState;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::palette::{Kind, PaletteAction, PaletteItem, Section};
use crate::model::sessions::{LiveStatus, SessionEntry, age, project_label};
use crate::model::timeline::file_changes;
use crate::views::shell::{
    AppShell, Confirm, Route, SECTION_ROWS, SideAction, SwitchSheet, now_ms,
};

/// Colour of a session state (A2.2: colour + icon + word).
fn state_color(t: &OinoTheme, status: LiveStatus) -> Hsla {
    match status {
        LiveStatus::Attention => t.brand(),
        LiveStatus::Running => t.working(),
        LiveStatus::Unread => t.success(),
        LiveStatus::Idle => t.fg_subtle(),
    }
}

/// A nav item: 38 high, radius 10, icon 16 + label 14/500; the shortcut fades in on hover.
fn nav_item(
    t: &OinoTheme,
    id: &'static str,
    glyph: IconName,
    label: &'static str,
    keys: Option<&'static str>,
    active: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let group = SharedString::from(format!("{id}-g"));
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .items_center()
        .gap(px(space::S2_5))
        .h(px(size::NAV_ITEM))
        .px(px(space::S2_5))
        .rounded(px(radius::R_ROW))
        .cursor_pointer()
        .text_role(t, TextRole::Base)
        .weight_medium()
        .text_color(if active { t.fg() } else { t.fg_subtle() })
        .when(active, |el| {
            el.bg(t.selected())
                .border_1()
                .border_color(t.selected_border())
        })
        .when(!active, |el| el.hover(|s| s.bg(t.hover())))
        .child(icon(
            glyph,
            size::ICON,
            if active { t.fg() } else { t.fg_subtle() },
        ))
        .child(div().flex_1().child(label))
        .when_some(keys, |el, k| {
            el.child(
                div()
                    .invisible()
                    .group_hover(group, gpui_kit::Styled::visible)
                    .child(kbd(t, k)),
            )
        })
}

impl AppShell {
    /// Opens `id` (if needed) and applies `action` to its session.
    pub(crate) fn side_action(
        &mut self,
        id: &str,
        action: SideAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_side = Some((id.to_owned(), action));
        self.open_session(id, window, cx);
        self.run_pending_side(window, cx);
    }

    /// Applies a pending list action once its session is open.
    pub(crate) fn run_pending_side(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, action)) = self.pending_side.clone() else {
            return;
        };
        let Some(pane) = self
            .panes
            .iter()
            .find(|p| p.entity.read(cx).model.id == id)
            .map(|p| p.entity.clone())
        else {
            return;
        };
        self.pending_side = None;
        pane.update(cx, |p, cx| match action {
            SideAction::Rename => p.start_rename(window, cx),
            SideAction::Fork => p.send(oino_client::PiCommand::Clone.into(), cx),
        });
    }

    /// Asks before deleting a session file (app-convention: important actions confirm).
    pub(crate) fn confirm_delete(
        &mut self,
        id: &str,
        title: &str,
        path: Option<std::path::PathBuf>,
    ) {
        let Some(path) = path else {
            return;
        };
        let id = id.to_owned();
        self.confirm = Some(Confirm {
            title: "Delete session?".into(),
            body: SharedString::from(format!(
                "\u{201c}{title}\u{201d} and its file {} are deleted. This cannot be undone.",
                path.display()
            )),
            action: "Delete".into(),
            run: Rc::new(move |ws, _window, cx| {
                // Stop a session that is open first (it would keep writing the file).
                if let Some(key) = ws
                    .panes
                    .iter()
                    .find(|p| p.entity.read(cx).model.id == id)
                    .map(|p| p.key)
                {
                    ws.drop_pane(key, cx);
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => {
                        ws.sessions.remove(&id);
                        ws.toast(
                            crate::components::toast::ToastKind::Info,
                            None,
                            "Session deleted",
                            cx,
                        );
                    }
                    Err(error) => ws.toast(
                        crate::components::toast::ToastKind::Error,
                        None,
                        format!("Deleting the session: {error}"),
                        cx,
                    ),
                }
            }),
        });
    }

    /// The confirmation dialog, when one is pending. It owns the keyboard while open and
    /// returns focus on close (QA-1).
    pub(crate) fn render_confirm(
        &self,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let c = self.confirm.clone()?;
        let e = cx.entity().downgrade();
        let run = Rc::clone(&c.run);
        let restore: crate::components::dialog::OnClose = {
            let e = e.clone();
            Rc::new(move |w, cx| {
                let _ = e.update(cx, |ws, cx| {
                    ws.confirm = None;
                    ws.refocus_after_overlay(w, cx);
                    cx.notify();
                });
            })
        };
        let cancel = Rc::clone(&restore);
        let focus = crate::components::dialog::owned_focus("confirm", window, cx);
        Some(crate::components::dialog::modal_owned(
            t,
            "confirm",
            c.title.clone(),
            size::DIALOG_SM,
            div()
                .text_role(t, TextRole::Base)
                .text_color(t.fg_subtle())
                .child(c.body.clone())
                .into_any_element(),
            vec![
                Btn::new("confirm-cancel")
                    .label("Cancel")
                    .variant(Variant::Secondary)
                    .size(BtnSize::Xl)
                    .on_click(move |_, w, cx| cancel(w, cx))
                    .into_any_element(),
                Btn::new("confirm-ok")
                    .label(c.action.clone())
                    .icon(IconName::Trash)
                    .variant(Variant::Danger)
                    .size(BtnSize::Xl)
                    .on_click(move |_, window, cx| {
                        let run = Rc::clone(&run);
                        let _ = e.update(cx, |ws, cx| {
                            ws.confirm = None;
                            run(ws, window, cx);
                            ws.refocus_after_overlay(window, cx);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            ],
            &focus,
            restore,
        ))
    }

    /// The flat list of session rows as shown (list keyboard order).
    fn list_rows(&self) -> Vec<SessionEntry> {
        let mut rows: Vec<SessionEntry> = self.sessions.pinned();
        for section in self.sessions.sections() {
            if self.sessions.is_collapsed(&section.cwd) {
                continue;
            }
            let all = self.list_expanded.contains(&section.cwd);
            rows.extend(
                section
                    .sessions
                    .into_iter()
                    .filter(|e| !e.pinned)
                    .take(if all { 200 } else { SECTION_ROWS }),
            );
        }
        rows
    }

    /// List keys (A2.8): ↑/↓ move, Home/End, Enter opens, F2 renames, Delete asks to delete,
    /// `/` searches, Esc returns to the composer.
    fn on_list_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.list_rows();
        if rows.is_empty() {
            return;
        }
        let n = rows.len();
        let cur = self.sidebar_cursor.unwrap_or(0).min(n - 1);
        let row = rows.get(cur).cloned();
        match (event.keystroke.key.as_str(), row) {
            ("down" | "j", _) => self.sidebar_cursor = Some((cur + 1).min(n - 1)),
            ("up" | "k", _) => self.sidebar_cursor = Some(cur.saturating_sub(1)),
            ("home", _) => self.sidebar_cursor = Some(0),
            ("end", _) => self.sidebar_cursor = Some(n - 1),
            ("enter", Some(e)) => self.open_session(&e.summary.id, window, cx),
            ("f2", Some(e)) => self.side_action(&e.summary.id, SideAction::Rename, window, cx),
            ("delete" | "backspace", Some(e)) => {
                self.confirm_delete(&e.summary.id, &e.summary.title, e.summary.path.clone());
            }
            ("/", _) => self.open_palette("", window, cx),
            ("escape", _) => {
                self.sidebar_cursor = None;
                if let Some(p) = self.active_pane().cloned() {
                    crate::views::session_pane::SessionPane::focus(&p, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The anchored session menu (right-click on a row).
    fn session_menu(
        &self,
        entry: &SessionEntry,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let e = cx.entity().downgrade();
        let id = entry.summary.id.clone();
        let path = entry.summary.path.clone();
        let title = entry.summary.title.clone();
        let pinned = entry.pinned;
        let act =
            |label: &'static str,
             glyph: IconName,
             f: Rc<dyn Fn(&mut AppShell, &str, &mut Window, &mut Context<AppShell>)>| {
                let e = e.clone();
                let id = id.clone();
                MenuEntry::item(label, move |window, cx| {
                    let _ = e.update(cx, |ws, cx| {
                        ws.sidebar_menu = None;
                        f(ws, &id, window, cx);
                        cx.notify();
                    });
                })
                .icon(glyph)
            };
        let copy_id = id.clone();
        let reveal = path.clone();
        let delete_path = path.clone();
        menu(
            "session-menu",
            t,
            size::MENU_MIN * 1.1,
            vec![
                act(
                    "Open",
                    IconName::MessageSquare,
                    Rc::new(AppShell::open_session),
                )
                .keys("↵"),
                act(
                    "Rename",
                    IconName::TextCursorInput,
                    Rc::new(|ws, id, window, cx| {
                        ws.side_action(id, SideAction::Rename, window, cx);
                    }),
                )
                .keys("F2"),
                act(
                    if pinned { "Unpin" } else { "Pin" },
                    IconName::Pin,
                    Rc::new(|ws, id, _, _| ws.sessions.toggle_pin(id)),
                ),
                act(
                    "Fork",
                    IconName::GitFork,
                    Rc::new(|ws, id, window, cx| {
                        ws.side_action(id, SideAction::Fork, window, cx);
                    }),
                ),
                MenuEntry::Separator,
                MenuEntry::item("Copy session id", {
                    let e = e.clone();
                    move |_, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()));
                        let _ = e.update(cx, |ws, cx| {
                            ws.sidebar_menu = None;
                            cx.notify();
                        });
                    }
                })
                .icon(IconName::Copy),
                MenuEntry::item("Reveal file", {
                    let e = e.clone();
                    move |_, cx| {
                        if let Some(p) = &reveal {
                            cx.reveal_path(p);
                        }
                        let _ = e.update(cx, |ws, cx| {
                            ws.sidebar_menu = None;
                            cx.notify();
                        });
                    }
                })
                .icon(IconName::FolderSearch)
                .disabled(path.is_none()),
                MenuEntry::Separator,
                act(
                    "Delete…",
                    IconName::Trash,
                    Rc::new(move |ws, id, _, _| {
                        ws.confirm_delete(id, &title, delete_path.clone());
                    }),
                )
                .keys("Del")
                .disabled(path.is_none()),
            ],
        )
        .into_any_element()
    }

    /// A 64 px session row (A2.8): icon tile with the state dot, title, state word + age,
    /// chevron; the open session gets `selected` and a 3 px shell bar.
    fn render_session_row(
        &self,
        ix: usize,
        entry: &SessionEntry,
        active_id: Option<&str>,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.summary.id.clone();
        let is_active = self.detail_open && active_id == Some(entry.summary.id.as_str());
        let cursor = self.sidebar_cursor == Some(ix) && self.sidebar_kb;
        let menu_open = self.sidebar_menu.as_deref() == Some(entry.summary.id.as_str());
        let narrow = f32::from(self.list_width) < size::LIST - space::S10;
        let color = state_color(t, entry.status);
        let pulse = if entry.status == LiveStatus::Attention {
            crate::components::unicrab::pulse(cx)
        } else {
            1.0
        };
        let sub = match entry.status {
            LiveStatus::Idle => None,
            s => Some(s.word()),
        };
        let when = age(now_ms(), entry.summary.modified_ms);
        let row = div()
            .id(ElementId::NamedInteger("session-row".into(), ix as u64))
            .relative()
            .flex()
            .items_center()
            .gap(px(space::S3))
            .min_h(px(if narrow { size::ROW_COMPACT } else { size::ROW }))
            .px(px(space::S3_5))
            .py(px(space::S2))
            .cursor_pointer()
            .when(is_active, |el| el.bg(t.selected()))
            .when(!is_active, |el| el.hover(|s| s.bg(t.hover())))
            .when(cursor, |el| el.border_2().border_color(t.focus()))
            .on_click(cx.listener({
                let id = id.clone();
                move |this, _: &ClickEvent, window, cx| this.open_session(&id, window, cx)
            }))
            .on_mouse_down(
                gpui_kit::MouseButton::Right,
                cx.listener({
                    let id = id.clone();
                    move |this, _, _, cx| {
                        this.sidebar_menu = Some(id.clone());
                        cx.notify();
                    }
                }),
            )
            .when(is_active, |el| {
                el.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(space::S2_5))
                        .bottom(px(space::S2_5))
                        .w(px(size::RAIL))
                        .rounded_r(px(radius::R_FULL))
                        .bg(t.primary()),
                )
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .size(px(size::TILE_ROW))
                    .rounded(px(radius::R_ROW))
                    .bg(t.chip())
                    .child(oino_mark(
                        t,
                        size::ICON_LG,
                        if is_active { 1.0 } else { 0.7 },
                    ))
                    .when(entry.status != LiveStatus::Idle, |el| {
                        el.child(
                            div()
                                .absolute()
                                .bottom(-px(space::HALF))
                                .right(-px(space::HALF))
                                .size(px(size::DOT + space::HALF * 2.0))
                                .rounded(px(radius::R_FULL))
                                .bg(t.card())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .size(px(size::DOT))
                                        .rounded(px(radius::R_FULL))
                                        .bg(color)
                                        .opacity(pulse),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(space::HALF))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .min_w_0()
                            .when(entry.pinned, |el| {
                                el.child(icon(IconName::Pin, size::ICON_SM, t.fg_subtlest()))
                            })
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, TextRole::Prose)
                                    .line_height(t.text(TextRole::Base).1)
                                    .weight_medium()
                                    .text_color(t.fg())
                                    .child(SharedString::from(entry.summary.title.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .text_role(t, TextRole::Caption)
                            .text_color(t.fg_subtlest())
                            .when_some(sub, |el, w| {
                                el.child(div().text_color(color).child(w)).child("·")
                            })
                            .child(SharedString::from(when)),
                    ),
            )
            .child(icon(IconName::ChevronRight, size::ICON_MD, t.fg_subtlest()));
        let e = cx.entity().downgrade();
        let entry = entry.clone();
        let row = crate::automation::probe(
            row,
            crate::automation::Meta::labelled(
                crate::automation::Role::Row,
                entry.summary.title.clone(),
            )
            .selected(is_active),
        );
        popover(
            row,
            menu_open,
            Placement::BelowEnd,
            || self.session_menu(&entry, t, cx),
            move |_: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |ws, cx| {
                    ws.sidebar_menu = None;
                    cx.notify();
                });
            },
        )
        .into_any_element()
    }

    /// The needs-you strip (A2.9): amber, crab 28, "2 sessions need you", ▾ opens the drawer.
    fn render_strip(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let waiting = self.sessions.needs_you();
        let first = waiting.first()?.summary.id.clone();
        let text = if waiting.len() == 1 {
            waiting[0].summary.title.clone()
        } else {
            format!("{} sessions need you", waiting.len())
        };
        Some(
            div()
                .id("needs-strip")
                .flex()
                .items_center()
                .gap(px(space::S3))
                .min_h(px(size::STRIP))
                .px(px(space::S4))
                .mb(px(space::S2))
                .rounded(px(radius::R_FLOAT))
                .bg(t.attention_surface())
                .border_1()
                .border_color(t.attention_border())
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_session(&first, window, cx);
                }))
                .child(unicrab(CrabState::NeedsYou, size::CRAB_HEADER, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_role(t, TextRole::Prose)
                        .weight_medium()
                        .text_color(t.attention_fg())
                        .child(SharedString::from(text)),
                )
                .child(
                    Btn::icon_only("strip-more", IconName::ChevronDown, "What needs you")
                        .color(t.attention_fg())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.needs_drawer = !this.needs_drawer;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    /// The Sessions list column.
    #[allow(clippy::too_many_lines, reason = "list column layout")]
    pub(crate) fn render_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        self.sidebar_kb = self.sidebar_focus.is_focused(window);
        let total = self.sessions.len();
        let needs = self.sessions.needs_you().len();
        let working = self.sessions.count(LiveStatus::Running);
        let mut meta = vec![format!(
            "{total} session{}",
            if total == 1 { "" } else { "s" }
        )];
        if needs > 0 {
            meta.push(format!("{needs} need you"));
        }
        if working > 0 {
            meta.push(format!("{working} working"));
        }
        let crab = self.crab(cx);
        let header = PageHeader::new("Sessions")
            .meta(meta.join(" · "))
            .tool(
                Btn::icon_only("list-refresh", IconName::RefreshCw, "Refresh")
                    .size(BtnSize::Xl)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_sessions(cx))),
            )
            .tool(
                Btn::icon_only("list-new", IconName::Plus, "New session")
                    .size(BtnSize::Xl)
                    .tooltip("New session", Some("⌘N"))
                    .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
            )
            .tool(
                div()
                    .id("list-crab")
                    .cursor_pointer()
                    .rounded(px(radius::R_FULL))
                    .hover(|s| s.bg(t.hover()))
                    .tooltip(crate::components::tooltip::tip(crab.label(), None))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.needs_drawer = !this.needs_drawer;
                        cx.notify();
                    }))
                    .child(header_crab(crab, needs, cx))
                    .automation(crate::automation::Meta::labelled(
                        crate::automation::Role::Button,
                        crab.label(),
                    )),
            )
            .render(&t);
        let active = self.active_pane().map(|p| p.read(cx).model.id.clone());
        let strip = self.render_strip(&t, cx);
        let mut body: Vec<AnyElement> = Vec::new();
        let mut ix = 0usize;
        let pinned = self.sessions.pinned();
        if !pinned.is_empty() {
            body.push(section_row(&t, "Pinned", body.is_empty(), None).into_any_element());
            let rows: Vec<AnyElement> = pinned
                .iter()
                .map(|e| {
                    let r = self.render_session_row(ix, e, active.as_deref(), &t, cx);
                    ix += 1;
                    r
                })
                .collect();
            body.push(
                crate::components::card::grouped(&t, rows, size::ROW_DIVIDER_INSET)
                    .into_any_element(),
            );
        }
        for (si, section) in self.sessions.sections().into_iter().enumerate() {
            let collapsed = self.sessions.is_collapsed(&section.cwd);
            let expanded = self.list_expanded.contains(&section.cwd);
            let entries: Vec<SessionEntry> =
                section.sessions.into_iter().filter(|e| !e.pinned).collect();
            if entries.is_empty() {
                continue;
            }
            let count = entries.len();
            let cwd = section.cwd.clone();
            let menu_open = self.group_menu.as_deref() == Some(cwd.as_str());
            let e = cx.entity().downgrade();
            let more = popover(
                Btn::icon_only(
                    ElementId::NamedInteger("section-more".into(), si as u64),
                    IconName::Ellipsis,
                    "Project actions",
                )
                .size(BtnSize::Sm)
                .on_click(cx.listener({
                    let cwd = cwd.clone();
                    move |this, _, _, cx| {
                        this.group_menu = Some(cwd.clone());
                        cx.notify();
                    }
                })),
                menu_open,
                Placement::BelowEnd,
                || {
                    let (e1, e2, e3, e4) = (e.clone(), e.clone(), e.clone(), e.clone());
                    let (c1, c2, c3, c4) = (cwd.clone(), cwd.clone(), cwd.clone(), cwd.clone());
                    menu(
                        "project-menu",
                        &t,
                        size::MENU_MIN * 1.1,
                        vec![
                            MenuEntry::item(
                                if collapsed {
                                    "Show sessions"
                                } else {
                                    "Hide sessions"
                                },
                                move |_, cx| {
                                    let _ = e4.update(cx, |ws, cx| {
                                        ws.group_menu = None;
                                        ws.sessions.toggle_group(&c4);
                                        cx.notify();
                                    });
                                },
                            )
                            .icon(if collapsed {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            }),
                            MenuEntry::item("Reveal folder", move |_, cx| {
                                cx.reveal_path(std::path::Path::new(&c1));
                                let _ = e1.update(cx, |ws, cx| {
                                    ws.group_menu = None;
                                    cx.notify();
                                });
                            })
                            .icon(IconName::FolderSearch),
                            MenuEntry::item("Copy path", move |_, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(c2.clone()));
                                let _ = e2.update(cx, |ws, cx| {
                                    ws.group_menu = None;
                                    cx.notify();
                                });
                            })
                            .icon(IconName::Copy),
                            MenuEntry::Separator,
                            MenuEntry::item("Remove from the list", move |_, cx| {
                                let _ = e3.update(cx, |ws, cx| {
                                    ws.group_menu = None;
                                    ws.sessions.hide_group(&c3);
                                    cx.notify();
                                });
                            })
                            .icon(IconName::X),
                        ],
                    )
                    .into_any_element()
                },
                {
                    let e = cx.entity().downgrade();
                    move |_: &mut Window, cx: &mut App| {
                        let _ = e.update(cx, |ws, cx| {
                            ws.group_menu = None;
                            cx.notify();
                        });
                    }
                },
            );
            let aside = div()
                .flex()
                .items_center()
                .gap(px(space::S1))
                .child(
                    div()
                        .pr(px(space::S1))
                        .child(SharedString::from(count.to_string())),
                )
                .child(
                    Btn::icon_only(
                        ElementId::NamedInteger("section-new".into(), si as u64),
                        IconName::Plus,
                        "New session in this project",
                    )
                    .size(BtnSize::Sm)
                    .tooltip(
                        SharedString::from(format!("New session in {}", section.label)),
                        None,
                    )
                    .on_click(cx.listener({
                        let cwd = cwd.clone();
                        move |this, _, window, cx| this.new_session_in(&cwd, window, cx)
                    })),
                )
                .child(more)
                .into_any_element();
            body.push(
                section_row(&t, section.label.clone(), body.is_empty(), Some(aside))
                    .into_any_element(),
            );
            if collapsed {
                continue;
            }
            let shown = if expanded {
                count.min(200)
            } else {
                count.min(SECTION_ROWS)
            };
            let rows: Vec<AnyElement> = entries[..shown]
                .iter()
                .map(|e| {
                    let r = self.render_session_row(ix, e, active.as_deref(), &t, cx);
                    ix += 1;
                    r
                })
                .collect();
            body.push(
                crate::components::card::grouped(&t, rows, size::ROW_DIVIDER_INSET)
                    .into_any_element(),
            );
            if count > SECTION_ROWS {
                let cwd = cwd.clone();
                body.push(
                    div()
                        .flex()
                        .pt(px(space::S2))
                        .child(
                            div()
                                .id(ElementId::NamedInteger("section-all".into(), si as u64))
                                .px(px(space::S2_5))
                                .h(px(size::CONTROL))
                                .flex()
                                .items_center()
                                .rounded(px(radius::R_FULL))
                                .cursor_pointer()
                                .text_role(&t, TextRole::Sm)
                                .weight_medium()
                                .text_color(t.gold())
                                .hover(|s| s.bg(t.hover()))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !this.list_expanded.remove(&cwd) {
                                        this.list_expanded.insert(cwd.clone());
                                    }
                                    cx.notify();
                                }))
                                .child(if expanded {
                                    "Show less".to_owned()
                                } else {
                                    format!("Show all {count}")
                                }),
                        )
                        .into_any_element(),
                );
            }
        }
        let empty = body.is_empty();
        div()
            .id("session-list")
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.sidebar_focus)
            .on_key_down(cx.listener(Self::on_list_key))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.sidebar_focus, cx);
                }),
            )
            .child(header)
            .child(
                div()
                    .id("session-list-scroll")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(size::PAGE_GUTTER))
                    .pb(px(size::PAGE_GUTTER))
                    .children(strip)
                    .when(empty, |el| {
                        el.child(
                            crate::components::empty::empty_state(
                                &t,
                                IconName::MessageSquare,
                                "No sessions yet",
                                Some("Start one and Oino gets to work in this folder.".into()),
                                vec![
                                    Btn::new("list-empty-new")
                                        .label("New session")
                                        .icon(IconName::Plus)
                                        .variant(Variant::Primary)
                                        .size(BtnSize::Xl)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.new_session(window, cx);
                                        }))
                                        .into_any_element(),
                                ],
                            )
                            .mt(px(size::EMPTY_TOP)),
                        )
                    })
                    .children(body),
            )
            .into_any_element()
    }

    /// The nav sidebar (A2.8): brand row, search field, nav items, build info.
    #[allow(clippy::too_many_lines, reason = "sidebar layout")]
    pub(crate) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let route = self.route;
        let has_board = self
            .active_pane()
            .is_some_and(|p| p.read(cx).has_command("kanboard"));
        let board_ready = self
            .active_pane()
            .and_then(|p| {
                p.read(cx)
                    .model
                    .ui
                    .panel(crate::model::kanban::BOARD_PANEL)
                    .and_then(|panel| crate::model::kanban::recognise(&panel.spec))
            })
            .map_or(0, |b| crate::model::kanban::ready_count(&b));
        let crab = self.crab(cx);
        let needs = self.sessions.needs_you().len();
        let mac = cfg!(target_os = "macos");
        let version = self.host.info().version;
        let brand = {
            let row = div()
                .id("sidebar-brand")
                .flex()
                .items_center()
                .gap(px(space::S2_5))
                .h(px(size::BRAND_ROW))
                .px(px(space::S1_5))
                .mb(px(space::S1_5));
            if crate::components::window_chrome::client_decorated(window) {
                crate::components::window_chrome::drag_region(row, Rc::clone(&self.drag_armed))
            } else {
                row
            }
        }
        .when(mac, |el| el.pl(px(size::TRAFFIC_LIGHTS)))
        .child(unicrab(crab, size::CRAB_BRAND, cx))
        .child(
            div()
                .flex_1()
                .text_role(&t, TextRole::Xl)
                .line_height(px(size::CRAB_BRAND))
                .weight_semibold()
                .text_color(t.fg())
                .child("Oino"),
        )
        .when_some(self.update_available.clone(), |el, _| {
            el.child(
                div()
                    .id("sb-update")
                    .flex()
                    .items_center()
                    .h(px(size::BADGE + space::S1))
                    .px(px(space::S2))
                    .rounded(px(radius::R_FULL))
                    .bg(t.info_surface())
                    .text_color(t.info())
                    .text_role(&t, TextRole::Xs)
                    .weight_semibold()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_settings(Some("about"), window, cx);
                    }))
                    .child("Update")
                    .automation(crate::automation::Meta::labelled(
                        crate::automation::Role::Button,
                        "Update",
                    )),
            )
        });
        let search = div()
            .id("sidebar-search")
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::SEARCH))
            .pl(px(space::S2_5))
            .pr(px(space::S2))
            .mb(px(space::S2_5))
            .rounded(px(radius::R_ROW))
            .bg(t.chip())
            .border_1()
            .border_color(t.border())
            .cursor_text()
            .text_role(&t, TextRole::Base)
            .text_color(t.fg_subtlest())
            .hover(|s| s.border_color(t.border_hover()))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_palette("", window, cx);
            }))
            .child(icon(IconName::Search, size::ICON, t.fg_subtlest()))
            .child(div().flex_1().child("Search"))
            .child(kbd(&t, "⌘K"))
            .automation(crate::automation::Meta::labelled(
                crate::automation::Role::Button,
                "Search",
            ));
        div()
            .flex()
            .flex_col()
            .size_full()
            .pt(px(space::S3_5))
            .px(px(space::S2_5))
            .pb(px(space::S3))
            .child(brand)
            .child(search)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::HALF))
                    .child(
                        nav_item(
                            &t,
                            "nav-sessions",
                            IconName::MessageSquare,
                            "Sessions",
                            Some("⌘1"),
                            route == Route::Sessions,
                        )
                        .when(needs > 0, |el| {
                            el.child(crate::components::chip::count_badge(&t, needs, true))
                        })
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.run_app_action("sessions", window, cx);
                        }))
                        .automation(
                            crate::automation::Meta::labelled(
                                crate::automation::Role::Row,
                                "Sessions",
                            )
                            .selected(route == Route::Sessions),
                        ),
                    )
                    .when(has_board, |el| {
                        el.child(
                            nav_item(
                                &t,
                                "nav-board",
                                IconName::Kanban,
                                "Board",
                                Some("⌘2"),
                                route == Route::Board,
                            )
                            .when(board_ready > 0, |el| {
                                el.child(crate::components::chip::count_badge(
                                    &t,
                                    board_ready,
                                    false,
                                ))
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_board(window, cx);
                            }))
                            .automation(
                                crate::automation::Meta::labelled(
                                    crate::automation::Role::Row,
                                    "Board",
                                )
                                .selected(route == Route::Board),
                            ),
                        )
                    })
                    .child(
                        nav_item(
                            &t,
                            "nav-cron",
                            IconName::CalendarClock,
                            "Automations",
                            Some("⌘4"),
                            route == Route::Cron,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_cron(window, cx);
                        }))
                        .automation(
                            crate::automation::Meta::labelled(
                                crate::automation::Role::Row,
                                "Automations",
                            )
                            .selected(route == Route::Cron),
                        ),
                    )
                    .child(
                        nav_item(
                            &t,
                            "nav-ext",
                            IconName::Blocks,
                            "Extensions",
                            Some("⌘3"),
                            route == Route::Extensions,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.run_app_action("extensions", window, cx);
                        }))
                        .automation(
                            crate::automation::Meta::labelled(
                                crate::automation::Role::Row,
                                "Extensions",
                            )
                            .selected(route == Route::Extensions),
                        ),
                    )
                    .child(
                        nav_item(
                            &t,
                            "nav-settings",
                            IconName::Settings,
                            "Settings",
                            Some("⌘,"),
                            route == Route::Settings,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_settings(None, window, cx);
                        }))
                        .automation(
                            crate::automation::Meta::labelled(
                                crate::automation::Role::Row,
                                "Settings",
                            )
                            .selected(route == Route::Settings),
                        ),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px(px(space::S2_5))
                    .text_role(&t, TextRole::Xs)
                    .text_color(t.fg_subtlest())
                    .child(SharedString::from(format!("Oino {version}"))),
            )
            .into_any_element()
    }

    // ── switch sheet ────────────────────────────────────────────────────────

    /// Opens the switch sheet (Ctrl+O, A2.8).
    pub(crate) fn open_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Switch to a session…"));
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        if let Some(s) = this.switch.as_mut() {
                            s.cursor = 0;
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.switch_pick(window, cx),
                    _ => {}
                },
            );
        self.switch = Some(SwitchSheet {
            input,
            cursor: 0,
            _subscription: sub,
        });
        cx.notify();
    }

    /// Sessions of the switch sheet for its filter (attention-first, grouped by project).
    fn switch_rows(&self, cx: &App) -> Vec<(String, SessionEntry)> {
        let query = self
            .switch
            .as_ref()
            .map(|s| s.input.read(cx).value().to_lowercase())
            .unwrap_or_default();
        let mut out = Vec::new();
        for section in self.sessions.sections() {
            for e in section.sessions {
                let hit = query.is_empty()
                    || e.summary.title.to_lowercase().contains(&query)
                    || e.summary.cwd.to_lowercase().contains(&query);
                if hit && out.len() < 200 {
                    out.push((section.label.clone(), e));
                }
            }
        }
        out
    }

    fn switch_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.switch_rows(cx);
        let cur = self.switch.as_ref().map_or(0, |s| s.cursor);
        if let Some((_, e)) = rows.get(cur) {
            let id = e.summary.id.clone();
            self.switch = None;
            self.open_session(&id, window, cx);
        }
        cx.notify();
    }

    fn on_switch_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.switch_rows(cx).len();
        let Some(s) = self.switch.as_mut() else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => {
                self.switch = None;
                if let Some(p) = self.active_pane().cloned() {
                    crate::views::session_pane::SessionPane::focus(&p, window, cx);
                }
            }
            "down" => s.cursor = (s.cursor + 1).min(n.saturating_sub(1)),
            "up" => s.cursor = s.cursor.saturating_sub(1),
            "enter" => self.switch_pick(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The switch sheet dialog.
    pub(crate) fn render_switch(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let sheet = self.switch.as_ref()?;
        let rows = self.switch_rows(cx);
        let cursor = sheet.cursor;
        let active = self.active_pane().map(|p| p.read(cx).model.id.clone());
        let mut list: Vec<AnyElement> = Vec::new();
        let mut last: Option<String> = None;
        for (i, (project, e)) in rows.iter().enumerate() {
            if last.as_deref() != Some(project.as_str()) {
                list.push(section_row(t, project.clone(), last.is_none(), None).into_any_element());
                last = Some(project.clone());
            }
            let color = state_color(t, e.status);
            let id = e.summary.id.clone();
            let selected = i == cursor;
            let model = self
                .panes
                .iter()
                .map(|p| p.entity.read(cx))
                .find(|p| p.model.id == e.summary.id)
                .map(|p| {
                    let m = p.model.model_label();
                    p.model
                        .context_percent()
                        .map_or(m.clone(), |c| format!("{m} · ctx {c}%"))
                });
            list.push(
                div()
                    .id(ElementId::NamedInteger("switch-row".into(), i as u64))
                    .flex()
                    .items_center()
                    .gap(px(space::S3))
                    .min_h(px(size::PALETTE_ROW + space::S2))
                    .px(px(space::S3))
                    .rounded(px(radius::R_ROW))
                    .cursor_pointer()
                    .when(selected, |el| {
                        el.bg(t.selected())
                            .border_1()
                            .border_color(t.selected_border_strong())
                    })
                    .when(!selected, |el| el.hover(|s| s.bg(t.hover())))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_session(&id, window, cx);
                    }))
                    .child(oino_mark(t, size::ICON, 0.8))
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
                                    .weight_medium()
                                    .text_color(t.fg())
                                    .child(SharedString::from(e.summary.title.clone())),
                            )
                            .when_some(model, |el, m| {
                                el.child(
                                    div()
                                        .truncate()
                                        .text_role(t, TextRole::Sm)
                                        .text_color(t.fg_subtlest())
                                        .child(SharedString::from(m)),
                                )
                            }),
                    )
                    .when(active.as_deref() == Some(e.summary.id.as_str()), |el| {
                        el.child(
                            div()
                                .text_role(t, TextRole::Sm)
                                .text_color(t.fg_subtlest())
                                .child("Open"),
                        )
                    })
                    .when(e.status != LiveStatus::Idle, |el| {
                        el.child(crate::components::chip::status(t, color, e.status.word()))
                    })
                    .into_any_element(),
            );
        }
        let empty = rows.is_empty();
        let close = cx.listener(|this, _: &ClickEvent, _, cx| {
            this.switch = None;
            cx.notify();
        });
        Some(
            div()
                .id("switch-scrim")
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(relative(size::PALETTE_TOP_FRACTION))
                .bg(t.scrim())
                .occlude()
                .on_click(close)
                .child(crate::design::motion::enter_ms(
                    t,
                    "switch-enter",
                    div()
                        .id("switch")
                        .flex()
                        .flex_col()
                        .w(px(size::DIALOG_MD))
                        .max_w(relative(0.94))
                        .max_h(relative(size::PALETTE_MAX_FRACTION))
                        .rounded(px(radius::R_FLOAT))
                        .bg(t.sheet())
                        .border_1()
                        .border_color(t.border())
                        .shadow(t.e_palette())
                        .text_color(t.fg())
                        .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                        .capture_key_down(cx.listener(Self::on_switch_key))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S2_5))
                                .h(px(size::PALETTE_INPUT))
                                .px(px(space::S4))
                                .border_b_1()
                                .border_color(t.line_soft())
                                .text_role(t, TextRole::Prose)
                                .child(icon(IconName::Search, size::ICON_LG, t.fg_subtle()))
                                .child(
                                    div()
                                        .flex_1()
                                        .child(Input::new(&sheet.input).appearance(false)),
                                )
                                .child(kbd(t, "esc")),
                        )
                        .child(
                            div()
                                .id("switch-rows")
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .p(px(space::S1_5))
                                .px(px(space::S3))
                                .when(empty, |el| {
                                    el.child(
                                        div()
                                            .p(px(space::S4))
                                            .text_role(t, TextRole::Base)
                                            .text_color(t.fg_subtle())
                                            .child("No session matches. Try another word."),
                                    )
                                })
                                .children(list),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S4))
                                .h(px(size::CONTROL_LG + space::S1))
                                .px(px(space::S4))
                                .border_t_1()
                                .border_color(t.line_soft())
                                .child(legend(t, "↑↓", "move"))
                                .child(legend(t, "↵", "open"))
                                .child(legend(t, "esc", "close")),
                        ),
                    crate::design::tokens::motion::PALETTE_MS,
                ))
                .into_any_element(),
        )
    }

    // ── needs-you drawer ────────────────────────────────────────────────────

    /// The needs-you drawer (A2.13): waiting and done-unseen sessions with Open, Mark all
    /// read; a lore line when nothing needs you.
    pub(crate) fn render_needs_drawer(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.needs_drawer {
            return None;
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        let groups = self.sessions.groups();
        let all: Vec<SessionEntry> = groups.into_iter().flat_map(|g| g.sessions).collect();
        let waiting: Vec<&SessionEntry> = all
            .iter()
            .filter(|e| e.status == LiveStatus::Attention)
            .collect();
        let done: Vec<&SessionEntry> = all
            .iter()
            .filter(|e| e.status == LiveStatus::Unread)
            .collect();
        for (i, e) in waiting.iter().chain(done.iter()).enumerate() {
            let what = self
                .panes
                .iter()
                .map(|p| p.entity.read(cx))
                .find(|p| p.model.id == e.summary.id)
                .and_then(|p| p.model.ui.dialog().map(|d| d.title.clone()))
                .filter(|s| !s.is_empty());
            let sub = match e.status {
                LiveStatus::Attention => what.unwrap_or_else(|| "Waiting for you".into()),
                _ => "Done".into(),
            };
            let id = e.summary.id.clone();
            let color = state_color(t, e.status);
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S3))
                    .min_h(px(size::ROW_COMPACT))
                    .px(px(space::S3_5))
                    .child(
                        div()
                            .size(px(size::DOT))
                            .rounded(px(radius::R_FULL))
                            .bg(color),
                    )
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
                                    .weight_medium()
                                    .child(SharedString::from(e.summary.title.clone())),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, TextRole::Sm)
                                    .text_color(color)
                                    .child(SharedString::from(sub)),
                            ),
                    )
                    .child(
                        Btn::new(ElementId::NamedInteger("needs-open".into(), i as u64))
                            .label("Open")
                            .variant(Variant::Secondary)
                            .size(BtnSize::Md)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_session(&id, window, cx);
                            })),
                    )
                    .into_any_element(),
            );
        }
        let empty = rows.is_empty();
        let left = if self.sidebar_open {
            size::SIDEBAR
        } else {
            space::S0
        } + f32::from(self.list_width)
            - size::PAGE_GUTTER
            - size::SIDE_PANE_MIN
            - space::S10;
        Some(
            div()
                .id("needs-scrim")
                .absolute()
                .inset_0()
                .occlude()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.needs_drawer = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .absolute()
                        .top(px(size::PAGE_HEAD_PAD.0 + size::PAGE_HEAD_BAR))
                        .left(px(left.max(space::S4)))
                        .child(crate::design::motion::enter(
                            t,
                            "needs-enter",
                            div()
                                .id("needs-drawer")
                                .w(px(size::SIDE_PANE_MIN + space::S10))
                                .flex()
                                .flex_col()
                                .p(px(space::S1_5))
                                .rounded(px(radius::R_FLOAT))
                                .bg(t.menu())
                                .border_1()
                                .border_color(t.border())
                                .shadow(t.e_menu())
                                .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .px(px(space::S3_5))
                                        .pt(px(space::S2))
                                        .pb(px(space::S1))
                                        .child(section_label(t, "Needs you").flex_1())
                                        .when(!done.is_empty(), |el| {
                                            el.child(
                                                Btn::new("needs-read")
                                                    .label("Mark all read")
                                                    .size(BtnSize::Sm)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        let ids: Vec<String> = this
                                                            .sessions
                                                            .groups()
                                                            .into_iter()
                                                            .flat_map(|g| g.sessions)
                                                            .filter(|e| {
                                                                e.status == LiveStatus::Unread
                                                            })
                                                            .map(|e| e.summary.id)
                                                            .collect();
                                                        for id in ids {
                                                            this.sessions
                                                                .set_live(&id, LiveStatus::Idle);
                                                        }
                                                        cx.notify();
                                                    })),
                                            )
                                        }),
                                )
                                .when(empty, |el| {
                                    el.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S3))
                                    .p(px(space::S3_5))
                                    .child(unicrab(CrabState::Idle, size::CRAB_HEADER, cx))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_role(t, TextRole::Caption)
                                            .text_color(t.fg_subtle())
                                            .child(
                                                "Unicrab is watching the tide. Nothing needs \
                                                 you.",
                                            ),
                                    ),
                            )
                                })
                                .children(rows),
                        )),
                )
                .into_any_element(),
        )
    }

    // ── palette ─────────────────────────────────────────────────────────────

    /// Command-palette items (A2.17 groups: Go to, Session, View, Extensions with extension
    /// shortcuts (GA-16), App).
    pub(crate) fn palette_items(&self, cx: &App) -> Vec<PaletteItem> {
        let a = |label: &str, section: Section, icon: &'static str, id: &str| {
            PaletteItem::new(
                label,
                section,
                Kind::Action,
                icon,
                PaletteAction::App(id.to_owned()),
            )
        };
        let mut items = Vec::new();
        let pane = self.active_pane().map(|p| p.read(cx));
        if let Some(p) = pane {
            for change in file_changes(p.model.transcript.rows()) {
                items.push(
                    PaletteItem::new(
                        change.path.clone(),
                        Section::RecentChanges,
                        Kind::File,
                        "file",
                        PaletteAction::File(change.path.clone()),
                    )
                    .detail(format!("+{} −{}", change.added, change.removed)),
                );
            }
        }
        let active = pane.map(|p| p.model.id.clone());
        let mut recent: Vec<SessionEntry> = self
            .sessions
            .groups()
            .into_iter()
            .flat_map(|g| g.sessions)
            .collect();
        recent.sort_by_key(|e| (e.status.rank(), std::cmp::Reverse(e.summary.modified_ms)));
        for e in recent {
            items.push(
                PaletteItem::new(
                    e.summary.title.clone(),
                    Section::RecentSessions,
                    Kind::Session,
                    "message-square",
                    PaletteAction::Session(e.summary.id.clone()),
                )
                .detail(format!(
                    "{}{} · {}",
                    if e.status == LiveStatus::Idle {
                        String::new()
                    } else {
                        format!("{} · ", e.status.word())
                    },
                    project_label(&e.summary.cwd),
                    age(now_ms(), e.summary.modified_ms)
                ))
                .current(active.as_deref() == Some(e.summary.id.as_str())),
            );
        }
        items.push(a("Sessions", Section::Suggested, "message-square", "sessions").keys("⌘1"));
        if pane.is_some_and(|p| p.has_command("kanboard")) {
            items.push(a("Board", Section::Suggested, "kanban", "board").keys("⌘2"));
        }
        items.push(a("Extensions", Section::Suggested, "blocks", "extensions").keys("⌘3"));
        items.push(a("Settings", Section::Suggested, "settings", "settings").keys("⌘,"));
        items.push(a("Switch session…", Section::Suggested, "search", "switch").keys("⌘O"));
        items.push(a("New session", Section::Chat, "square-pen", "new-session").keys("⌘N"));
        items.push(
            a(
                "New session in folder…",
                Section::Chat,
                "folder-open",
                "open-folder",
            )
            .keys("⌘⇧O"),
        );
        items.push(a("Model", Section::Chat, "sparkles", "model").keys("⌘M"));
        items.push(a("Compact", Section::Chat, "archive", "compact"));
        items.push(a("Fork", Section::Chat, "git-branch", "fork"));
        items.push(a("Tree", Section::Chat, "list-tree", "tree").keys("⌘⇧T"));
        items.push(a("Stop", Section::Chat, "square", "abort").keys("⌘."));
        items.push(
            a(
                "Toggle sidebar",
                Section::Panels,
                "panel-left",
                "toggle-sidebar",
            )
            .keys("⌘B"),
        );
        items.push(a("Side pane", Section::Panels, "panel-right", "toggle-side").keys("⌘J"));
        items.push(a("Terminal", Section::Panels, "panel-bottom", "toggle-bottom").keys("⌘`"));
        items.push(a("Review changes", Section::Panels, "file", "review"));
        items.push(a("Files", Section::Panels, "folder-open", "files"));
        if let Some(p) = pane {
            for s in &p.model.ui.shortcuts {
                items.push(
                    PaletteItem::new(
                        s.description.clone(),
                        Section::Extensions,
                        Kind::Action,
                        "keyboard",
                        PaletteAction::Shortcut(s.id.clone()),
                    )
                    .detail(s.id.clone())
                    .keys(pi_keys(&s.keys)),
                );
            }
            for (name, description) in p.commands() {
                let mut item = PaletteItem::new(
                    format!("/{name}"),
                    Section::Extensions,
                    Kind::Action,
                    "zap",
                    PaletteAction::Slash(name.clone()),
                );
                if let Some(d) = description {
                    item = item.detail(d.clone());
                }
                items.push(item);
            }
        }
        items.push(a(
            "Keyboard shortcuts",
            Section::App,
            "keyboard",
            "shortcuts",
        ));
        items.push(a("Theme: Light", Section::App, "sun", "theme-light"));
        items.push(a("Theme: Dark", Section::App, "moon", "theme-dark"));
        items.push(a("Theme: System", Section::App, "settings", "theme-system"));
        items.push(a(
            "Check for updates",
            Section::App,
            "circle-arrow-down",
            "check-updates",
        ));
        items.push(a(
            "Refresh session list",
            Section::App,
            "refresh-cw",
            "refresh",
        ));
        items.push(a("Frame-time overlay", Section::App, "zap", "frame-overlay").keys("⌃⌥⇧F"));
        items
    }
}

/// A pi key spec (`ctrl+alt+c`) in the palette's glyph form (`⌃⌥C`; platform labels follow).
#[must_use]
pub fn pi_keys(spec: &str) -> String {
    let spec = spec.split(',').next().unwrap_or_default();
    let mut out = String::new();
    let parts: Vec<&str> = spec.split('+').collect();
    let n = parts.len();
    for (i, part) in parts.into_iter().enumerate() {
        let p = part.trim().to_ascii_lowercase();
        if i + 1 < n {
            out.push_str(match p.as_str() {
                "ctrl" | "control" => "⌃",
                "alt" | "option" => "⌥",
                "shift" => "⇧",
                "cmd" | "super" | "meta" => "⌘",
                _ => "",
            });
        } else {
            out.push_str(&match p.as_str() {
                "down" => "↓".to_owned(),
                "up" => "↑".to_owned(),
                "left" => "←".to_owned(),
                "right" => "→".to_owned(),
                "enter" | "return" => "↵".to_owned(),
                "escape" | "esc" => "esc".to_owned(),
                "tab" => "Tab".to_owned(),
                "space" => "Space".to_owned(),
                other => other.to_uppercase(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn pi_key_specs_render_as_glyphs() {
        assert_eq!(super::pi_keys("alt+p"), "⌥P");
        assert_eq!(super::pi_keys("shift+down"), "⇧↓");
        assert_eq!(super::pi_keys("ctrl+alt+c"), "⌃⌥C");
    }
}
