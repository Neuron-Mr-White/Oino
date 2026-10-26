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
use crate::views::shell::{Route, SidebarView, Workspace, now_ms};

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
        match row {
            SideRow::Group {
                cwd,
                label,
                running,
                collapsed,
            } => {
                let toggle = cwd.clone();
                let new_cwd = cwd.clone();
                let group = SharedString::from(format!("sg{ix}"));
                div()
                    .id(ElementId::NamedInteger("side-group".into(), ix as u64))
                    .group(group.clone())
                    .flex()
                    .items_center()
                    .gap(px(space::S1_5))
                    .h(px(size::CONTROL_LG))
                    .px(px(space::S2))
                    .rounded(px(radius::R_LG))
                    .cursor_pointer()
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
                            .invisible()
                            .group_hover(group, gpui_kit::Styled::visible)
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
                    LiveStatus::Attention => dot(t, DotKind::Waiting).into_any_element(),
                    LiveStatus::Unread => dot(t, DotKind::Unread).into_any_element(),
                    LiveStatus::Idle => div().w(px(size::DOT)).into_any_element(),
                };
                let pinned = entry.pinned;
                let row = div()
                    .id(ElementId::NamedInteger("side-session".into(), ix as u64))
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .h(px(size::CONTROL_LG))
                    .pl(px(if self.sidebar_view == SidebarView::Group && !pinned {
                        space::S6
                    } else {
                        space::S2
                    }))
                    .pr(px(space::S2))
                    .rounded(px(radius::R_LG))
                    .cursor_pointer()
                    .when(is_active, |el| el.bg(t.selected()))
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
                        let (e1, e2, e3) = (e.clone(), e.clone(), e.clone());
                        let (i1, i2, i3) = (id.clone(), id.clone(), id.clone());
                        menu(
                            "session-menu",
                            t,
                            size::MENU_MIN,
                            vec![
                                MenuEntry::item(
                                    if pinned { "Unpin" } else { "Pin" },
                                    move |_, cx| {
                                        let _ = e1.update(cx, |ws, cx| {
                                            ws.sessions.toggle_pin(&i1);
                                            ws.sidebar_menu = None;
                                            cx.notify();
                                        });
                                    },
                                )
                                .icon(IconName::Pin),
                                MenuEntry::item("Open in new pane", move |window, cx| {
                                    let _ = e2.update(cx, |ws, cx| {
                                        ws.sidebar_menu = None;
                                        ws.focused = None;
                                        ws.open_session(&i2, window, cx);
                                    });
                                })
                                .icon(IconName::PanelRight)
                                .keys("⌘↵"),
                                MenuEntry::item("Copy session id", move |_, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(i3.clone()));
                                    let _ = e3.update(cx, |ws, cx| {
                                        ws.sidebar_menu = None;
                                        cx.notify();
                                    });
                                })
                                .icon(IconName::Copy),
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let route = self.route;
        let rows = self.side_rows();
        let count = rows.len();
        let has_board = self
            .active_pane()
            .is_some_and(|p| p.read(cx).has_command("unipi:kanboard"));
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
        div()
            .flex()
            .flex_col()
            .size_full()
            .pr(px(space::S1))
            .gap(px(space::HALF))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .h(px(size::HEADER))
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
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("sb-crab")
                            .tooltip(crate::components::tooltip::tip(crab.label(), None))
                            .child(unicrab(crab, size::CRAB.0, cx)),
                    ),
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
                    .child(kbd(&t, "⌘K")),
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
