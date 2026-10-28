//! `Composer` (design.md §4.11): `R_2XL` shell with optional context header, the token-aware
//! editor, toolbar (attach, mode chip, model, thinking, context ring, send/stop) and the
//! completion menu. Editor-replace panels (work tray, btw) take the shell's place (D11).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{self, Textarea};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};
use oino_ai::JsonValue;

use crate::automation::Automatable as _;
use crate::components::badge::kbd;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::{LEVELS, context_ring, level_label, thinking_bars};
use crate::components::menu::{MenuEntry, Placement, menu, popover, popover_key};
use crate::components::ous::{self, Ous};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::composer::{COMPRESS_BELOW, ModeChip};
use crate::model::sessions::workspace_label;
use crate::model::ui_state::{Panel, PanelPlacement};
use crate::views::session_pane::{PaneEvent, PaneMenu, SessionPane};

fn mode_icon(mode: ModeChip) -> IconName {
    match mode {
        ModeChip::Ask => IconName::Hand,
        ModeChip::Plan => IconName::ListChecks,
        ModeChip::Auto => IconName::Sparkles,
        ModeChip::Full => IconName::ShieldOff,
    }
}

fn token_color(t: &OinoTheme, text: &str) -> (gpui_kit::Hsla, gpui_kit::Hsla, IconName) {
    if text.starts_with('@') {
        (t.file_node(), t.file_node_fg(), IconName::FileText)
    } else if text.starts_with('$') || text.starts_with("/skill:") {
        (t.skill_node(), t.skill_node_fg(), IconName::Zap)
    } else if text.starts_with('#') {
        (
            t.session_node(),
            t.session_node_fg(),
            IconName::MessageSquareDashed,
        )
    } else if text.starts_with('/') {
        (
            t.command_node(),
            t.command_node_fg(),
            IconName::SquareTerminal,
        )
    } else {
        (t.tag(), t.fg(), IconName::Paperclip)
    }
}

/// Mode tint (A2.2): Ask `fg_subtle`, Plan `gold`, Auto `claw`, Full `danger`.
fn mode_tint(t: &OinoTheme, mode: ModeChip) -> gpui_kit::Hsla {
    match mode {
        ModeChip::Ask => t.fg_subtle(),
        ModeChip::Plan => t.gold(),
        ModeChip::Auto => t.working(),
        ModeChip::Full => t.danger(),
    }
}

/// A 30 px toolbar pill (A2.11 capsule items).
fn capsule_pill(t: &OinoTheme, id: &'static str) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(px(space::S1))
        .h(px(size::MODE_CHIP))
        .pl(px(space::S2))
        .pr(px(space::S2_5))
        .rounded(px(radius::R_FULL))
        .cursor_pointer()
        .text_role(t, TextRole::Sm)
        .weight_medium()
}

