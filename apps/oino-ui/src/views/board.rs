//! D9 Board (design.md §5.8): the `unipi.kanboard/board` panel promoted to the main area.
//!
//! Header "Board · <project ▾>" (picker over the sidebar's workspaces), a toolbar with Filter,
//! Display, search and `+ New task` (plus the extension's own keys), the native lanes with a
//! keyboard cursor, and the selected task's detail as an overlay.
//!
//! Keys (board focused): h/j/k/l or arrows move, Enter opens the task, `m` move menu,
//! `c` new task, `/` search, Esc closes the overlay / leaves the board.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use oino_ai::JsonValue;

use crate::components::badge::pill;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::dialog::modal_frame;
use crate::components::empty::empty_state;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::ous::kanban::{BoardOptions, filtered};
use crate::components::ous::{self, Ous};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::kanban::{self as kb, BOARD_PANEL};
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
    focus: Option<FocusHandle>,
    /// Open menu.
    pub menu: Option<BoardMenu>,
    /// Task id whose detail overlay is shown.
    pub detail: Option<String>,
    /// The extension's selection last frame (a change opens the overlay).
    last_selected: Option<String>,
}

impl Workspace {
    fn board_pane(&self) -> Option<Entity<SessionPane>> {
        self.active_pane().cloned()
    }

    fn board_action(&self, action: &str, value: JsonValue, cx: &mut Context<Self>) {
        if let Some(p) = self.board_pane() {
            let action = action.to_owned();
            p.update(cx, |p, cx| p.panel_action(BOARD_PANEL, &action, value, cx));
        }
    }

    fn board_model(&self, cx: &App) -> Option<kb::Board> {
        let p = self.board_pane()?;
        let p = p.read(cx);
        let panel = p.model.ui.panel(BOARD_PANEL)?;
        kb::recognise(&panel.spec)
    }

    fn board_new_task(&mut self, cx: &mut Context<Self>) {
        let has_add = self
            .board_pane()
            .and_then(|p| {
                p.read(cx)
                    .model
                    .ui
                    .panel(BOARD_PANEL)
                    .map(|panel| panel.keys.iter().any(|k| k.action_id == "add"))
            })
            .unwrap_or(false);
        if has_add {
            self.board_action("add", JsonValue::Null, cx);
        } else {
            self.with_pane(cx, |p, cx| p.set_composer_text("/kanboard add ", cx));
            self.route = Route::Sessions;
        }
        cx.notify();
    }

    fn on_board_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
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
            "c" => self.board_new_task(cx),
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

