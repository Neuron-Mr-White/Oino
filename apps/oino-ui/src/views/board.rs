//! D9 Board (design.md §5.8): the `unipi.kanboard/board` panel promoted to the main area.
//!
//! Header "Board · <project ▾>" (picker over the sidebar's workspaces), a toolbar with Filter,
//! Display, search and `+ New task` (plus the extension's own keys), the native lanes with a
//! keyboard cursor, and the selected task's detail as an overlay.
//!
//! Keys (board focused): h/j/k/l or arrows move, Enter opens the task, `m` move menu,
//! `c` new task, `/` search, Esc closes the overlay / leaves the board.

use std::rc::Rc;

use gpui_kit::Focusable as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, px,
};
use oino_ai::JsonValue;

use crate::automation::Automatable as _;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::dialog::modal_frame;
use crate::components::empty::empty_state;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::ous::kanban::{BoardOptions, filtered};
use crate::components::ous::{self, Ous};
use crate::components::page_header::{PageHeader, header_crab};
use crate::components::toast::ToastKind;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::kanban as kb;
use crate::model::sessions::workspace_label;
use crate::views::session_pane::SessionPane;
use crate::views::shell::{Route, Workspace};

/// Open toolbar menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardMenu {
    /// Project picker.
    Project,
    /// Filter.
    Filter,
    /// Display.
    Display,
    /// Move the focused / selected task.
    Move,
}

/// Board view state kept by the workspace.
#[derive(Default)]
pub struct BoardUi {
    /// Toolbar state.
    pub opts: BoardOptions,
    search: Option<Entity<InputState>>,
    /// The board page's keyboard focus (public: the shell focuses it on route change, QA-1).
    pub focus: Option<FocusHandle>,
    /// Open menu.
    pub menu: Option<BoardMenu>,
    /// Task id whose detail overlay is shown (also the selected card).
    pub detail: Option<String>,
    /// Board project slug (resolved per workspace cwd, or picked in the menu).
    pub project: Option<String>,
    /// Status id of the card being dragged (lanes highlight / dim, #8).
    drag_status: Option<String>,
    /// Lane id under the pointer during a drag.
    hover_lane: Option<String>,
    /// The `(pane, cwd)` `project` was resolved for — a change re-resolves.
    resolved_for: Option<(gpui_kit::EntityId, String)>,
    /// Last board error (shown in the empty state).
    error: Option<String>,
    /// The native New task dialog: title and description fields (GA-20).
    new_task: Option<(Entity<InputState>, Entity<InputState>)>,
}

impl Workspace {
    fn board_pane(&self) -> Option<Entity<SessionPane>> {
        self.active_pane().cloned()
    }

    /// The slug this board shows: the project registered for the active
    /// workspace's cwd, re-resolved when the pane or its cwd changes (a menu
    /// pick survives until then).
    fn board_project(&mut self, cx: &mut Context<Self>) -> Option<String> {
        // No session yet: keep a picked project (the board works without one).
        if self.board_pane().is_none() {
            return self.board_ui.project.clone();
        }
        let pane = self.board_pane()?;
        let id = pane.entity_id();
        let cwd = pane.read(cx).model.cwd.clone();
        // A miss is retried every render: a board can be seeded (scenario
        // `kanboard_seed`) or registered after the pane first drew.
        let resolved = self.board_ui.project.is_some()
            && self
                .board_ui
                .resolved_for
                .as_ref()
                .is_some_and(|(pid, c)| *pid == id && *c == cwd);
        if !resolved {
            self.board_ui.resolved_for = Some((id, cwd.clone()));
            self.board_ui.project = crate::kanboard::kanboard(cx)
                .and_then(|s| s.project_for_cwd(std::path::Path::new(&cwd)).ok())
                .map(|project| project.slug);
        }
        self.board_ui.project.clone()
    }

