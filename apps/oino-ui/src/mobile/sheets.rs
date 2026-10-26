//! M6 Bottom sheets (design.md §6.6, shapes A1.2): `frame` bg, 24 top corners, drag handle
//! 36 × 4 `border_hover`, title `Lg`/500, max 90 % of the visible height. Model (rows 56 by
//! provider, thinking as a six-segment control), Actions (chat, panels, extension commands
//! and shortcuts, status chips), Status, Rename, Confirm, row menus, Display — and M3
//! interaction sheets for blocking cards that are too tall to dock.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, ClipboardItem, Context, ElementId, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, relative,
};
use oino_ai::ThinkingLevel;

use crate::components::badge::{section_label, tone_dot};
use crate::components::button::Variant;
use crate::components::controls::{LEVELS, level_label};
use crate::components::interaction::InteractionView;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::tokens::touch;
use crate::design::{TextRole, radius, size, space};
use crate::mobile::shell::{MobileShell, Page, SettingsPage, Sheet, pill_btn};
use crate::model::status::RunKind;
use crate::views::session_pane::{SessionPane, SideTab};

/// Type of a sheet row action.
type RowAction = Rc<dyn Fn(&mut MobileShell, &mut Window, &mut Context<MobileShell>)>;

impl MobileShell {
    /// The sheet frame: scrim (tap closes when `dismissible`), handle, title, scrolling body.
    pub(crate) fn sheet_frame(
        &self,
        t: &OinoTheme,
        id: &'static str,
        title: Option<SharedString>,
        body: AnyElement,
        footer: Option<AnyElement>,
        dismissible: bool,
        full: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (hw, hh) = touch::HANDLE;
        div()
            .id("m-sheet-scrim")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .justify_end()
            .bg(t.scrim())
            .when(dismissible, |el| {
                el.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_sheet(cx)))
            })
            .child(
                div()
                    .id(id)
                    .w_full()
                    .flex()
                    .flex_col()
                    .max_h(relative(touch::SHEET_MAX))
                    .when(full, |el| el.h(relative(touch::SHEET_MAX)))
                    .rounded_tl(px(radius::R_SHEET))
                    .rounded_tr(px(radius::R_SHEET))
                    .bg(t.frame())
                    .border_t_1()
                    .border_color(t.border())
                    .shadow(t.e3())
                    .occlude()
                    // Taps inside the sheet must not reach the scrim.
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .id("m-sheet-handle")
                            .flex()
                            .justify_center()
                            .w_full()
                            .pt(px(space::S2))
                            .pb(px(space::S1))
                            .when(dismissible, |el| {
                                // Drag the handle down to dismiss.
                                el.on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                                    let d = e.delta.pixel_delta(px(touch::ROW));
                                    if f32::from(d.y) > space::S6 {
                                        this.close_sheet(cx);
                                    }
                                }))
                            })
                            .child(
                                div()
                                    .w(px(hw))
                                    .h(px(hh))
                                    .rounded(px(radius::R_FULL))
                                    .bg(t.border_hover()),
                            ),
                    )
                    .when_some(title, |el, title| {
                        el.child(
                            div()
                                .px(px(space::S4))
                                .pt(px(space::S1))
                                .pb(px(space::S2))
                                .text_role(t, TextRole::Lg)
                                .weight_medium()
                                .text_color(t.fg())
                                .child(title),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-body")))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .pb(px(space::S3))
                            .child(body),
                    )
                    .children(footer.map(|f| {
                        div()
                            .flex_none()
                            .px(px(space::S4))
                            .pt(px(space::S2))
                            .pb(px(space::S4))
                            .border_t_1()
                            .border_color(t.border())
                            .child(f)
                    })),
            )
            .into_any_element()
    }

    /// A 56-high sheet row running `f` (the sheet closes first).
    fn action_row(
        t: &OinoTheme,
        id: impl Into<ElementId>,
        glyph: IconName,
        label: impl Into<SharedString>,
        detail: Option<String>,
        f: RowAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Self::list_row(t, id, Some(glyph), label, detail, None)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.sheet = None;
                f(this, window, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    pub(crate) fn render_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let t = OinoTheme::get(cx);
        if self.sheet.is_none()
            && let Some(view) = self.sheet_interaction(cx)
        {
            return Some(self.interaction_sheet(&t, &view, window, cx));
        }
        let sheet = self.sheet.clone()?;
        let pane = self.active_pane().cloned();
        Some(match (sheet, pane) {
            (Sheet::Model, Some(p)) => self.model_sheet(&t, &p, cx),
            (Sheet::Actions, Some(p)) => self.actions_sheet(&t, &p, cx),
            (Sheet::Status, Some(p)) => self.status_sheet(&t, &p, cx),
            (Sheet::Rename(id), _) => self.rename_sheet(&t, &id, window, cx),
            (Sheet::Confirm(c), _) => {
                let run = Rc::clone(&c.run);
                let body = div()
                    .px(px(space::S4))
                    .text_role(&t, TextRole::Base)
                    .text_color(t.fg_subtle())
                    .child(c.body.clone())
                    .into_any_element();
                let footer = div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S2))
                    .child(
                        pill_btn(
                            "m-confirm-ok",
                            c.action.clone(),
                            Some(IconName::Trash),
                            Variant::Danger,
                        )
                        .full()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.sheet = None;
                                run(this, window, cx);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        pill_btn("m-confirm-cancel", "Cancel", None, Variant::Outline)
                            .full()
                            .on_click(cx.listener(|this, _, _, cx| this.close_sheet(cx))),
                    )
                    .into_any_element();
                self.sheet_frame(
                    &t,
                    "m-confirm",
                    Some(c.title.clone()),
                    body,
                    Some(footer),
                    true,
                    false,
                    cx,
                )
            }
            (Sheet::RowMenu(id), _) => self.row_menu(&t, &id, cx),
            (Sheet::CardMenu(id), _) => self.card_menu(&t, &id, window, cx),
            (Sheet::Edit { ns, key, label }, _) => {
                self.edit_sheet(&t, &ns, &key, &label, window, cx)
            }
            (Sheet::Display, _) => self.display_sheet(&t, cx),
            _ => return None,
        })
    }

    fn interaction_sheet(
        &mut self,
        t: &OinoTheme,
        view: &Entity<InteractionView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dismissible = view.read(cx).touch_dismissible();
        let h = window.viewport_size().height * (touch::SHEET_MAX * 0.75);
        view.update(cx, |v, _| v.set_max_height(h));
        let body = div()
            .px(px(space::S3))
            .child(view.clone())
            .into_any_element();
        let id: &'static str = "m-interaction-sheet";
        // Not dismissible while pending (approval / question / plan): no scrim close.
        let frame = self.sheet_frame(t, id, None, body, None, false, false, cx);
        if dismissible {
            let v = view.clone();
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .id("m-dialog-scrim")
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            v.update(cx, InteractionView::dismiss);
                        })),
                )
                .child(frame)
                .into_any_element()
        } else {
            frame
        }
    }

    fn model_sheet(
        &mut self,
        t: &OinoTheme,
        pane: &Entity<SessionPane>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (models, current, thinking) = {
            let p = pane.read(cx);
            (p.models.clone(), p.model.model.clone(), p.model.thinking)
        };
        let mut providers: Vec<String> = models.iter().map(|m| m.provider.clone()).collect();
        providers.dedup();
        let mut rows: Vec<AnyElement> = Vec::new();
        for provider in providers {
            rows.push(Self::section(t, provider.clone()));
            for m in models.iter().filter(|m| m.provider == provider) {
                let is_current = current
                    .as_ref()
                    .is_some_and(|c| c.id == m.id && c.provider == m.provider);
                let label = if m.name.is_empty() {
                    m.id.clone()
                } else {
                    m.name.clone()
                };
                let detail = format!(
                    "{}k context{}",
                    m.context_window / 1000,
                    if m.reasoning { " · reasoning" } else { "" }
                );
                let (prov, id) = (m.provider.clone(), m.id.clone());
                rows.push(
                    Self::list_row(
                        t,
                        ElementId::Name(format!("m-model-{prov}-{id}").into()),
                        None,
                        label,
                        Some(detail),
                        is_current.then(|| {
                            icon(IconName::Check, size::ICON, t.primary()).into_any_element()
                        }),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let (prov, id) = (prov.clone(), id.clone());
                        this.with_pane(cx, |p, cx| p.set_model(prov, id, cx));
                        this.close_sheet(cx);
                    }))
                    .into_any_element(),
                );
            }
        }
        if rows.is_empty() {
            rows.push(
                Self::list_row(
                    t,
                    "m-model-login",
                    Some(IconName::Lock),
                    "Log in to a provider",
                    Some("No models with credentials yet".into()),
                    Some(Self::chevron(t)),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.close_sheet(cx);
                    this.with_pane(cx, |p, cx| p.run_prompt("/login", cx));
                }))
                .into_any_element(),
            );
        }
        rows.push(
            Self::list_row(
                t,
                "m-model-manage",
                Some(IconName::SlidersHorizontal),
                "Manage models",
                None,
                Some(Self::chevron(t)),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_settings(SettingsPage::Models, cx);
            }))
            .into_any_element(),
        );
        let footer = div()
            .flex()
            .flex_col()
            .gap(px(space::S2))
            .child(section_label(t, "Thinking"))
            .child(thinking_segments(t, thinking, cx))
            .into_any_element();
        let body = div().flex().flex_col().children(rows).into_any_element();
        self.sheet_frame(
            t,
            "m-model-sheet",
            Some("Model".into()),
            body,
            Some(footer),
            true,
            false,
            cx,
        )
    }

    fn actions_sheet(
        &mut self,
        t: &OinoTheme,
        pane: &Entity<SessionPane>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (busy, commands, shortcuts, panels, statuses, id, has_board) = {
            let p = pane.read(cx);
            (
                p.model.is_busy(),
                p.commands().to_vec(),
                p.model.ui.shortcuts.clone(),
                p.side_panels(),
                p.model
                    .ui
                    .statuses()
                    .into_iter()
                    .map(|s| (s.text.clone(), s.tone))
                    .collect::<Vec<_>>(),
                p.model.id.clone(),
                p.has_command("unipi:kanboard"),
            )
        };
        let act = |f: fn(&mut SessionPane, &mut Context<SessionPane>)| -> RowAction {
            Rc::new(
                move |this: &mut MobileShell, _: &mut Window, cx: &mut Context<MobileShell>| {
                    this.with_pane(cx, f);
                },
            )
        };
        let mut rows: Vec<AnyElement> = vec![Self::section(t, "Chat")];
        rows.push(Self::action_row(
            t,
            "m-act-new",
            IconName::SquarePen,
            "New session",
            None,
            Rc::new(|this, window, cx| this.new_session(None, window, cx)),
            cx,
        ));
        if busy {
            rows.push(Self::action_row(
                t,
                "m-act-abort",
                IconName::Square,
                "Stop",
                Some("Abort the run".into()),
                act(SessionPane::abort),
                cx,
            ));
        }
        rows.push(Self::action_row(
            t,
            "m-act-compact",
            IconName::Archive,
            "Compact",
            Some("Summarise older turns".into()),
            act(SessionPane::compact),
            cx,
        ));
        rows.push(Self::action_row(
            t,
            "m-act-fork",
            IconName::GitFork,
            "Fork",
            Some("New session from here".into()),
            act(SessionPane::fork_session),
            cx,
        ));
        rows.push(Self::action_row(
            t,
            "m-act-tree",
            IconName::ListTree,
            "Tree",
            Some("Branches of this session".into()),
            Rc::new(|this, _, cx| {
                this.with_pane(cx, |p, cx| p.open_side(SideTab::Tree, cx));
            }),
            cx,
        ));
        rows.push(Self::action_row(
            t,
            "m-act-find",
            IconName::Search,
            "Find in conversation",
            None,
            Rc::new(|this, window, cx| {
                if let Some(p) = this.active_pane().cloned() {
                    p.update(cx, |p, cx| p.open_find(window, cx));
                }
            }),
            cx,
        ));
        let rename_id = id.clone();
        rows.push(Self::action_row(
            t,
            "m-act-rename",
            IconName::Pencil,
            "Rename",
            None,
            Rc::new(move |this, _, cx| this.open_sheet(Sheet::Rename(rename_id.clone()), cx)),
            cx,
        ));
        rows.push(Self::section(t, "Panels"));
        rows.push(Self::action_row(
            t,
            "m-act-review",
            IconName::FilePen,
            "Review changes",
            None,
            Rc::new(|this, _, cx| {
                this.with_pane(cx, |p, cx| p.open_side(SideTab::Review, cx));
            }),
            cx,
        ));
        if has_board {
            rows.push(Self::action_row(
                t,
                "m-act-board",
                IconName::Kanban,
                "Board",
                None,
                Rc::new(|this, _, cx| {
                    if let Some(key) = this.active_key() {
                        this.open_board(key, true, cx);
                    }
                }),
                cx,
            ));
        }
        for (pid, title) in panels {
            let open = pid.clone();
            rows.push(Self::action_row(
                t,
                ElementId::Name(format!("m-act-panel-{pid}").into()),
                IconName::PanelRight,
                title,
                None,
                Rc::new(move |this, _, cx| {
                    let open = open.clone();
                    this.with_pane(cx, |p, cx| p.open_side(SideTab::Panel(open), cx));
                }),
                cx,
            ));
        }
        let ext: Vec<_> = commands
            .iter()
            .filter(|(n, _)| n.contains(':'))
            .cloned()
            .collect();
        if !ext.is_empty() {
            rows.push(Self::section(t, "Extensions"));
            for (name, description) in ext {
                let cmd = format!("/{name}");
                rows.push(Self::action_row(
                    t,
                    ElementId::Name(format!("m-act-cmd-{name}").into()),
                    IconName::SquareTerminal,
                    cmd.clone(),
                    description,
                    Rc::new(move |this, _, cx| {
                        let cmd = cmd.clone();
                        this.with_pane(cx, |p, cx| p.run_prompt(&cmd, cx));
                    }),
                    cx,
                ));
            }
        }
        if !shortcuts.is_empty() {
            rows.push(Self::section(t, "Shortcuts"));
            for s in shortcuts {
                let sid = s.id.clone();
                rows.push(Self::action_row(
                    t,
                    ElementId::Name(format!("m-act-sc-{}", s.id).into()),
                    IconName::Zap,
                    s.description.clone(),
                    None,
                    Rc::new(move |this, _, cx| {
                        let sid = sid.clone();
                        this.with_pane(cx, |p, cx| p.invoke_shortcut(&sid, cx));
                    }),
                    cx,
                ));
            }
        }
        rows.push(Self::section(t, "Session"));
        let copy = id.clone();
        rows.push(Self::action_row(
            t,
            "m-act-copy",
            IconName::Copy,
            "Copy session id",
            None,
            Rc::new(move |this, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                this.toast(
                    crate::components::toast::ToastKind::Success,
                    None,
                    "Session id copied",
                    cx,
                );
            }),
            cx,
        ));
        rows.push(Self::action_row(
            t,
            "m-act-export",
            IconName::ExternalLink,
            "Export HTML",
            None,
            act(SessionPane::export_html),
            cx,
        ));
        rows.push(Self::action_row(
            t,
            "m-act-login",
            IconName::Lock,
            "Log in to a provider",
            Some("/login".into()),
            Rc::new(|this, _, cx| this.with_pane(cx, |p, cx| p.run_prompt("/login", cx))),
            cx,
        ));
        rows.push(Self::action_row(
            t,
            "m-act-settings",
            IconName::Settings,
            "Settings",
            None,
            Rc::new(|this, _, cx| this.open_settings(SettingsPage::Root, cx)),
            cx,
        ));
        let chips = (!statuses.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(space::S2))
                .px(px(space::S4))
                .pb(px(space::S2))
                .children(statuses.into_iter().map(|(text, tone)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S1_5))
                        .h(px(size::CAPSULE))
                        .px(px(space::S2))
                        .rounded(px(radius::R_FULL))
                        .bg(t.surface())
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(tone_dot(t, tone))
                        .child(SharedString::from(text))
                }))
        });
        let body = div()
            .flex()
            .flex_col()
            .children(chips)
            .children(rows)
            .into_any_element();
        self.sheet_frame(
            t,
            "m-actions-sheet",
            Some("Actions".into()),
            body,
            None,
            true,
            false,
            cx,
        )
    }

    fn status_sheet(
        &mut self,
        t: &OinoTheme,
        pane: &Entity<SessionPane>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = pane.read(cx).status().clone();
        let frame = Ticker::peek(cx);
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(g) = &s.goal {
            rows.push(Self::section(t, "Goal"));
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S1))
                    .px(px(space::S4))
                    .py(px(space::S2))
                    .child(
                        div()
                            .text_role(t, TextRole::Base)
                            .text_color(t.fg())
                            .child(SharedString::from(g.text.clone())),
                    )
                    .child(
                        div()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(if g.paused {
                                "Paused".to_owned()
                            } else {
                                g.detail.clone().unwrap_or_default()
                            })),
                    )
                    .into_any_element(),
            );
        }
        let (done, total) = s.progress();
        if total > 0 {
            rows.push(Self::section(t, format!("Progress {done}/{total}")));
            for (i, st) in s.steps.iter().enumerate() {
                rows.push(
                    div()
                        .id(ElementId::NamedInteger("m-step".into(), i as u64))
                        .flex()
                        .items_center()
                        .gap(px(space::S3))
                        .min_h(px(touch::TARGET))
                        .px(px(space::S4))
                        .text_role(t, TextRole::Base)
                        .text_color(if st.done { t.fg_subtlest() } else { t.fg() })
                        .when(st.done, gpui_kit::Styled::line_through)
                        .child(icon(
                            if st.done {
                                IconName::CircleCheck
                            } else {
                                IconName::CircleDashed
                            },
                            size::ICON,
                            if st.done { t.success() } else { t.fg_subtle() },
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(SharedString::from(st.text.clone())),
                        )
                        .into_any_element(),
                );
            }
        }
        if !s.run.is_empty() {
            rows.push(Self::section(t, "Run"));
            for r in &s.run {
                let glyph = match r.kind {
                    RunKind::Task => IconName::Terminal,
                    RunKind::Agent => IconName::Bot,
                    RunKind::Sidekick => IconName::Users,
                    RunKind::Dream => IconName::Moon,
                };
                let agent = matches!(r.kind, RunKind::Agent | RunKind::Sidekick);
                let (call, title) = (r.id.clone(), r.label.clone());
                rows.push(
                    Self::list_row(
                        t,
                        ElementId::Name(format!("m-run-{}", r.id).into()),
                        Some(glyph),
                        r.label.clone(),
                        r.elapsed.clone(),
                        Some(spinner(frame, size::ICON, t.working()).into_any_element()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.sheet = None;
                        if agent {
                            // AgentRow → child timeline in the right drawer (design.md §6.2).
                            let tab = SideTab::Agent {
                                call_id: call.clone(),
                                title: title.clone(),
                            };
                            this.with_pane(cx, |p, cx| p.open_side(tab, cx));
                        } else {
                            this.with_pane(cx, SessionPane::open_bottom_tasks);
                            this.toast(
                                crate::components::toast::ToastKind::Info,
                                None,
                                "Task output is in the tool row",
                                cx,
                            );
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                );
            }
        }
        if let Some(b) = &s.branch {
            rows.push(Self::section(t, "Git"));
            rows.push(
                Self::list_row(
                    t,
                    "m-git",
                    Some(IconName::GitBranch),
                    b.clone(),
                    Some(format!("{} changed", s.changed)),
                    None,
                )
                .into_any_element(),
            );
        }
        if rows.is_empty() {
            rows.push(
                crate::components::empty::empty_compact(t, IconName::CircleDot, "Nothing running")
                    .into_any_element(),
            );
        }
        let body = div().flex().flex_col().children(rows).into_any_element();
        self.sheet_frame(
            t,
            "m-status-sheet",
            Some("Status".into()),
            body,
            None,
            true,
            false,
            cx,
        )
    }

    fn rename_sheet(
        &mut self,
        t: &OinoTheme,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = if let Some(i) = &self.rename {
            i.clone()
        } else {
            let title = self
                .sessions
                .get(id)
                .map(|s| s.title.clone())
                .unwrap_or_default();
            let input: Entity<InputState> =
                cx.new(|cx| InputState::new(window, cx).default_value(SharedString::from(title)));
            input.update(cx, |s, cx| s.focus(window, cx));
            let sid = id.to_owned();
            cx.subscribe_in(
                &input,
                window,
                move |this, input, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter { .. } => {
                        let name = input.read(cx).value().trim().to_owned();
                        this.commit_rename(&sid, name, window, cx);
                    }
                    InputEvent::Focus => (crate::mobile::MobilePlatform::get(cx).show_keyboard)(),
                    InputEvent::Blur => (crate::mobile::MobilePlatform::get(cx).hide_keyboard)(),
                    InputEvent::Change => {}
                },
            )
            .detach();
            self.rename = Some(input.clone());
            input
        };
        let sid = id.to_owned();
        let save_input = input.clone();
        let body = div()
            .px(px(space::S4))
            .text_size(px(touch::EDIT_FONT))
            .child(Input::new(&input).large())
            .into_any_element();
        let footer = pill_btn(
            "m-rename-save",
            "Save",
            Some(IconName::Check),
            Variant::Primary,
        )
        .full()
        .on_click(cx.listener(move |this, _, window, cx| {
            let name = save_input.read(cx).value().trim().to_owned();
            this.commit_rename(&sid, name, window, cx);
        }))
        .into_any_element();
        self.sheet_frame(
            t,
            "m-rename",
            Some("Rename session".into()),
            body,
            Some(footer),
            true,
            false,
            cx,
        )
    }

    fn commit_rename(
        &mut self,
        id: &str,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_sheet(cx);
        window.blur(cx);
        if name.is_empty() {
            return;
        }
        self.with_session(id, window, cx, move |p, cx| {
            p.send(oino_client::PiCommand::SetSessionName { name }.into(), cx);
        });
    }

    fn row_menu(&mut self, t: &OinoTheme, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(summary) = self.sessions.get(id).cloned() else {
            return div().into_any_element();
        };
        let pinned = self.sessions.pinned().iter().any(|e| e.summary.id == id);
        let (a, b, c, d, e) = (
            id.to_owned(),
            id.to_owned(),
            id.to_owned(),
            id.to_owned(),
            id.to_owned(),
        );
        let title = summary.title.clone();
        let path = summary.path.clone();
        let rows = vec![
            Self::action_row(
                t,
                "m-menu-open",
                IconName::MessageSquareDashed,
                "Open",
                None,
                Rc::new(move |this, window, cx| this.open_session(&a, window, cx)),
                cx,
            ),
            Self::action_row(
                t,
                "m-menu-rename",
                IconName::Pencil,
                "Rename",
                None,
                Rc::new(move |this, _, cx| this.open_sheet(Sheet::Rename(b.clone()), cx)),
                cx,
            ),
            Self::action_row(
                t,
                "m-menu-fork",
                IconName::GitFork,
                "Fork",
                Some("New session with this history".into()),
                Rc::new(move |this, window, cx| {
                    this.with_session(&c, window, cx, |p, cx| {
                        p.send(oino_client::PiCommand::Clone.into(), cx);
                    });
                }),
                cx,
            ),
            Self::action_row(
                t,
                "m-menu-copy",
                IconName::Copy,
                "Copy id",
                None,
                Rc::new(move |this, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(d.clone()));
                    this.toast(
                        crate::components::toast::ToastKind::Success,
                        None,
                        "Session id copied",
                        cx,
                    );
                }),
                cx,
            ),
            Self::action_row(
                t,
                "m-menu-pin",
                IconName::Pin,
                if pinned { "Unpin" } else { "Pin" },
                None,
                Rc::new(move |this, _, _| this.sessions.toggle_pin(&e)),
                cx,
            ),
            Self::list_row(
                t,
                "m-menu-delete",
                Some(IconName::Trash),
                "Delete",
                None,
                None,
            )
            .text_color(t.danger_surface_fg())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let id = summary.id.clone();
                this.confirm_delete(&id, &title, path.clone(), cx);
            }))
            .into_any_element(),
        ];
        let body = div().flex().flex_col().children(rows).into_any_element();
        let title: SharedString = if summary.title.is_empty() {
            "Untitled".into()
        } else {
            summary.title.into()
        };
        self.sheet_frame(t, "m-row-menu", Some(title), body, None, true, false, cx)
    }

    fn edit_sheet(
        &mut self,
        t: &OinoTheme,
        ns: &str,
        key: &str,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = oino_view::settings_hub::fields(&self.hub.schema, ns)
            .into_iter()
            .find(|f| f.key == key);
        let Some(field) = field else {
            return div().into_any_element();
        };
        let current = self
            .hub
            .values
            .get(ns)
            .and_then(|v| oino_view::settings_hub::value_at(v, key).cloned());
        let input = if let Some(i) = &self.rename {
            i.clone()
        } else {
            let secret = matches!(field.kind, oino_view::settings_hub::FieldKind::Secret);
            let text = if secret {
                String::new()
            } else {
                match &current {
                    Some(oino_ai::JsonValue::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                }
            };
            let input: Entity<InputState> = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(secret)
                    .default_value(SharedString::from(text))
            });
            input.update(cx, |s, cx| s.focus(window, cx));
            cx.subscribe_in(&input, window, |_, _, e: &InputEvent, _, cx| match e {
                InputEvent::Focus => (crate::mobile::MobilePlatform::get(cx).show_keyboard)(),
                InputEvent::Blur => (crate::mobile::MobilePlatform::get(cx).hide_keyboard)(),
                _ => {}
            })
            .detach();
            self.rename = Some(input.clone());
            input
        };
        let (ns_s, key_s) = (ns.to_owned(), key.to_owned());
        let save_input = input.clone();
        let reset = (ns.to_owned(), key.to_owned());
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::S2))
            .px(px(space::S4))
            .text_size(px(touch::EDIT_FONT))
            .when_some(field.description.clone(), |el, d| {
                el.child(
                    div()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(d)),
                )
            })
            .child(Input::new(&input).large())
            .into_any_element();
        let footer = div()
            .flex()
            .flex_col()
            .gap(px(space::S2))
            .child(
                pill_btn(
                    "m-edit-save",
                    "Save",
                    Some(IconName::Check),
                    Variant::Primary,
                )
                .full()
                .on_click(cx.listener(move |this, _, window, cx| {
                    let text = save_input.read(cx).value().to_string();
                    match field.parse(&text) {
                        Ok(value) => {
                            this.write_field(&ns_s, &key_s, value, cx);
                            this.close_sheet(cx);
                            window.blur(cx);
                        }
                        Err(e) => {
                            this.toast(crate::components::toast::ToastKind::Error, None, e, cx);
                        }
                    }
                })),
            )
            .child(
                pill_btn("m-edit-reset", "Reset to default", None, Variant::Ghost)
                    .full()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.write_field(&reset.0, &reset.1, None, cx);
                        this.close_sheet(cx);
                        window.blur(cx);
                    })),
            )
            .into_any_element();
        self.sheet_frame(
            t,
            "m-edit",
            Some(SharedString::from(label.to_owned())),
            body,
            Some(footer),
            true,
            false,
            cx,
        )
    }

    fn display_sheet(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let by_name = self.sessions.sorted_by_name();
        let option = |id: &'static str, label: &'static str, on: bool| {
            Self::list_row(
                t,
                id,
                None,
                label,
                None,
                on.then(|| icon(IconName::Check, size::ICON, t.primary()).into_any_element()),
            )
        };
        let rows = vec![
            Self::section(t, "Sort"),
            option("m-sort-recent", "Most recent", !by_name)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.sessions.sorted_by_name() {
                        this.sessions.toggle_sort();
                    }
                    this.close_sheet(cx);
                }))
                .into_any_element(),
            option("m-sort-name", "Workspace name", by_name)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if !this.sessions.sorted_by_name() {
                        this.sessions.toggle_sort();
                    }
                    this.close_sheet(cx);
                }))
                .into_any_element(),
            Self::section(t, "More"),
            Self::list_row(
                t,
                "m-display-hosts",
                Some(IconName::Monitor),
                "Hosts",
                Some("This phone and paired desktops".into()),
                Some(Self::chevron(t)),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_settings(SettingsPage::Remote, cx);
            }))
            .into_any_element(),
            Self::list_row(
                t,
                "m-display-search",
                Some(IconName::Search),
                "Search",
                None,
                Some(Self::chevron(t)),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.push(Page::Search, cx)))
            .into_any_element(),
        ];
        let body = div().flex().flex_col().children(rows).into_any_element();
        self.sheet_frame(
            t,
            "m-display",
            Some("Display".into()),
            body,
            None,
            true,
            false,
            cx,
        )
    }
}

/// The six-segment thinking control (40 high).
fn thinking_segments(
    t: &OinoTheme,
    current: ThinkingLevel,
    cx: &mut Context<MobileShell>,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .w_full()
        .h(px(touch::SEGMENT))
        .p(px(space::HALF))
        .gap(px(space::HALF))
        .rounded(px(radius::R_FULL))
        .bg(t.surface())
        .children(LEVELS.iter().take(6).map(|l| {
            let level = *l;
            let on = level == current;
            div()
                .id(ElementId::Name(format!("m-think-{level}").into()))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(radius::R_FULL))
                .cursor_pointer()
                .text_role(t, TextRole::Sm)
                .weight_medium()
                .text_color(if on { t.fg() } else { t.fg_subtle() })
                .when(on, |el| {
                    el.bg(t.tab_active()).border_1().border_color(t.border())
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.with_pane(cx, |p, cx| p.set_thinking(level, cx));
                    cx.notify();
                }))
                .child(level_label(level))
        }))
        .into_any_element()
}