    fn board_header(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let project = self
            .board_pane()
            .map(|p| workspace_label(&p.read(cx).model.cwd))
            .unwrap_or_default();
        let groups: Vec<(String, String)> = self
            .sessions
            .groups()
            .into_iter()
            .map(|g| (g.cwd, g.label))
            .collect();
        let e = cx.entity().downgrade();
        let picker = popover(
            Btn::new("board-project")
                .label(SharedString::from(project))
                .icon(IconName::Folder)
                .trailing(IconName::ChevronDown)
                .size(BtnSize::Sm)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.board_ui.menu = Some(BoardMenu::Project);
                    cx.notify();
                })),
            self.board_ui.menu == Some(BoardMenu::Project),
            Placement::BelowStart,
            || {
                let entries = groups
                    .into_iter()
                    .map(|(cwd, label)| {
                        let e = e.clone();
                        MenuEntry::item(label, move |window, cx| {
                            let cwd = cwd.clone();
                            let _ = e.update(cx, |ws, cx| {
                                ws.board_ui.menu = None;
                                ws.board_open_project(&cwd, window, cx);
                            });
                        })
                        .icon(IconName::Folder)
                    })
                    .collect();
                menu("board-projects", t, size::MENU_MIN, entries).into_any_element()
            },
            self.close_board_menu(cx),
        );
        div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::HEADER))
            .px(px(space::S3))
            .border_b_1()
            .border_color(t.border())
            .child(
                div()
                    .text_role(t, TextRole::Base)
                    .weight_medium()
                    .child("Board"),
            )
            .child(div().text_color(t.fg_subtlest()).child("·"))
            .child(picker)
            .child(div().flex_1())
            .child(
                Btn::new("board-back")
                    .label("Back to sessions")
                    .variant(Variant::Ghost)
                    .icon(IconName::ArrowLeft)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.route = Route::Sessions;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    /// Opens the board of another workspace: an open pane there, else a new session.
    fn board_open_project(&mut self, cwd: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self
            .panes
            .iter()
            .find(|p| p.entity.read(cx).model.cwd == cwd)
            .map(|p| p.key)
        {
            self.focus_pane(key, window, cx);
        } else {
            self.new_session_in(cwd, window, cx);
        }
        self.with_pane(cx, |p, cx| p.run_prompt("/kanboard", cx));
        self.board_ui = BoardUi::default();
        cx.notify();
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
                .size(BtnSize::Sm)
                .variant(Variant::Outline)
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
                .size(BtnSize::Sm)
                .variant(Variant::Outline)
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
                .size(BtnSize::Sm)
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
        // The extension's own keys (Refresh, Show in chat…), minus the ones the toolbar owns.
        let ext_keys: Vec<AnyElement> = self
            .board_pane()
            .and_then(|p| {
                p.read(cx)
                    .model
                    .ui
                    .panel(BOARD_PANEL)
                    .map(|panel| panel.keys.clone())
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|k| k.action_id != "add")
            .map(|k| {
                let action = k.action_id.clone();
                Btn::new(SharedString::from(format!("board-key-{action}")))
                    .label(SharedString::from(k.label.clone()))
                    .kbd(SharedString::from(k.key.clone()))
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.board_action(&action, JsonValue::Null, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::CONTROL_LG + space::S2))
            .px(px(space::S3))
            .border_b_1()
            .border_color(t.border())
            .child(filter)
            .child(display)
            .child(div().w(px(size::SIDEBAR)).child(Input::new(&search)))
            .child(move_menu)
            .child(div().flex_1())
            .children(ext_keys)
            .child(
                Btn::new("board-new")
                    .label("New task")
                    .icon(IconName::Plus)
                    .kbd("c")
                    .variant(Variant::Primary)
                    .size(BtnSize::Sm)
                    .on_click(cx.listener(|this, _, _, cx| this.board_new_task(cx))),
            )
            .into_any_element()
    }

    /// The board frame.
    pub(crate) fn render_board(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let pane = self.board_pane();
        let board = self.board_model(cx);
        let header = self.board_header(&t, cx);
        let toolbar = self.board_toolbar(board.as_ref(), &t, window, cx);
        let focus = self.board_ui.focus.clone();
        if let Some(f) = &focus
            && window.focused(cx).is_none()
        {
            window.focus(f, cx);
        }
        let body = match (&pane, &board) {
            (Some(p), Some(b)) => {
                let opts = self.board_ui.opts.clone();
                let on_action = p.update(cx, |p, cx| p.action_fn(BOARD_PANEL, cx));
                let lanes = p.update(cx, |p, cx| {
                    let mut o = Ous {
                        t,
                        on_action: Rc::clone(&on_action),
                        inputs: Some(&mut p.inputs),
                        cols: crate::components::ous_cols(p.width),
                        roomy: true,
                    };
                    ous::kanban::board(&mut o, b, &opts, BOARD_PANEL, window, cx)
                });
                div()
                    .flex_1()
                    .min_h_0()
                    .p(px(space::S4))
                    .child(lanes)
                    .into_any_element()
            }
            _ => empty_state(
                &t,
                IconName::Kanban,
                "Opening the board…",
                Some("Runs /kanboard in the active session.".into()),
                Vec::new(),
            )
            .into_any_element(),
        };
        // The selected task's detail as an overlay (Enter / click), not inline.
        let selected = board.as_ref().and_then(|b| {
            b.lanes
                .iter()
                .flat_map(|l| &l.cards)
                .find(|c| c.selected)
                .map(|c| (c.id.clone(), c.title.clone()))
        });
        let overlay = match (&pane, &board, &self.board_ui.detail, &selected) {
            (Some(p), Some(b), Some(open), Some((id, title))) if open == id => {
                b.detail.clone().map(|detail| {
                    let rendered = p.update(cx, |p, cx| {
                        let on_action = p.action_fn(BOARD_PANEL, cx);
                        let mut o = Ous {
                            t,
                            on_action,
                            inputs: Some(&mut p.inputs),
                            cols: crate::components::ous_cols(px(size::DIALOG_MD)),
                            roomy: true,
                        };
                        ous::render(&detail, "board/detail", &mut o, window, cx)
                    });
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
        // Clicking a card selects it in the extension; open its overlay on that change.
        if let Some((id, _)) = &selected
            && self.board_ui.last_selected.as_deref() != Some(id.as_str())
        {
            if self.board_ui.last_selected.is_some() {
                self.board_ui.detail = Some(id.clone());
            }
            self.board_ui.last_selected = Some(id.clone());
        }
        let ready = board.as_ref().map_or(0, kb::ready_count);
        div()
            .id("board")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.frame())
            .border_1()
            .border_color(t.border())
            .rounded(px(radius::R_CARD))
            .overflow_hidden()
            .when_some(focus, |el, f| el.track_focus(&f))
            .on_key_down(cx.listener(Self::on_board_key))
            .child(header)
            .child(toolbar)
            .when(ready > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .px(px(space::S4))
                        .pt(px(space::S3))
                        .text_role(&t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(pill(&t, t.accent(), t.fg(), format!("{ready} ready")))
                        .child("h j k l move · Enter open · m move · c new task · / search"),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
            .children(overlay)
            .into_any_element()
    }
}