    /// The board's action path: `select` opens the detail overlay, `move:<x>`
    /// runs the move in-process (round-5 #7 — no panel round-trip).
    fn board_action(&mut self, action: &str, value: JsonValue, cx: &mut Context<Self>) {
        if action == "select" {
            if let Some(id) = value.as_str() {
                self.board_ui.detail = Some(id.to_owned());
            }
            cx.notify();
            return;
        }
        if let Some(status) = action.strip_prefix("drag-start:") {
            self.board_ui.drag_status = Some(status.to_owned());
            self.board_ui.hover_lane = None;
            cx.notify();
            return;
        }
        if let Some(lane) = action.strip_prefix("hover:") {
            // Fires per pointer move; only a lane change repaints.
            if self.board_ui.hover_lane.as_deref() != Some(lane) {
                self.board_ui.hover_lane = Some(lane.to_owned());
                cx.notify();
            }
            return;
        }
        let Some(service) = crate::kanboard::kanboard(cx) else {
            return;
        };
        let Some(slug) = self.board_ui.project.clone() else {
            return;
        };
        if let Some(to) = action.strip_prefix("move:") {
            self.board_ui.drag_status = None;
            self.board_ui.hover_lane = None;
            let Some(id) = self
                .board_ui
                .detail
                .clone()
                .or_else(|| value.as_str().map(str::to_owned))
            else {
                return;
            };
            let Ok(status) = to.parse::<oino_kanboard::Status>() else {
                return;
            };
            match service.move_task(&slug, &id, status, None) {
                Err(e) => self.toast(ToastKind::Error, None, e.to_string(), cx),
                Ok(_) => {
                    self.toast(
                        ToastKind::Success,
                        None,
                        format!("Moved {id} to {}", status.as_str()),
                        cx,
                    );
                }
            }
        } else if action == "show" {
            // The extension prints the board into the chat (kept: it is a
            // command, not the panel).
            if let Some(p) = self.board_pane()
                && p.read(cx).has_command("kanboard")
            {
                p.update(cx, |p, cx| p.run_prompt("/kanboard", cx));
                self.route = Route::Sessions;
            }
        }
        // `refresh` and every mutation: the watcher bumps the windows; the
        // service cache is already invalidated, so a notify is the refresh.
        cx.notify();
    }

    fn board_model(&mut self, cx: &mut Context<Self>) -> Option<kb::Board> {
        let slug = self.board_project(cx)?;
        let service = crate::kanboard::kanboard(cx)?;
        let session = self.board_pane().map(|pane| pane.read(cx).model.id.clone());
        match service.board(&slug, self.board_ui.detail.as_deref(), session.as_deref()) {
            Ok(board) => {
                self.board_ui.error = None;
                Some(board)
            }
            Err(e) => {
                self.board_ui.error = Some(e.to_string());
                None
            }
        }
    }

