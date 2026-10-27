//! Sidebar (design.md §4.7): top row, nav items, view switcher, virtualised workspace groups
//! and session rows, footer with settings, update badge, model label and the Unicrab glyph.
//! Also builds the command-center items (§4.25).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, ElementId, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px, uniform_list,
};

use crate::components::badge::{Capsule, DotKind, capsule, dot, hash_color, kbd, pill};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::segmented;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::unicrab::unicrab;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::palette::{Kind, PaletteAction, PaletteItem, Section};
use crate::model::sessions::{LiveStatus, SessionEntry, age};
use crate::model::timeline::file_changes;
use crate::views::shell::{Confirm, Route, SideAction, SidebarView, Workspace, now_ms};

/// One virtualised sidebar row.
#[derive(Clone, Debug)]
enum SideRow {
    Group {
        cwd: String,
        label: String,
        running: usize,
        collapsed: bool,
    },
    Session(SessionEntry),
}

fn nav_item(
    t: &OinoTheme,
    id: &'static str,
    glyph: IconName,
    label: &'static str,
    keys: Option<&'static str>,
    active: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(space::S2))
        .h(px(size::CONTROL_LG))
        .px(px(space::S2))
        .rounded(px(radius::R_LG))
        .cursor_pointer()
        .text_role(t, TextRole::Base)
        .text_color(t.fg())
        .when(active, |el| el.bg(t.selected()))
        .hover(|s| s.bg(t.hover()))
        .child(icon(glyph, size::ICON, t.fg_subtle()))
        .child(div().flex_1().child(label))
        .when_some(keys, |el, k| {
            el.child(
                div()
                    .mono_text(t, TextRole::Xs)
                    .text_color(t.fg_subtlest())
                    .child(SharedString::from(crate::components::badge::platform_keys(
                        k,
                    ))),
            )
        })
}

impl Workspace {
    /// Opens `id` (if needed) and applies `action` to its pane.
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