impl SessionPane {
    /// The composer area: shell (or editor-replace panel) plus completion menu.
    pub(crate) fn render_composer(
        &mut self,
        welcome: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let panel = self
            .model
            .ui
            .panels()
            .find(|p| p.placement == PanelPlacement::EditorReplace)
            .cloned();
        if let Some(panel) = panel {
            return self.render_editor_panel(&panel, &t, window, cx);
        }
        if self.panel_focus.1.take().is_some() {
            let composer = self.composer.clone();
            composer.update(cx, |s, cx| s.focus(window, cx));
        }
        let busy = self.model.is_busy();
        let narrow = self.width < px(COMPRESS_BELOW);
        let empty = self.composer.read(cx).value().is_empty();
        let blocked = self.model.ui.needs_attention();
        let frame_top: Vec<_> = self.model.ui.frame().top;
        let header = (welcome || !frame_top.is_empty())
            .then(|| self.render_context_header(welcome, &frame_top, &t, cx));
        let completion = (!self.completion.rows.is_empty()).then(|| self.render_completion(&t, cx));
        let (prose, _) = t.text(TextRole::Prose);
        // Automation (`gui-driver.md`): the composer is the session's only text input.
        let draft = self.composer.read(cx);
        let automation =
            crate::automation::Meta::labelled(crate::automation::Role::Input, "Message")
                .value(draft.value().to_string())
                .focus(draft.focus_handle(cx));
        let editor = div()
            .id("composer")
            .w_full()
            .min_h(px(size::CONTROL_LG))
            .pl(px(space::S2_5))
            .text_size(prose)
            // Touch: editable text is 16 (no zoom-on-focus sizes, design.md §6).
            .when(t.touch(), |el| {
                el.text_size(px(crate::design::tokens::touch::EDIT_FONT))
            })
            .text_color(t.fg())
            .capture_key_down(cx.listener(Self::on_composer_key))
            // The editor binds ↑ ↓ Tab Esc as actions, which GPUI dispatches before key-down
            // listeners: intercept them in the capture phase (GA-21, completion keys).
            .capture_action(cx.listener(|this, _: &input::MoveUp, w, cx| {
                this.composer_action("up", w, cx);
            }))
            .capture_action(cx.listener(|this, _: &input::MoveDown, w, cx| {
                this.composer_action("down", w, cx);
            }))
            .capture_action(cx.listener(|this, _: &input::IndentInline, w, cx| {
                this.composer_action("tab", w, cx);
            }))
            .capture_action(cx.listener(|this, _: &input::Escape, w, cx| {
                this.composer_action("escape", w, cx);
            }))
            .child(Textarea::new(&self.composer).appearance(false).token({
                move |ctx, _, cx| {
                    let t = OinoTheme::get(cx);
                    let text = ctx.token().text().to_string();
                    let (bg, fg, glyph) = token_color(&t, &text);
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S1))
                        .h(px(size::TOKEN))
                        .px(px(space::S1_5))
                        .rounded(px(radius::R_CODE))
                        .bg(bg)
                        .text_color(fg)
                        .text_role(&t, TextRole::Sm)
                        .child(icon(glyph, size::ICON_SM, fg))
                        .child(ctx.token().label().clone())
                }
            }));
        let editor = crate::automation::probe(editor, automation);
        let toolbar = self.render_toolbar(busy, empty, blocked, narrow, &t, cx);
        // A2.11 shell: input (surface-2), radius 26, padding 8 8 8 0, inset line at 80 %,
        // composer elevation. The editor has no inner border.
        div()
            .relative()
            .w_full()
            .children(completion)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(space::S1_5))
                    .pt(px(space::S2))
                    .pr(px(space::S2))
                    .pb(px(space::S2))
                    .pl(px(space::S2))
                    .rounded(px(radius::R_COMPOSER))
                    .bg(t.input())
                    .border_1()
                    .border_color(t.border().opacity(0.8))
                    .shadow(t.e_composer())
                    .children(header)
                    .child(editor)
                    .child(toolbar),
            )
            .child({
                let top = Rc::clone(&self.composer_top);
                gpui_kit::canvas(
                    move |bounds, _, _| top.set(f32::from(bounds.origin.y)),
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full()
            })
            .into_any_element()
    }

    fn render_context_header(
        &self,
        welcome: bool,
        top: &[crate::model::ui_state::Segment],
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let project = workspace_label(&self.model.cwd);
        let branch = self.status.branch.clone();
        let open = self.menu == Some(PaneMenu::Project);
        let cwd = self.model.cwd.clone();
        let project_btn = popover(
            Btn::new("ctx-project")
                .label(SharedString::from(project))
                .icon(IconName::Folder)
                .trailing(IconName::ChevronDown)
                .variant(Variant::Ghost)
                .size(BtnSize::Sm)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Project, cx))),
            open,
            Placement::BelowStart,
            || {
                let copy = cwd.clone();
                menu(
                    "project-menu",
                    t,
                    size::MENU_MIN * 1.25,
                    vec![
                        MenuEntry::Label(SharedString::from(cwd.clone()), None),
                        MenuEntry::item("Copy path", move |_, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                copy.clone(),
                            ));
                        })
                        .icon(IconName::Copy),
                    ],
                )
                .into_any_element()
            },
            {
                let this = cx.entity().downgrade();
                move |_, cx| {
                    let _ = this.update(cx, |p, cx| {
                        p.menu = None;
                        cx.notify();
                    });
                }
            },
        );
        // Glance top segments (setFrame top): Sm, dim, 1 px `line` dividers, one line.
        let segments: Vec<AnyElement> = top
            .iter()
            .filter(|s| s.id.as_deref() != Some("branch") || !welcome)
            .enumerate()
            .flat_map(|(i, s)| {
                let divider = (i > 0).then(|| {
                    div()
                        .w(px(size::HAIRLINE))
                        .h(px(space::S3))
                        .bg(t.border())
                        .into_any_element()
                });
                let seg = div()
                    .flex_shrink_0()
                    .text_role(t, TextRole::Sm)
                    .text_color(match s.tone {
                        crate::model::ous::Tone::Default => t.fg_subtlest(),
                        tone => t.tone_fg(tone),
                    })
                    // gui-fix-unipi U5: spans, tones and the wordmark brand.
                    .child(crate::components::meta_row::segment_body(t, s))
                    .into_any_element();
                divider.into_iter().chain(std::iter::once(seg))
            })
            .collect();
        div()
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .gap(px(space::S2))
            .pl(px(space::S2))
            .when(welcome, |el| el.child(project_btn))
            .when_some(branch.filter(|_| welcome), |el, b| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S1))
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(icon(IconName::GitBranch, size::ICON_SM, t.fg_subtlest()))
                        .child(SharedString::from(b)),
                )
            })
            .children(segments)
            .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "toolbar capsule with its menus")]
    fn render_toolbar(
        &self,
        busy: bool,
        empty: bool,
        blocked: bool,
        narrow: bool,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let close = {
            let this = cx.entity().downgrade();
            move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |p, cx| {
                    p.menu = None;
                    cx.notify();
                });
            }
        };
        let entity = cx.entity().downgrade();
        // Attach: 34 circle, surface-3, paperclip (A2.11).
        let attach = popover(
            div()
                .id("attach")
                .flex()
                .items_center()
                .justify_center()
                .size(px(size::ATTACH))
                .rounded(px(radius::R_FULL))
                .bg(t.bubble())
                .cursor_pointer()
                .hover(|s| s.bg(t.border()))
                .tooltip(crate::components::tooltip::tip("Attach", None))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Attach, cx)))
                .child(icon(IconName::Paperclip, size::ICON, t.fg()))
                .automation(crate::automation::Meta::icon_button("Attach")),
            self.menu == Some(PaneMenu::Attach),
            Placement::AboveStart,
            || {
                let e1 = entity.clone();
                let e2 = entity.clone();
                let e3 = entity.clone();
                menu(
                    "attach-menu",
                    t,
                    size::MENU_MIN * 1.25,
                    vec![
                        MenuEntry::item("Files…", move |_, cx| {
                            let _ = e1.update(cx, |p, cx| {
                                p.menu = None;
                                p.pick_files(cx);
                            });
                        })
                        .icon(IconName::FileText),
                        MenuEntry::item("Image from clipboard", move |_, cx| {
                            let _ = e2.update(cx, |p, cx| {
                                p.menu = None;
                                p.paste_image(cx);
                            });
                        })
                        .icon(IconName::Clipboard),
                        MenuEntry::item("Mention a file (@)", move |_, cx| {
                            let _ = e3.update(cx, |p, cx| {
                                p.menu = None;
                                let text = p.composer.read(cx).value().to_string();
                                p.set_composer_text(&format!("{text}@"), cx);
                            });
                        })
                        .icon(IconName::Paperclip),
                    ],
                )
                .into_any_element()
            },
            close.clone(),
        );
        // Mode chip: the workflow permission mode, tinted (A2.2), from the
        // `unipi:workflow:status` topic (GA-25).
        let mode = self.mode();
        let mode_el = mode.map(|m| {
            let entity = cx.entity().downgrade();
            let tint = mode_tint(t, m);
            popover(
                capsule_pill(t, "mode-chip")
                    .bg(tint.opacity(0.13))
                    .border_1()
                    .border_color(tint.opacity(0.45))
                    .text_color(tint)
                    .tooltip(crate::components::tooltip::tip(
                        "Permission mode",
                        Some("alt+m".into()),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Mode, cx)))
                    .child(icon(mode_icon(m), size::ICON_MD, tint))
                    .when(!narrow, |el| el.child(m.label()))
                    .child(icon(IconName::ChevronDown, size::ICON_SM, tint))
                    // Read-back for scenarios: the workflow permission mode the chip shows.
                    .automation(crate::automation::Meta::labelled(
                        crate::automation::Role::Chip,
                        m.label(),
                    )),
                self.menu == Some(PaneMenu::Mode),
                Placement::AboveStart,
                || {
                    menu(
                        "mode-menu",
                        t,
                        size::MENU_MIN * 1.25,
                        ModeChip::ALL
                            .iter()
                            .map(|choice| {
                                let e = entity.clone();
                                let choice = *choice;
                                MenuEntry::item(choice.label(), move |_, cx| {
                                    let _ = e.update(cx, |p, cx| {
                                        p.menu = None;
                                        let current = p.mode();
                                        for c in choice.commands(current) {
                                            p.run_prompt(&c, cx);
                                        }
                                    });
                                })
                                .icon(mode_icon(choice))
                                .checked(choice == m)
                            })
                            .collect(),
                    )
                    .into_any_element()
                },
                close.clone(),
            )
        });
        // Model chip: context ring + % + model name + thinking level (A2.11).
        let model_label = self.model.model_label();
        let model_label_text = self.model.model_label();
        let reasoning = self.model.model.as_ref().is_some_and(|m| m.reasoning);
        let level = self.model.thinking;
        let ratio = self.model.context_percent().map(|p| f32::from(p) / 100.0);
        let percent = self.model.context_percent();
        // The model menu is a drill-down: ←/Esc at a provider level goes back one level
        // first (design.md §4.26); at the top level they close. Clicks outside always close.
        let model_key: Rc<dyn Fn(&str, &mut Window, &mut App)> = {
            let this = cx.entity().downgrade();
            Rc::new(move |_: &str, _, cx| {
                let _ = this.update(cx, |p, cx| {
                    if p.menu == Some(PaneMenu::Model) && p.menu_sub.is_some() {
                        p.open_model_sub(None, cx);
                    } else {
                        p.menu = None;
                        p.menu_sub = None;
                        cx.notify();
                    }
                });
            })
        };
        let model_el = popover_key(
            capsule_pill(t, "model-trigger")
                .bg(t.card())
                .text_color(t.fg())
                .hover(|s| s.bg(t.bubble()))
                .tooltip(crate::components::tooltip::tip(
                    "Model and context",
                    Some("⌘M".into()),
                ))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Model, cx)))
                .when_some(ratio, |el, r| el.child(context_ring(t, r)))
                .when_some(percent.filter(|_| !narrow), |el, p| {
                    el.child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child(format!("{p}%")),
                    )
                })
                .when(!narrow, |el| {
                    el.child(
                        div()
                            .max_w(px(size::MENU_MIN))
                            .truncate()
                            .child(SharedString::from(model_label)),
                    )
                })
                .when(reasoning && !narrow, |el| {
                    el.child(
                        div()
                            .text_color(t.fg_subtlest())
                            .child(level_label(level).to_lowercase()),
                    )
                })
                .child(icon(IconName::ChevronDown, size::ICON_SM, t.fg_subtlest()))
                // Read-back for scenarios: the model and thinking level the composer shows.
                .automation(
                    crate::automation::Meta::labelled(
                        crate::automation::Role::Chip,
                        model_label_text,
                    )
                    .value(level_label(level).to_lowercase()),
                ),
            self.menu == Some(PaneMenu::Model),
            Placement::AboveEnd,
            || crate::components::model_menu::model_menu(self, t, cx),
            close.clone(),
            Some(model_key),
        );
        let thinking_el = (reasoning && self.menu == Some(PaneMenu::Thinking)).then(|| {
            let entity = cx.entity().downgrade();
            popover(
                div(),
                true,
                Placement::AboveEnd,
                || {
                    menu(
                        "thinking-menu",
                        t,
                        size::MENU_MIN,
                        LEVELS
                            .iter()
                            .map(|l| {
                                let e = entity.clone();
                                let l = *l;
                                MenuEntry::item(level_label(l), move |_, cx| {
                                    let _ = e.update(cx, |p, cx| p.set_thinking(l, cx));
                                })
                                .checked(l == level)
                            })
                            .collect(),
                    )
                    .into_any_element()
                },
                close.clone(),
            )
        });
        // Send / stop: a 38 shell circle; stop (square) while running with an empty editor.
        let stop = busy && empty;
        let disabled = (empty || blocked) && !stop;
        let send = div()
            .id("send")
            .flex()
            .items_center()
            .justify_center()
            .size(px(size::SEND))
            .rounded(px(radius::R_FULL))
            .bg(if disabled {
                t.primary().opacity(0.35)
            } else {
                t.primary()
            })
            .when(!disabled, |el| {
                el.cursor_pointer().hover(|s| s.bg(t.primary_hover()))
            })
            .tooltip(crate::components::tooltip::tip(
                if stop {
                    "Stop"
                } else if busy {
                    "Steer (Enter) · Queue (Ctrl+Enter)"
                } else {
                    "Send"
                },
                Some(SharedString::from(if stop { "esc" } else { "↵" })),
            ))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if stop {
                    this.abort(cx);
                } else if !disabled {
                    this.submit(crate::model::composer::SubmitKey::Enter, cx);
                }
            }))
            .child(icon(
                if stop {
                    IconName::Square
                } else {
                    IconName::ArrowUp
                },
                size::ICON_LG,
                t.primary_fg(),
            ))
            .automation(
                crate::automation::Meta::icon_button(if stop { "Stop" } else { "Send" })
                    .disabled(disabled && !stop),
            );
        // ⌄: the Steer / Queue choice while a run is active (also Ctrl+Enter).
        let choice = (busy && !empty).then(|| {
            let e1 = entity.clone();
            let e2 = entity.clone();
            popover(
                div()
                    .id("send-choice")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(space::S5 + space::HALF))
                    .h(px(size::SEND))
                    .rounded(px(radius::R_FULL))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bubble()))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Send, cx)))
                    .child(icon(IconName::ChevronDown, size::ICON_MD, t.fg_subtle())),
                self.menu == Some(PaneMenu::Send),
                Placement::AboveEnd,
                || {
                    menu(
                        "send-menu",
                        t,
                        size::MENU_MIN * 1.25,
                        vec![
                            MenuEntry::item("Steer now", move |_, cx| {
                                let _ = e1.update(cx, |p, cx| {
                                    p.menu = None;
                                    p.submit(crate::model::composer::SubmitKey::Enter, cx);
                                });
                            })
                            .keys("↵"),
                            MenuEntry::item("Queue after this run", move |_, cx| {
                                let _ = e2.update(cx, |p, cx| {
                                    p.menu = None;
                                    p.submit(crate::model::composer::SubmitKey::FollowUp, cx);
                                });
                            })
                            .keys("Ctrl+↵"),
                        ],
                    )
                    .into_any_element()
                },
                close.clone(),
            )
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::S1_5))
            .child(attach)
            .children(mode_el)
            .child(div().flex_1())
            .child(model_el)
            .children(thinking_el)
            .when(narrow && reasoning, |el| el.child(thinking_bars(t, level)))
            .children(choice)
            .child(send)
            .into_any_element()
    }

    /// Completion menu (A2.11): a `sheet` popover, radius 16, 360 wide, rows 40 radius 10,
    /// at most 8 visible; section labels; package tags as colour pills (GA-4).
    fn render_completion(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut last_section: Option<&str> = None;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, row) in self.completion.rows.iter().enumerate() {
            if last_section != Some(row.section.as_str()) {
                last_section = Some(row.section.as_str());
                rows.push(
                    div()
                        .px(px(space::S2_5))
                        .pt(px(if i == 0 { space::S1 } else { space::S2_5 }))
                        .pb(px(space::S1))
                        .child(crate::components::section_label::label(
                            t,
                            SharedString::from(row.section.clone()),
                        ))
                        .into_any_element(),
                );
            }
            let active = i == self.completion.active;
            let tag_color = row.tone.as_deref().and_then(|tone| {
                tone.strip_prefix('c')
                    .and_then(|n| n.parse::<usize>().ok())
                    .map(|n| t.categorical(n.saturating_sub(1)))
                    .or_else(|| {
                        let tone = crate::model::ous::Tone::parse(tone);
                        (tone != crate::model::ous::Tone::Default).then(|| t.tone_fg(tone))
                    })
            });
            rows.push(
                div()
                    .id(ElementId::NamedInteger(
                        "completion".into(),
                        u64::try_from(i).unwrap_or(0),
                    ))
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap(px(space::S2_5))
                    .h(px(size::MENU_ROW))
                    .px(px(space::S2_5))
                    .rounded(px(radius::R_ROW))
                    .cursor_pointer()
                    .when(active, |el| {
                        el.bg(t.selected())
                            .border_1()
                            .border_color(t.selected_border_strong())
                    })
                    .when(!active, |el| el.hover(|s| s.bg(t.bubble())))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.completion.active = i;
                        this.accept_completion(window, cx);
                    }))
                    .child(icon(
                        token_color(t, &row.value).2,
                        size::ICON,
                        t.fg_subtle(),
                    ))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_role(t, TextRole::Base)
                            .child(SharedString::from(row.label.clone())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .children(row.description.clone().map(SharedString::from)),
                    )
                    .when_some(row.tag.clone(), |el, tag| {
                        el.child(match tag_color {
                            Some(c) => crate::components::chip::tag(t, c, tag),
                            None => crate::components::chip::tag(t, t.fg_subtle(), tag),
                        })
                    })
                    // Read-back for scenarios: every completion row with its package tag.
                    .automation_named(
                        crate::automation::Meta::labelled(
                            crate::automation::Role::MenuRow,
                            format!(
                                "{} {} {}",
                                row.label,
                                row.description.clone().unwrap_or_default(),
                                row.tag.clone().unwrap_or_default()
                            ),
                        )
                        .selected(active),
                        format!("completion-{i}"),
                    )
                    .into_any_element(),
            );
        }
        div()
            .absolute()
            .bottom_full()
            .left_0()
            .mb(px(space::S1_5))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(size::COMPLETION))
                    .p(px(space::S1_5))
                    .rounded(px(radius::R_FLOAT))
                    .bg(t.sheet())
                    .border_1()
                    .border_color(t.border())
                    .shadow(t.e_menu())
                    .child(
                        div()
                            .id("completion-menu")
                            .flex()
                            .flex_col()
                            .max_h(px({
                                // Never taller than the room above the composer (A2.11: at
                                // most 8 rows), so the first rows stay on screen.
                                let room = self.composer_top.get() - size::DETAIL_HEAD - space::S12;
                                let full = size::MENU_ROW * 8.0 + space::S8;
                                if room > size::MENU_ROW * 2.0 {
                                    full.min(room)
                                } else {
                                    full
                                }
                            }))
                            .overflow_y_scroll()
                            .children(rows)
                            .automation(crate::automation::Meta::labelled(
                                crate::automation::Role::Card,
                                "completions",
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .px(px(space::S2_5))
                            .pt(px(space::S1_5))
                            .text_role(t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child(kbd(t, "↑↓"))
                            .child("move")
                            .child(kbd(t, "tab"))
                            .child("accept")
                            .child(kbd(t, "esc"))
                            .child("close"),
                    ),
            )
            .into_any_element()
    }

    /// An `editorReplace` panel in the composer shell (work tray, btw side chat).
    fn render_editor_panel(
        &mut self,
        panel: &Panel,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = panel.id.clone();
        let close_id = id.clone();
        let on_action = self.action_fn(&id, cx);
        let keys = panel.keys.clone();
        let body = {
            let mut o = Ous {
                t: *t,
                on_action: Rc::clone(&on_action),
                inputs: Some(&mut self.inputs),
                cols: crate::components::ous_cols(self.width),
                roomy: false,
            };
            ous::render(&panel.spec, &id, &mut o, window, cx)
        };
        let esc_id = close_id.clone();
        // A2.11 / U4: the tray takes the composer shell (radius 26). Panel keys are not
        // buttons; they stay bound while the panel has focus and the OUS footer shows the
        // legend once (GA-22). Esc closes.
        let key_action = Rc::clone(&on_action);
        // The panel takes the composer slot, so it takes the keyboard (keys + Esc) when it
        // opens; the composer gets focus back when it closes.
        if self.panel_focus.1.as_deref() != Some(id.as_str()) {
            self.panel_focus.1 = Some(id.clone());
            window.focus(&self.panel_focus.0, cx);
        }
        div()
            .id("editor-panel")
            .track_focus(&self.panel_focus.0)
            .flex()
            .flex_col()
            .w_full()
            .px(px(space::S3_5))
            .py(px(space::S3))
            .gap(px(space::S2))
            .rounded(px(radius::R_COMPOSER))
            .bg(t.input())
            .border_1()
            .border_color(t.border().opacity(0.8))
            .shadow(t.e_composer())
            .on_key_down(cx.listener(move |this, e: &gpui_kit::KeyDownEvent, _, cx| {
                let key = e.keystroke.key.as_str();
                if key == "escape" {
                    this.close_panel(&esc_id, cx);
                    cx.stop_propagation();
                } else if let Some(k) = keys.iter().find(|k| k.key == key) {
                    key_action(&k.action_id, JsonValue::Null, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Base)
                            .weight_semibold()
                            .child(SharedString::from(panel.title.clone())),
                    )
                    .child(
                        Btn::icon_only("editor-panel-close", IconName::X, "Close (Esc)")
                            .size(BtnSize::Md)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.close_panel(&close_id, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("editor-panel-body")
                    .max_h(px(size::DETAIL_MAX * 1.25))
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    /// Attaches an image from the clipboard as an `[Image #n]` token (sent as image content).
    /// Returns whether the clipboard held an image.
    pub(crate) fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(image) = item.entries().iter().find_map(|e| match e {
            gpui_kit::ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) else {
            return false;
        };
        let n = self.images.len() + 1;
        let id = format!("img-{}-{n}", self.model.id);
        self.images.insert(
            id.clone(),
            oino_ai::ImageContent {
                data: crate::model::composer::base64(&image.bytes),
                mime_type: image.format.mime_type().to_owned(),
            },
        );
        self.image_order.push(id.clone());
        self.pending_tokens
            .push((id, format!("[Image #{n}]"), format!("Image #{n}")));
        cx.notify();
        true
    }

    /// The attachment strip above the composer: one chip per attached image (`Image #n`), so
    /// what the prompt will carry is visible before sending (design.md A2.11, attachments).
    pub(crate) fn render_attachments(
        &self,
        t: &OinoTheme,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.image_order.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S1_5))
                .children(self.image_order.iter().enumerate().map(|(i, id)| {
                    let mime = self
                        .images
                        .get(id)
                        .map(|image| image.mime_type.clone())
                        .unwrap_or_default();
                    crate::automation::probe(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .h(px(size::CONTROL_SM))
                            .px(px(space::S2))
                            .rounded(px(radius::R_FULL))
                            .bg(t.bubble())
                            .border_1()
                            .border_color(t.border())
                            .text_role(t, TextRole::Xs)
                            .text_color(t.fg_subtle())
                            .child(icon(IconName::File, size::ICON_SM, t.fg_subtle()))
                            .child(SharedString::from(format!("Image #{}", i + 1)))
                            .when(!mime.is_empty(), |el| {
                                el.child(
                                    div()
                                        .text_color(t.fg_subtlest())
                                        .child(SharedString::from(mime)),
                                )
                            }),
                        crate::automation::Meta::labelled(
                            crate::automation::Role::Row,
                            format!("Image #{}", i + 1),
                        ),
                    )
                }))
                .into_any_element(),
        )
    }

    /// Attaches dropped paths (design.md A2.11, drag and drop onto the conversation): an image
    /// file becomes an image attachment (sent as image content with the prompt), anything else
    /// an `@path` mention relative to the session cwd. Returns how many were attached.
    pub(crate) fn attach_paths(
        &mut self,
        paths: &[std::path::PathBuf],
        cx: &mut Context<Self>,
    ) -> usize {
        /// Largest image attached (a model payload is base64 of this).
        const MAX_IMAGE: u64 = 16 * 1024 * 1024;
        let cwd = std::path::PathBuf::from(&self.model.cwd);
        let mut attached = 0;
        for path in paths {
            if path.is_dir() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let image = match ext.as_str() {
                "png" => Some("image/png"),
                "jpg" | "jpeg" => Some("image/jpeg"),
                "webp" => Some("image/webp"),
                "gif" => Some("image/gif"),
                _ => None,
            };
            if let Some(mime) = image {
                let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
                if bytes == 0 || bytes > MAX_IMAGE {
                    self.toast(&format!("{} is too large to attach", path.display()), cx);
                    continue;
                }
                match std::fs::read(path) {
                    Ok(data) => {
                        let n = self.images.len() + 1;
                        let id = format!("img-{}-{n}", self.model.id);
                        self.images.insert(
                            id.clone(),
                            oino_ai::ImageContent {
                                data: crate::model::composer::base64(&data),
                                mime_type: mime.to_owned(),
                            },
                        );
                        self.image_order.push(id.clone());
                        self.pending_tokens.push((
                            id,
                            format!("[Image #{n}]"),
                            format!("Image #{n}"),
                        ));
                        attached += 1;
                    }
                    Err(error) => {
                        self.toast(&format!("Cannot read {}: {error}", path.display()), cx);
                    }
                }
                continue;
            }
            let rel = path
                .strip_prefix(&cwd)
                .unwrap_or(path)
                .display()
                .to_string();
            let token = format!("@{rel}");
            self.pending_tokens
                .push((token.clone(), token.clone(), rel));
            attached += 1;
        }
        cx.notify();
        attached
    }

    /// Opens a native file picker and attaches the files as `@` tokens.
    pub(crate) fn pick_files(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        let cwd = std::path::PathBuf::from(&self.model.cwd);
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                let _ = this.update(cx, |pane, cx| {
                    for p in paths {
                        let rel = p.strip_prefix(&cwd).unwrap_or(&p).display().to_string();
                        let text = format!("@{rel}");
                        pane.pending_tokens.push((text.clone(), text.clone(), rel));
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The action callback of panel `id` (sends `extension_ui_action`).
    pub(crate) fn action_fn(&self, id: &str, cx: &mut Context<Self>) -> ous::ActionFn {
        let entity = cx.entity().downgrade();
        let panel = id.to_owned();
        Rc::new(move |action: &str, value: JsonValue, cx: &mut App| {
            let action = action.to_owned();
            let _ = entity.update(cx, |p, cx| p.panel_action(&panel, &action, value, cx));
        })
    }

    /// Toast helper for components.
    pub(crate) fn toast(&self, text: &str, cx: &mut Context<Self>) {
        cx.emit(PaneEvent::Toast(text.to_owned()));
    }
}