    /// Opens the native New task dialog (FAB, `c`, the lane's "Add task"); it adds the task
    /// with `/kanboard-add` in the board's session (GA-20).
    fn board_new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let body = cx.new(|cx| InputState::new(window, cx).placeholder("Description (optional)"));
        title.update(cx, |s, cx| s.focus(window, cx));
        for input in [&title, &body] {
            cx.subscribe_in(input, window, |ws, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    ws.board_add_task(window, cx);
                }
            })
            .detach();
        }
        self.board_ui.new_task = Some((title, body));
        cx.notify();
    }

    /// Commit the native New task dialog: `add` in-process through the service
    /// (round-5 #7 — no `/kanboard-add` prompt through the session).
    fn board_add_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((title, body)) = self.board_ui.new_task.take() else {
            return;
        };
        let title = title.read(cx).value().trim().to_owned();
        let body = body.read(cx).value().trim().to_owned();
        if !title.is_empty() {
            match crate::kanboard::kanboard(cx).zip(self.board_ui.project.clone()) {
                Some((service, slug)) => {
                    if let Err(e) =
                        service.add_task(&slug, &title, (!body.is_empty()).then_some(body.as_str()))
                    {
                        self.toast(ToastKind::Error, None, e.to_string(), cx);
                    }
                }
                None => self.toast(
                    ToastKind::Error,
                    None,
                    "No board project — pick or register one.",
                    cx,
                ),
            }
        }
        if let Some(f) = &self.board_ui.focus {
            window.focus(f, cx);
        }
        cx.notify();
    }

    fn new_task_dialog(&self, t: &OinoTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (title, body) = self.board_ui.new_task.clone()?;
        let e = cx.entity().downgrade();
        let e2 = e.clone();
        // QA-1: closing the dialog returns focus to the board (the title input's handle
        // would otherwise die with the dialog and leave the page keyboard-dead).
        let refocus = |ws: &mut Self, w: &mut Window, cx: &mut Context<Self>| {
            if let Some(f) = &ws.board_ui.focus {
                w.focus(f, cx);
            }
        };
        // The inputs register for the automation driver: the dialog owns the keyboard, so a
        // scenario has to click the field it types into (docs/dev/v1/gui-driver.md §2).
        let title_input = crate::automation::registry::probe_named(
            Input::new(&title).into_any_element(),
            crate::automation::Meta::labelled(crate::automation::Role::Input, "Title")
                .value(title.read(cx).value().to_string())
                .focus(title.read(cx).focus_handle(cx)),
            "board-new-task-title-input",
        );
        let body_input = crate::automation::registry::probe_named(
            Input::new(&body).into_any_element(),
            crate::automation::Meta::labelled(crate::automation::Role::Input, "Description")
                .value(body.read(cx).value().to_string())
                .focus(body.read(cx).focus_handle(cx)),
            "board-new-task-description-input",
        );
        let fields = div()
            .flex()
            .flex_col()
            .gap(px(space::S3))
            .child(title_input)
            .child(body_input)
            .child(
                div()
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtlest())
                    .child("New tasks start in Backlog."),
            )
            .into_any_element();
        Some(modal_frame(
            t,
            "board-new-task",
            "New task".into(),
            size::DIALOG_MD,
            fields,
            vec![
                Btn::new("board-new-cancel")
                    .label("Cancel")
                    .variant(Variant::Secondary)
                    .size(BtnSize::Xl)
                    .on_click(move |_, w, cx| {
                        let _ = e.update(cx, |ws, cx| {
                            ws.board_ui.new_task = None;
                            refocus(ws, w, cx);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
                Btn::new("board-new-add")
                    .label("Add task")
                    .variant(Variant::Primary)
                    .size(BtnSize::Xl)
                    .on_click(move |_, window, cx| {
                        let _ = e2.update(cx, |ws, cx| ws.board_add_task(window, cx));
                    })
                    .into_any_element(),
            ],
            {
                let e = cx.entity().downgrade();
                Rc::new(move |w, cx: &mut App| {
                    let _ = e.update(cx, |ws, cx| {
                        ws.board_ui.new_task = None;
                        if let Some(f) = ws.board_ui.focus.clone() {
                            w.focus(&f, cx);
                        }
                        cx.notify();
                    });
                })
            },
        ))
    }

    fn on_board_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The New task dialog owns the keys (its fields type; Tab switches; Esc closes).
        if let Some((title, body)) = self.board_ui.new_task.clone() {
            match e.keystroke.key.as_str() {
                "escape" => {
                    self.board_ui.new_task = None;
                    if let Some(f) = self.board_ui.focus.clone() {
                        window.focus(&f, cx);
                    }
                    cx.notify();
                    cx.stop_propagation();
                }
                "tab" => {
                    let next = if gpui_kit::Focusable::focus_handle(title.read(cx), cx)
                        .is_focused(window)
                    {
                        body
                    } else {
                        title
                    };
                    next.update(cx, |s, cx| s.focus(window, cx));
                    cx.stop_propagation();
                }
                _ => {}
            }
            return;
        }
        if self
            .board_ui
            .search
            .as_ref()
            .is_some_and(|s| gpui_kit::Focusable::focus_handle(s.read(cx), cx).is_focused(window))
        {
            if e.keystroke.key == "escape" {
                if let Some(f) = &self.board_ui.focus {
                    window.focus(f, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        let Some(board) = self.board_model(cx) else {
            return;
        };
        let lanes = filtered(&board, &self.board_ui.opts);
        if lanes.is_empty() {
            return;
        }
        let (mut l, mut c) = self.board_ui.opts.cursor.unwrap_or((0, 0));
        l = l.min(lanes.len() - 1);
        match e.keystroke.key.as_str() {
            "h" | "left" => l = l.saturating_sub(1),
            "l" | "right" => l = (l + 1).min(lanes.len() - 1),
            "k" | "up" => c = c.saturating_sub(1),
            "j" | "down" => c += 1,
            "enter" | "space" => {
                if let Some(card) = lanes[l].cards.get(c) {
                    let id = card.id.clone();
                    if let Some(a) = board.select_action.clone() {
                        self.board_action(&a, JsonValue::from(id.clone()), cx);
                    }
                    self.board_ui.detail = Some(id);
                }
            }
            "m" => self.board_ui.menu = Some(BoardMenu::Move),
            "c" => self.board_new_task(window, cx),
            "/" => {
                if let Some(s) = &self.board_ui.search {
                    s.update(cx, |s, cx| s.focus(window, cx));
                }
            }
            "escape" => {
                if self.board_ui.detail.take().is_none() && self.board_ui.menu.take().is_none() {
                    self.route = Route::Sessions;
                }
            }
            _ => return,
        }
        let n = lanes[l].cards.len();
        c = c.min(n.saturating_sub(1));
        self.board_ui.opts.cursor = Some((l, c));
        cx.stop_propagation();
        cx.notify();
    }

    /// A2.17 page header: project avatar tile, project name with the picker chevron, meta
    /// "5 tasks · ● 1 running", tools (refresh, show in chat, header crab).
    fn board_header(
        &mut self,
        board: Option<&kb::Board>,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let service = crate::kanboard::kanboard(cx);
        let project = self
            .board_ui
            .project
            .clone()
            .or_else(|| {
                self.board_pane()
                    .map(|p| workspace_label(&p.read(cx).model.cwd))
            })
            .unwrap_or_else(|| "Board".to_owned());
        let projects: Vec<(String, String)> = service
            .and_then(|s| s.projects().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.name.clone(), p.slug))
            .collect();
        let e = cx.entity().downgrade();
        let picker = popover(
            Btn::icon_only("board-project", IconName::ChevronDown, "Switch project")
                .size(BtnSize::Md)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board_ui.menu = Some(BoardMenu::Project);
                    cx.notify();
                })),
            self.board_ui.menu == Some(BoardMenu::Project),
            Placement::BelowStart,
            || {
                let entries = projects
                    .into_iter()
                    .map(|(label, slug)| {
                        let e = e.clone();
                        MenuEntry::item(label, move |_, cx| {
                            let slug = slug.clone();
                            let _ = e.update(cx, |ws, cx| {
                                ws.board_ui.menu = None;
                                ws.board_pick_project(&slug, cx);
                            });
                        })
                        .icon(IconName::Folder)
                    })
                    .collect();
                menu("board-projects", t, size::MENU_MIN, entries).into_any_element()
            },
            self.close_board_menu(cx),
        );
        let initial = project
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let avatar = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(size::CRAB_HEADER))
            .rounded(px(radius::R_LG))
            .bg(t.brand().opacity(0.14))
            .border_1()
            .border_color(t.brand().opacity(0.38))
            .text_role(t, TextRole::Sm)
            .weight_semibold()
            .text_color(t.fg())
            .child(SharedString::from(initial));
        let (tasks, running) = board.map_or((0, 0), |b| {
            (
                b.lanes.iter().map(|l| l.cards.len()).sum::<usize>(),
                b.lanes
                    .iter()
                    .filter(|l| l.id == "in_progress")
                    .map(|l| l.cards.len())
                    .sum::<usize>(),
            )
        });
        let meta = if running > 0 {
            format!(
                "{tasks} task{} · \u{25cf} {running} running",
                if tasks == 1 { "" } else { "s" }
            )
        } else {
            format!("{tasks} task{}", if tasks == 1 { "" } else { "s" })
        };
        let crab = self.crab(cx);
        let needs = self.sessions.needs_you().len();
        let has_board = board.is_some();
        PageHeader::new(SharedString::from(project))
            .lead(avatar)
            .title_slot(picker)
            .tool(
                Btn::icon_only("board-refresh", IconName::RefreshCw, "Refresh")
                    .size(BtnSize::Xl)
                    .disabled(!has_board)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.board_action("refresh", JsonValue::Null, cx);
                    })),
            )
            .tool(
                Btn::icon_only(
                    "board-show",
                    IconName::MessageSquare,
                    "Show the board in chat",
                )
                .size(BtnSize::Xl)
                .disabled(!has_board)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board_action("show", JsonValue::Null, cx);
                    this.route = Route::Sessions;
                    cx.notify();
                })),
            )
            .tool(
                div()
                    .id("board-crab")
                    .cursor_pointer()
                    .rounded(px(radius::R_FULL))
                    .hover(|s| s.bg(t.hover()))
                    .on_click(cx.listener(|this, _: &gpui_kit::ClickEvent, _, cx| {
                        this.needs_drawer = !this.needs_drawer;
                        cx.notify();
                    }))
                    .child(header_crab(crab, needs, cx)),
            )
            .window_controls(window, t)
            .meta(meta)
            .render(t)
            .into_any_element()
    }

    /// The picker sets the shown project; the next pane switch re-resolves it
    /// from the workspace's cwd (round-5: the board follows the workspace, not
    /// an extension panel).
    fn board_pick_project(&mut self, slug: &str, cx: &mut Context<Self>) {
        self.board_ui.project = Some(slug.to_owned());
        self.board_ui.detail = None;
        cx.notify();
    }

    /// Resolves the board project once per render (no extension round-trip).
    pub(crate) fn ensure_board(&mut self, cx: &mut Context<Self>) {
        let _ = self.board_project(cx);
    }

    fn close_board_menu(&self, cx: &mut Context<Self>) -> impl Fn(&mut Window, &mut App) + 'static {
        let e = cx.entity().downgrade();
        move |_, cx| {
            let _ = e.update(cx, |ws, cx| {
                ws.board_ui.menu = None;
                cx.notify();
            });
        }
    }

    fn board_toolbar(
        &mut self,
        board: Option<&kb::Board>,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let search = self
            .board_ui
            .search
            .get_or_insert_with(|| {
                cx.new(|cx| InputState::new(window, cx).placeholder("Search tasks  /"))
            })
            .clone();
        if self.board_ui.focus.is_none() {
            self.board_ui.focus = Some(cx.focus_handle());
            cx.subscribe(&search, |ws, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    ws.board_ui.opts.query = input.read(cx).value().to_string();
                    ws.board_ui.opts.cursor = None;
                    cx.notify();
                }
            })
            .detach();
        }
        let e = cx.entity().downgrade();
        let opts = self.board_ui.opts.clone();
        let filter_label = match opts.filter.as_deref() {
            Some("blocked") => "Blocked",
            Some("high") => "High priority",
            Some("claimed") => "Claimed by an agent",
            _ => "All tasks",
        };
        let set_filter = |f: Option<&'static str>| {
            let e = e.clone();
            move |_: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |ws, cx| {
                    ws.board_ui.opts.filter = f.map(str::to_owned);
                    ws.board_ui.opts.cursor = None;
                    ws.board_ui.menu = None;
                    cx.notify();
                });
            }
        };
        let filter = popover(
            Btn::new("board-filter")
                .label(SharedString::from(filter_label))
                .icon(IconName::Funnel)
                .size(BtnSize::Md)
                .variant(Variant::Secondary)
                .selected(opts.filter.is_some())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board_ui.menu = Some(BoardMenu::Filter);
                    cx.notify();
                })),
            self.board_ui.menu == Some(BoardMenu::Filter),
            Placement::BelowStart,
            || {
                menu(
                    "board-filter-menu",
                    t,
                    size::MENU_MIN,
                    vec![
                        MenuEntry::item("All tasks", set_filter(None))
                            .checked(opts.filter.is_none()),
                        MenuEntry::item("Blocked", set_filter(Some("blocked")))
                            .checked(opts.filter.as_deref() == Some("blocked")),
                        MenuEntry::item("High priority", set_filter(Some("high")))
                            .checked(opts.filter.as_deref() == Some("high")),
                        MenuEntry::item("Claimed by an agent", set_filter(Some("claimed")))
                            .checked(opts.filter.as_deref() == Some("claimed")),
                    ],
                )
                .into_any_element()
            },
            self.close_board_menu(cx),
        );
        let toggle = |f: fn(&mut BoardOptions)| {
            let e = e.clone();
            move |_: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |ws, cx| {
                    f(&mut ws.board_ui.opts);
                    ws.board_ui.opts.cursor = None;
                    cx.notify();
                });
            }
        };
        let display = popover(
            Btn::new("board-display")
                .label("Display")
                .icon(IconName::SlidersHorizontal)
                .size(BtnSize::Md)
                .variant(Variant::Secondary)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board_ui.menu = Some(BoardMenu::Display);
                    cx.notify();
                })),
            self.board_ui.menu == Some(BoardMenu::Display),
            Placement::BelowStart,
            || {
                menu(
                    "board-display-menu",
                    t,
                    size::MENU_MIN,
                    vec![
                        MenuEntry::item(
                            "Show Backlog",
                            toggle(|o| o.show_backlog = !o.show_backlog),
                        )
                        .checked(opts.show_backlog),
                        MenuEntry::item(
                            "Show Done and Cancelled",
                            toggle(|o| o.hide_done = !o.hide_done),
                        )
                        .checked(!opts.hide_done),
                    ],
                )
                .into_any_element()
            },
            self.close_board_menu(cx),
        );
        // Move menu of the focused (or selected) task.
        let moves = board.map(|b| b.moves.clone()).unwrap_or_default();
        let move_menu = popover(
            Btn::new("board-move")
                .label("Move")
                .icon(IconName::ArrowRight)
                .kbd("m")
                .size(BtnSize::Md)
                .variant(Variant::Ghost)
                .disabled(moves.is_empty())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board_ui.menu = Some(BoardMenu::Move);
                    cx.notify();
                })),
            self.board_ui.menu == Some(BoardMenu::Move) && !moves.is_empty(),
            Placement::BelowStart,
            || {
                let entries = moves
                    .iter()
                    .map(|(label, action)| {
                        let e = e.clone();
                        let action = action.clone();
                        MenuEntry::item(label.clone(), move |_, cx| {
                            let action = action.clone();
                            let _ = e.update(cx, |ws, cx| {
                                ws.board_ui.menu = None;
                                ws.board_action(&action, JsonValue::Null, cx);
                            });
                        })
                    })
                    .collect();
                menu("board-move-menu", t, size::MENU_MIN, entries).into_any_element()
            },
            self.close_board_menu(cx),
        );
        // A2.17 toolbar pills: Filter, Display, search, Move (refresh / show live in the
        // header tools, New task is the FAB).
        div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .px(px(size::PAGE_GUTTER))
            .pb(px(space::S3))
            .child(filter)
            .child(display)
            .child(div().w(px(size::SIDEBAR)).child(Input::new(&search)))
            .child(move_menu)
            .into_any_element()
    }

    /// The board frame.
    pub(crate) fn render_board(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        self.ensure_board(cx);
        let board = self.board_model(cx);
        let header = self.board_header(board.as_ref(), &t, window, cx);
        let toolbar = self.board_toolbar(board.as_ref(), &t, window, cx);
        let focus = self.board_ui.focus.clone();
        if let Some(f) = &focus
            && window.focused(cx).is_none()
        {
            window.focus(f, cx);
        }
        let service = crate::kanboard::kanboard(cx);
        #[allow(
            clippy::single_match_else,
            reason = "the empty-state arm is the meat, not the default"
        )]
        let body = match &board {
            Some(b) => {
                let mut opts = self.board_ui.opts.clone();
                opts.add_row = true;
                // #8: drag state only matters while a drag is live — clearing
                // here also sweeps a drag that ended without a drop.
                if cx.has_active_drag() {
                    opts.dragging = self.board_ui.drag_status.clone();
                    opts.hover_lane = self.board_ui.hover_lane.clone();
                } else {
                    self.board_ui.drag_status = None;
                    self.board_ui.hover_lane = None;
                }
                let ws = cx.entity().downgrade();
                // Native actions: "add" opens the dialog, "select" the overlay,
                // "move:<x>" runs in-process — all through `board_action`.
                let on_action: ous::ActionFn = Rc::new(move |action, value, cx| {
                    let value = value.clone();
                    let action = action.to_owned();
                    if action == "add" {
                        let ws = ws.clone();
                        cx.defer(move |cx| {
                            if let Some(ws) = ws.upgrade()
                                && let Some(window) = cx.active_window()
                            {
                                let _ = window.update(cx, |_, window, cx| {
                                    ws.update(cx, |ws, cx| ws.board_new_task(window, cx));
                                });
                            }
                        });
                    } else {
                        let _ = ws.update(cx, |ws, cx| ws.board_action(&action, value, cx));
                    }
                });
                let mut o = Ous {
                    t,
                    on_action: Rc::clone(&on_action),
                    inputs: None,
                    cols: crate::components::ous_cols(self.window_width),
                    roomy: true,
                };
                let lanes = ous::kanban::board(&mut o, b, &opts, "board", window, cx);
                let visible = filtered(b, &self.board_ui.opts);
                let e = cx.entity().downgrade();
                let strip = ous::kanban::lane_strip(
                    &t,
                    &visible,
                    self.board_ui.opts.cursor.map(|(l, _)| l),
                    "board",
                    Rc::new(move |i, _, cx| {
                        let _ = e.update(cx, |ws, cx| {
                            ws.board_ui.opts.cursor = Some((i, 0));
                            cx.notify();
                        });
                    }),
                );
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap(px(space::S4))
                    .px(px(size::PAGE_GUTTER))
                    .child(strip)
                    .child(div().flex_1().min_h_0().child(lanes))
                    .into_any_element()
            }
            // No service → the board home cannot be resolved; a registered-but-
            // unreadable project → the error; an unregistered cwd → offer `init`.
            None => {
                let e = cx.entity().downgrade();
                match &service {
                    None => empty_state(
                        &t,
                        IconName::Kanban,
                        "Board is off",
                        Some("The board state directory cannot be resolved.".into()),
                        Vec::new(),
                    )
                    .into_any_element(),
                    Some(_) if self.board_ui.project.is_none() => {
                        let cwd = self
                            .board_pane()
                            .map(|p| p.read(cx).model.cwd.clone())
                            .unwrap_or_default();
                        let label = workspace_label(&cwd);
                        empty_state(
                            &t,
                            IconName::Kanban,
                            "No board here yet",
                            Some(
                                format!("{label} is not a board project. Set one up, or pick another project above.")
                                    .into(),
                            ),
                            vec![
                                Btn::new("board-init")
                                    .label("Set up the board")
                                    .variant(Variant::Primary)
                                    .size(BtnSize::Xl)
                                    .on_click(move |_, _, cx| {
                                        let cwd = cwd.clone();
                                        let _ = e.update(cx, |ws, cx| {
                                            let result = crate::kanboard::kanboard(cx)
                                                .map(|s| s.register(std::path::Path::new(&cwd)));
                                            if let Some(Ok(project)) = result {
                                                ws.board_ui.project = Some(project.slug);
                                            } else if let Some(Err(err)) = result {
                                                ws.toast(
                                                    ToastKind::Error,
                                                    None,
                                                    err.to_string(),
                                                    cx,
                                                );
                                            }
                                            cx.notify();
                                        });
                                    })
                                    .into_any_element(),
                            ],
                        )
                        .into_any_element()
                    }
                    Some(_) => empty_state(
                        &t,
                        IconName::Kanban,
                        "The board could not be read",
                        self.board_ui.error.clone().map(SharedString::from),
                        Vec::new(),
                    )
                    .into_any_element(),
                }
            }
        };
        // The selected task's detail as an overlay (Enter / click), not inline.
        let selected = board.as_ref().and_then(|b| {
            b.lanes
                .iter()
                .flat_map(|l| &l.cards)
                .find(|c| Some(c.id.as_str()) == self.board_ui.detail.as_deref())
                .map(|c| (c.id.clone(), c.title.clone()))
        });
        let overlay = match (&board, &self.board_ui.detail, &selected) {
            (Some(b), Some(open), Some((id, title))) if open == id => {
                b.detail.clone().map(|detail| {
                    let ws = cx.entity().downgrade();
                    let on_action: ous::ActionFn = Rc::new(move |action, value, cx| {
                        let action = action.to_owned();
                        let value = value.clone();
                        let _ = ws.update(cx, |ws, cx| ws.board_action(&action, value, cx));
                    });
                    let mut o = Ous {
                        t,
                        on_action,
                        inputs: None,
                        cols: crate::components::ous_cols(px(size::DIALOG_MD)),
                        roomy: true,
                    };
                    let rendered = ous::render(&detail, "board/detail", &mut o, window, cx);
                    let e = cx.entity().downgrade();
                    modal_frame(
                        &t,
                        "board-detail",
                        SharedString::from(format!("{id} · {title}")),
                        size::DIALOG_MD,
                        rendered,
                        Vec::new(),
                        Rc::new(move |_, cx: &mut App| {
                            let _ = e.update(cx, |ws, cx| {
                                ws.board_ui.detail = None;
                                cx.notify();
                            });
                        }),
                    )
                })
            }
            _ => None,
        };
        let new_task = self.new_task_dialog(&t, cx);
        let fab = board.is_some().then(|| {
            div()
                .id("board-fab")
                .absolute()
                .right(px(space::S6))
                .bottom(px(space::S6))
                .flex()
                .items_center()
                .justify_center()
                .size(px(size::FAB))
                .rounded(px(radius::R_CARD))
                .bg(t.primary())
                .shadow(t.e_fab())
                .cursor_pointer()
                .hover(|s| s.bg(t.primary_hover()))
                .tooltip(crate::components::tooltip::tip(
                    "New task",
                    Some("c".into()),
                ))
                .on_click(cx.listener(|this, _, window, cx| this.board_new_task(window, cx)))
                .child(crate::design::icons::icon(
                    IconName::Plus,
                    size::ICON_LG,
                    t.primary_fg(),
                ))
                // Pointer-only surface (STATUS.md §1b.7 gap 1): the FAB has no keyboard path.
                .automation(crate::automation::Meta::icon_button("New task"))
        });
        div()
            .id("board")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .when_some(focus, |el, f| el.track_focus(&f))
            .on_key_down(cx.listener(Self::on_board_key))
            .child(header)
            .when(board.is_some(), |el| el.child(toolbar))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .pb(px(space::S4))
                    .child(body),
            )
            .children(fab)
            .children(overlay)
            .children(new_task)
            .into_any_element()
    }
}