    /// Applies a pending sidebar action once its pane is open.
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
            run: Rc::new(move |ws, window, cx| {
                // Stop a session that is open first (its pane would keep writing the file).
                if let Some(key) = ws
                    .panes
                    .iter()
                    .find(|p| p.entity.read(cx).model.id == id)
                    .map(|p| p.key)
                {
                    ws.drop_pane(key, cx);
                    if ws.layout.is_empty() {
                        ws.new_session(window, cx);
                    } else if let Some(first) = ws.focused {
                        ws.focus_pane(first, window, cx);
                    }
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

    /// The confirmation dialog, when one is pending.
    pub(crate) fn render_confirm(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let c = self.confirm.clone()?;
        let e = cx.entity().downgrade();
        let run = Rc::clone(&c.run);
        let close: crate::components::dialog::OnClose = {
            let e = e.clone();
            Rc::new(move |_, cx: &mut App| {
                let _ = e.update(cx, |ws, cx| {
                    ws.confirm = None;
                    cx.notify();
                });
            })
        };
        let cancel = Rc::clone(&close);
        Some(crate::components::dialog::modal_frame(
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
                    .variant(Variant::Ghost)
                    .on_click(move |_, w, cx| cancel(w, cx))
                    .into_any_element(),
                Btn::new("confirm-ok")
                    .label(c.action.clone())
                    .icon(IconName::Trash)
                    .variant(Variant::Danger)
                    .on_click(move |_, window, cx| {
                        let run = Rc::clone(&run);
                        let _ = e.update(cx, |ws, cx| {
                            ws.confirm = None;
                            run(ws, window, cx);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            ],
            close,
        ))
    }

    /// Session-list keys (design.md §4.7): ↑/↓ move, ←/→ collapse/expand a group, Enter
    /// opens, ⌘Enter opens in a new pane, F2 renames, Delete asks to delete, `/` searches.
    fn on_sidebar_key(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.side_rows();
        if rows.is_empty() {
            return;
        }
        let n = rows.len();
        let cur = self.sidebar_cursor.unwrap_or(0).min(n - 1);
        let primary = crate::components::secondary(&event.keystroke.modifiers);
        let row = rows.get(cur).cloned();
        match (event.keystroke.key.as_str(), row) {
            ("down", _) => self.sidebar_cursor = Some((cur + 1).min(n - 1)),
            ("up", _) => self.sidebar_cursor = Some(cur.saturating_sub(1)),
            ("home", _) => self.sidebar_cursor = Some(0),
            ("end", _) => self.sidebar_cursor = Some(n - 1),
            ("left", Some(SideRow::Group { cwd, collapsed, .. })) => {
                if !collapsed {
                    self.sessions.toggle_group(&cwd);
                }
            }
            ("left", Some(SideRow::Session(_))) => {
                // Up to the owning group header.
                if let Some(g) = rows[..cur]
                    .iter()
                    .rposition(|r| matches!(r, SideRow::Group { .. }))
                {
                    self.sidebar_cursor = Some(g);
                }
            }
            ("right", Some(SideRow::Group { cwd, collapsed, .. })) => {
                if collapsed {
                    self.sessions.toggle_group(&cwd);
                } else {
                    self.sidebar_cursor = Some((cur + 1).min(n - 1));
                }
            }
            ("enter" | "space", Some(SideRow::Group { cwd, .. })) => {
                self.sessions.toggle_group(&cwd);
            }
            ("enter", Some(SideRow::Session(e))) => {
                if primary {
                    self.focused = None;
                }
                self.open_session(&e.summary.id, window, cx);
            }
            ("f2", Some(SideRow::Session(e))) => {
                self.side_action(&e.summary.id, SideAction::Rename, window, cx);
            }
            ("delete" | "backspace", Some(SideRow::Session(e))) => {
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

    fn side_rows(&self) -> Vec<SideRow> {
        let mut rows = Vec::new();
        match self.sidebar_view {
            SidebarView::Group => {
                for entry in self.sessions.pinned() {
                    rows.push(SideRow::Session(entry));
                }
                for group in self.sessions.groups() {
                    let collapsed = self.sessions.is_collapsed(&group.cwd);
                    let running = group
                        .sessions
                        .iter()
                        .filter(|s| matches!(s.status, LiveStatus::Running | LiveStatus::Attention))
                        .count();
                    rows.push(SideRow::Group {
                        cwd: group.cwd.clone(),
                        label: group.label.clone(),
                        running,
                        collapsed,
                    });
                    if !collapsed {
                        rows.extend(group.sessions.into_iter().take(50).map(SideRow::Session));
                    }
                }
            }
            SidebarView::Recent => {
                let mut all: Vec<SessionEntry> = self
                    .sessions
                    .groups()
                    .into_iter()
                    .flat_map(|g| g.sessions)
                    .collect();
                all.sort_by_key(|e| std::cmp::Reverse(e.summary.modified_ms));
                rows.extend(all.into_iter().take(200).map(SideRow::Session));
            }
        }
        rows
    }

    #[allow(clippy::too_many_lines, reason = "row layout with its context menu")]
    fn render_side_row(
        &self,
        ix: usize,
        row: &SideRow,
        active_id: Option<&str>,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Keyboard cursor ring only while the list has focus (focus-visible).
        let cursor = self.sidebar_cursor == Some(ix) && self.sidebar_kb;
        match row {
            SideRow::Group {
                cwd,
                label,
                running,
                collapsed,
            } => {
                let toggle = cwd.clone();
                let new_cwd = cwd.clone();
                let menu_cwd = cwd.clone();
                let group = SharedString::from(format!("sg{ix}"));
                let group_menu_open = self.group_menu.as_deref() == Some(cwd.as_str());
                let e = cx.entity().downgrade();
                let more = popover(
                    Btn::icon_only(
                        ElementId::NamedInteger("group-more".into(), ix as u64),
                        IconName::Ellipsis,
                        "Workspace actions",
                    )
                    .size(BtnSize::Xs)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.group_menu = Some(menu_cwd.clone());
                        cx.notify();
                    })),
                    group_menu_open,
                    Placement::BelowEnd,
                    || {
                        let (e1, e2, e3) = (e.clone(), e.clone(), e.clone());
                        let (c1, c2, c3) = (cwd.clone(), cwd.clone(), cwd.clone());
                        menu(
                            "group-menu",
                            t,
                            size::MENU_MIN,
                            vec![
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
                                MenuEntry::item("Remove from sidebar", move |_, cx| {
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
                div()
                    .id(ElementId::NamedInteger("side-group".into(), ix as u64))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap(px(space::S1_5))
                    .h(px(if self.settings.sidebar_compact {
                        size::CONTROL
                    } else {
                        size::CONTROL_LG
                    }))
                    .px(px(space::S2))
                    .rounded(px(radius::R_LG))
                    .cursor_pointer()
                    .when(cursor, |el| el.border_2().border_color(t.focus()))
                    .hover(|s| s.bg(t.hover()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.sessions.toggle_group(&toggle);
                        cx.notify();
                    }))
                    .child(icon(
                        if *collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        },
                        size::ICON_SM,
                        t.fg_subtlest(),
                    ))
                    .child(icon(
                        IconName::Folder,
                        size::ICON_MD,
                        t.categorical(hash_color(cwd)),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Sm)
                            .weight_medium()
                            .text_color(t.fg())
                            .child(SharedString::from(label.clone())),
                    )
                    .when(*running > 0, |el| {
                        el.child(pill(t, t.tag(), t.fg_subtle(), running.to_string()))
                    })
                    .child(
                        div()
                            .flex()
                            .when(!group_menu_open, |el| {
                                el.invisible().group_hover(group, gpui_kit::Styled::visible)
                            })
                            .child(more)
                            .child(
                                Btn::icon_only(
                                    ElementId::NamedInteger("group-new".into(), ix as u64),
                                    IconName::Plus,
                                    "New session here",
                                )
                                .size(BtnSize::Xs)
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.new_session_in(&new_cwd, window, cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            }
            SideRow::Session(entry) => {
                let id = entry.summary.id.clone();
                let is_active = active_id == Some(entry.summary.id.as_str());
                let menu_open = self.sidebar_menu.as_deref() == Some(entry.summary.id.as_str());
                let trailing: AnyElement = match entry.status {
                    LiveStatus::Running => capsule(Capsule::Running, cx).into_any_element(),
                    LiveStatus::Attention => capsule(Capsule::Waiting, cx).into_any_element(),
                    _ => div()
                        .text_role(t, TextRole::Xs)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(age(now_ms(), entry.summary.modified_ms)))
                        .into_any_element(),
                };
                let lead: AnyElement = match entry.status {
                    LiveStatus::Running => dot(t, DotKind::Running).into_any_element(),
                    LiveStatus::Attention => dot(t, DotKind::Waiting)
                        .opacity(crate::components::unicrab::pulse(cx))
                        .into_any_element(),
                    LiveStatus::Unread => dot(t, DotKind::Unread).into_any_element(),
                    LiveStatus::Idle => div().w(px(size::DOT)).into_any_element(),
                };
                let pinned = entry.pinned;
                let row = div()
                    .id(ElementId::NamedInteger("side-session".into(), ix as u64))
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .h(px(if self.settings.sidebar_compact {
                        size::CONTROL
                    } else {
                        size::CONTROL_LG
                    }))
                    .pl(px(if self.sidebar_view == SidebarView::Group && !pinned {
                        space::S6
                    } else {
                        space::S2
                    }))
                    .pr(px(space::S2))
                    .rounded(px(radius::R_LG))
                    .cursor_pointer()
                    .when(is_active, |el| el.bg(t.selected()))
                    .when(cursor, |el| el.border_2().border_color(t.focus()))
                    .hover(|s| s.bg(t.hover()))
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
                    .child(lead)
                    .when(pinned, |el| {
                        el.child(icon(IconName::Pin, size::ICON_SM, t.fg_subtlest()))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Base)
                            .text_color(if is_active { t.fg() } else { t.fg_subtle() })
                            .child(SharedString::from(entry.summary.title.clone())),
                    )
                    .child(trailing);
                let e = cx.entity().downgrade();
                popover(
                    row,
                    menu_open,
                    Placement::BelowStart,
                    || {
                        let path = entry.summary.path.clone();
                        let title = entry.summary.title.clone();
                        let act = |label: &'static str,
                                   glyph: IconName,
                                   f: Rc<
                            dyn Fn(&mut Workspace, &str, &mut Window, &mut Context<Workspace>),
                        >| {
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
                            size::MENU_MIN,
                            vec![
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
                                act(
                                    "Open in new pane",
                                    IconName::PanelRight,
                                    Rc::new(|ws, id, window, cx| {
                                        ws.focused = None;
                                        ws.open_session(id, window, cx);
                                    }),
                                )
                                .keys("⌘↵"),
                                MenuEntry::Separator,
                                MenuEntry::item("Copy session id", {
                                    let e = e.clone();
                                    move |_, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_id.clone(),
                                        ));
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
                    },
                    {
                        let e = cx.entity().downgrade();
                        move |_: &mut Window, cx: &mut App| {
                            let _ = e.update(cx, |ws, cx| {
                                ws.sidebar_menu = None;
                                cx.notify();
                            });
                        }
                    },
                )
                .into_any_element()
            }
        }
    }

    /// The sidebar.
    #[allow(clippy::too_many_lines, reason = "sidebar layout")]
    pub(crate) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        self.sidebar_kb = self.sidebar_focus.is_focused(window);
        let route = self.route;
        let rows = self.side_rows();
        let count = rows.len();
        let has_board = self
            .active_pane()
            .is_some_and(|p| p.read(cx).has_command("unipi:kanboard"));
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
        let model_label = self
            .active_pane()
            .map(|p| p.read(cx).model.model_label())
            .unwrap_or_default();
        let crab = self.crab(cx);
        let rows = Rc::new(rows);
        let list_rows = Rc::clone(&rows);
        let list = uniform_list(
            "sidebar-sessions",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let t = OinoTheme::get(cx);
                let active = this.active_pane().map(|p| p.read(cx).model.id.clone());
                range
                    .filter_map(|ix| {
                        list_rows
                            .get(ix)
                            .map(|r| this.render_side_row(ix, r, active.as_deref(), &t, cx))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1();
        let view_ix = usize::from(self.sidebar_view == SidebarView::Recent);
        let switch_entity = cx.entity().downgrade();
        let mac = cfg!(target_os = "macos");
        let list = div()
            .id("sidebar-list")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .track_focus(&self.sidebar_focus)
            .on_key_down(cx.listener(Self::on_sidebar_key))
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.sidebar_focus, cx);
                }),
            )
            .child(list);
        div()
            .flex()
            .flex_col()
            .size_full()
            .pr(px(space::S1))
            .gap(px(space::HALF))
            .child(
                {
                    let top = div()
                        .id("sidebar-top")
                        .flex()
                        .items_center()
                        .gap(px(space::S1))
                        .h(px(size::HEADER));
                    if crate::components::window_chrome::client_decorated(window) {
                        crate::components::window_chrome::drag_region(
                            top,
                            Rc::clone(&self.drag_armed),
                        )
                    } else {
                        top
                    }
                }
                .when(mac, |el| el.pl(px(size::TRAFFIC_LIGHTS)))
                .child(
                    Btn::icon_only("sb-toggle", IconName::PanelLeft, "Hide sidebar")
                        .tooltip("Hide sidebar", Some("⌘B"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    Btn::icon_only("sb-back", IconName::ArrowLeft, "Back")
                        .size(BtnSize::Sm)
                        .disabled(self.history.1 == 0)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.history_step(-1, window, cx);
                        })),
                )
                .child(
                    Btn::icon_only("sb-forward", IconName::ChevronRight, "Forward")
                        .size(BtnSize::Sm)
                        .disabled(self.history.1 + 1 >= self.history.0.len())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.history_step(1, window, cx)),
                        ),
                )
                .child(div().flex_1()),
            )
            .child(
                nav_item(
                    &t,
                    "nav-new",
                    IconName::SquarePen,
                    "New session",
                    Some("⌘N"),
                    false,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.new_session(window, cx)),
                ),
            )
            .child(
                nav_item(
                    &t,
                    "nav-search",
                    IconName::Search,
                    "Search",
                    Some("⌘K"),
                    false,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_palette("", window, cx);
                })),
            )
            .when(has_board, |el| {
                el.child(
                    nav_item(
                        &t,
                        "nav-board",
                        IconName::Kanban,
                        "Board",
                        None,
                        route == Route::Board,
                    )
                    .when(board_ready > 0, |el| {
                        el.child(pill(&t, t.accent(), t.fg(), board_ready.to_string()))
                    })
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.open_board(window, cx)),
                    ),
                )
            })
            .child(
                nav_item(
                    &t,
                    "nav-ext",
                    IconName::Blocks,
                    "Extensions",
                    None,
                    route == Route::Extensions,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.run_app_action("extensions", window, cx);
                })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .pt(px(space::S3))
                    .pb(px(space::S1))
                    .child(segmented(
                        "sb-view",
                        &t,
                        &["Group", "Recent"],
                        view_ix,
                        Rc::new(move |i, _, cx| {
                            let _ = switch_entity.update(cx, |ws, cx| {
                                ws.sidebar_view = if i == 0 {
                                    SidebarView::Group
                                } else {
                                    SidebarView::Recent
                                };
                                cx.notify();
                            });
                        }),
                    ))
                    .child(div().flex_1())
                    .child(
                        Btn::icon_only(
                            "sb-sort",
                            IconName::ArrowUpDown,
                            if self.sessions.sorted_by_name() {
                                "Sort: name (click for recent)"
                            } else {
                                "Sort: recent (click for name)"
                            },
                        )
                        .size(BtnSize::Sm)
                        .selected(self.sessions.sorted_by_name())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sessions.toggle_sort();
                            cx.notify();
                        })),
                    )
                    .child(
                        Btn::icon_only("sb-collapse", IconName::Minus, "Collapse all")
                            .size(BtnSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                for g in this.sessions.groups() {
                                    if !this.sessions.is_collapsed(&g.cwd) {
                                        this.sessions.toggle_group(&g.cwd);
                                    }
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .h(px(size::SIDEBAR_FOOTER))
                    .child(
                        Btn::icon_only("sb-settings", IconName::Settings, "Settings")
                            .tooltip("Settings", Some("⌘,"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_settings(None, window, cx);
                            })),
                    )
                    .when_some(self.update_available.clone(), |el, _| {
                        el.child(
                            Btn::new("sb-update")
                                .label("Update")
                                .icon(IconName::CircleArrowDown)
                                .size(BtnSize::Sm)
                                .variant(Variant::Ghost)
                                .color(t.progress_surface_fg())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.run_app_action("check-updates", window, cx);
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(&t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(model_label)),
                    )
                    .child(kbd(&t, "⌘M"))
                    .child(
                        // Aggregate Unicrab state glyph (design.md §4.7 footer, unipi §1.5.1).
                        div()
                            .id("sb-crab")
                            .px(px(space::S1))
                            .tooltip(crate::components::tooltip::tip(crab.label(), None))
                            .child(unicrab(crab, size::CRAB.0, cx)),
                    ),
            )
            .into_any_element()
    }

    /// Command-center items (design.md §4.25 sections).
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
        recent.sort_by_key(|e| std::cmp::Reverse(e.summary.modified_ms));
        for e in recent {
            items.push(
                PaletteItem::new(
                    e.summary.title.clone(),
                    Section::RecentSessions,
                    Kind::Session,
                    "dot",
                    PaletteAction::Session(e.summary.id.clone()),
                )
                .detail(format!(
                    "{} · {}",
                    crate::model::sessions::workspace_label(&e.summary.cwd),
                    age(now_ms(), e.summary.modified_ms)
                ))
                .current(active.as_deref() == Some(e.summary.id.as_str())),
            );
        }
        items.push(
            a(
                "New session",
                Section::Suggested,
                "square-pen",
                "new-session",
            )
            .keys("⌘N"),
        );
        items.push(
            a(
                "Open folder…",
                Section::Suggested,
                "folder-open",
                "open-folder",
            )
            .keys("⌘O"),
        );
        items.push(a("Settings", Section::Suggested, "settings", "settings").keys("⌘,"));
        items.push(a("Compact", Section::Chat, "archive", "compact"));
        items.push(a("Fork", Section::Chat, "git-branch", "fork"));
        items.push(a("Tree", Section::Chat, "list-tree", "tree").keys("⌘⇧T"));
        items.push(a("Abort", Section::Chat, "square", "abort").keys("⌘."));
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
        items.push(
            a(
                "Bottom frame",
                Section::Panels,
                "panel-bottom",
                "toggle-bottom",
            )
            .keys("⌘`"),
        );
        items.push(a("Split pane", Section::Panels, "panel-right", "split").keys("⌘\\"));
        items.push(a("Review changes", Section::Panels, "file", "review"));
        items.push(a("Files", Section::Panels, "folder-open", "files"));
        if pane.is_some_and(|p| p.has_command("unipi:kanboard")) {
            items.push(a("Board", Section::Panels, "kanban", "board"));
        }
        if let Some(p) = pane {
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
            for s in &p.model.ui.shortcuts {
                items.push(
                    PaletteItem::new(
                        s.description.clone(),
                        Section::Extensions,
                        Kind::Action,
                        "keyboard",
                        PaletteAction::Shortcut(s.id.clone()),
                    )
                    .keys(s.keys.clone()),
                );
            }
        }
        items.push(a("Theme: Light", Section::App, "sun", "theme-light"));
        items.push(a("Theme: Dark", Section::App, "moon", "theme-dark"));
        items.push(a("Theme: System", Section::App, "settings", "theme-system"));
        items.push(a(
            "Check for updates",
            Section::App,
            "circle-arrow-down",
            "check-updates",
        ));
        items.push(a("Extensions", Section::App, "blocks", "extensions"));
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
