//! `Composer` (round5 #2/#3, `ZCode` `ChatPromptEditor` + `V4ComposerModeControls`): **one
//! rounded card** (radius 16, 12 px padding, focus brighten) containing the token-aware
//! editor and the inner footer — mode switcher (left) + context ring, model, thought,
//! stop/send (right). The old context header (project button, memory count) and every strip
//! below the card except the native glance row are gone; context lives inside the input as
//! a ring gauge with a hover panel. The permission chip is removed (approvals live in the
//! conversation; the level stays settable via `/permission`), and the model chip opens the
//! model-picker modal ([`crate::components::model_picker`]). Editor-replace panels (work
//! tray, btw) take the shell's place (D11).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{self, Textarea};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, Focusable as _, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathBuilder, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, deferred, div, point, px,
};
use oino_ai::JsonValue;

use crate::automation::Automatable as _;
use crate::components::badge::kbd;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::{LEVELS, level_label, thinking_bars};
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::ous::{self, Ous};
use crate::design::composer as geom;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::composer::{COMPRESS_BELOW, SubmitKey};
use crate::model::ui_state::{Panel, PanelPlacement};
use crate::views::PickThinking;
use crate::views::session_pane::{PaneEvent, PaneMenu, SessionPane};

fn token_color(t: &OinoTheme, text: &str) -> (Hsla, Hsla, IconName) {
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

/// The 14 px context ring gauge (round5 #3; `ZCode` `context.tsx`): track at 25 % opacity,
/// arc `strokeDashoffset = C·(1−used)`, rotated −90°; tones healthy / normal / urgent.
fn ring14(t: &OinoTheme, ratio: f32) -> AnyElement {
    let ratio = ratio.clamp(0.0, 1.0);
    let fill = if ratio >= 0.95 {
        t.danger()
    } else if ratio >= 0.8 {
        t.warning()
    } else if ratio >= 0.5 {
        t.fg_subtle()
    } else {
        t.success()
    };
    let track = t.border();
    let d = geom::RING;
    canvas(
        |_, _, _| (),
        move |bounds: gpui_kit::Bounds<Pixels>, (), window: &mut Window, _| {
            let r = (d / 2.0) - 1.0;
            let c = bounds.center();
            let paint = |from: f32, to: f32, color: Hsla, window: &mut Window| {
                if to - from <= 0.0001 {
                    return;
                }
                let mut b = PathBuilder::stroke(px(geom::RING_STROKE));
                let steps = 32;
                #[allow(clippy::cast_precision_loss, reason = "≤ 32 steps")]
                for s in 0..=steps {
                    let a = from + (to - from) * (s as f32 / steps as f32);
                    let ang = a * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                    let p = point(c.x + px(r * ang.cos()), c.y + px(r * ang.sin()));
                    if s == 0 {
                        b.move_to(p);
                    } else {
                        b.line_to(p);
                    }
                }
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            paint(0.0, 1.0, track.opacity(0.5), window);
            paint(0.0, ratio, fill, window);
        },
    )
    .size(px(d))
    .flex_shrink_0()
    .into_any_element()
}

/// One source row of the context hover panel: colour square, label, mono % right.
fn source_row(t: &OinoTheme, color: Hsla, label: &str, pct: u8) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(space::S2))
        .child(
            div()
                .size(px(geom::SWATCH))
                .rounded(px(radius::R_CODE / 2.0))
                .bg(color),
        )
        .child(
            div()
                .flex_1()
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .mono_text(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .child(format!("{pct}%")),
        )
        .into_any_element()
}

impl SessionPane {
    /// The composer area: shell (or editor-replace panel) plus completion menu.
    pub(crate) fn render_composer(
        &mut self,
        _welcome: bool,
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
            .text_size(prose)
            // Touch: editable text is 16 (no zoom-on-focus sizes, design.md §6).
            .when(t.touch(), |el| {
                el.text_size(px(crate::design::tokens::touch::EDIT_FONT))
            })
            .text_color(t.fg())
            .capture_key_down(cx.listener(Self::on_composer_key))
            // Ctrl+Shift+M cycles the agent mode while the composer has focus (ZCode's
            // toolbar shortcut, round5 #2); outside the composer the chord keeps its
            // global meaning (thinking menu).
            .capture_action(cx.listener(|this, _: &PickThinking, w, cx| {
                this.cycle_agent_mode(w, cx);
            }))
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
        // One rounded card (round5 #3, `ZCode` `ChatPromptEditor`): radius 16, 12 px padding,
        // the border brightens while the editor has focus.
        let focused = self
            .composer
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let card = div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(space::S2))
            .p(px(space::S3))
            .rounded(px(radius::R_FLOAT))
            .bg(t.input())
            .border_1()
            .border_color(if focused {
                t.input_border_focus()
            } else {
                t.input_border()
            })
            .shadow(t.e_composer())
            .child(editor)
            .child(toolbar);
        // The model picker is a window-level modal, drawn deferred from here (its owner).
        let picker = (self.menu == Some(PaneMenu::Model))
            .then(|| crate::components::model_picker::modal(self, &t, window, cx));
        if self.menu != Some(PaneMenu::Model) {
            self.picker_close();
        }
        div()
            .relative()
            .w_full()
            .children(completion)
            .child(card)
            .when_some(picker, |el, p| el.child(deferred(p)))
            .child({
                let top = Rc::clone(&self.composer_top);
                canvas(
                    move |bounds, _, _| top.set(f32::from(bounds.origin.y)),
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full()
            })
            .into_any_element()
    }

    /// Marks the picker as closed (its open-time reset is keyed off this).
    fn picker_close(&mut self) {
        self.picker.was_open = false;
        self.picker_effort_menu = None;
    }

    /// The context cluster of the inner footer: the ring gauge with its hover panel
    /// (round5 #3; `ZCode` `contextUsage.tsx`, without its business logic — the sources are
    /// the session's own context breakdown).
    fn render_context_ring(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let ratio = self.model.context_percent().map(|p| f32::from(p) / 100.0);
        let percent = self.model.context_percent();
        let usage = self.model.usage;
        let breakdown = self.model.context_breakdown();
        let ctx_window = self.model.model.as_ref().map_or(0, |m| m.context_window);
        let compact = crate::components::meta_row::compact;
        let close = {
            let this = cx.entity().downgrade();
            move |_: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |p, cx| {
                    p.menu = None;
                    cx.notify();
                });
            }
        };
        let Some(ratio) = ratio else {
            return div().into_any_element();
        };
        let theme = *t;
        let compact_entity = cx.entity().downgrade();
        let panel =
            move || {
                let t = &theme;
                let label = if ctx_window > 0 {
                    format!(
                        "{} / {}",
                        compact(usage.context_tokens),
                        compact(ctx_window)
                    )
                } else {
                    compact(usage.context_tokens)
                };
                let total = breakdown.total().max(1);
                #[allow(clippy::cast_precision_loss, reason = "token ratios")]
                let segments: [(Hsla, f32, u8, &str); 4] = [
                    (
                        t.info(),
                        grow(breakdown.input, total),
                        pct(breakdown.input, total),
                        "Input",
                    ),
                    (
                        t.success(),
                        grow(breakdown.cache_read, total),
                        pct(breakdown.cache_read, total),
                        "Cache read",
                    ),
                    (
                        t.working(),
                        grow(breakdown.cache_write, total),
                        pct(breakdown.cache_write, total),
                        "Cache write",
                    ),
                    (
                        t.accent(),
                        grow(breakdown.output, total),
                        pct(breakdown.output, total),
                        "Output",
                    ),
                ];
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S2))
                    .w(px(geom::PANEL))
                    .p(px(space::S3))
                    .rounded(px(geom::PANEL_R))
                    .bg(t.sheet())
                    .border_1()
                    .border_color(t.border())
                    .shadow(t.e_menu())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .child(
                                div()
                                    .flex_1()
                                    .text_role(t, TextRole::Sm)
                                    .weight_semibold()
                                    .child("Context usage"),
                            )
                            .child(
                                div()
                                    .mono_text(t, TextRole::Sm)
                                    .text_color(t.fg_subtle())
                                    .child(SharedString::from(label)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .h(px(geom::BAR))
                            .overflow_hidden()
                            .rounded(px(radius::R_FULL))
                            .bg(t.bubble())
                            .children(segments.iter().map(|(color, portion, _, _)| {
                                div().flex_grow(*portion).bg(*color)
                            })),
                    )
                    .children(
                        segments
                            .iter()
                            .map(|(color, _, share, name)| source_row(t, *color, name, *share))
                            .collect::<Vec<_>>(),
                    )
                    .child(
                        Btn::new("ctx-compact")
                            .label("Compact")
                            .icon(IconName::Minimize2)
                            .size(BtnSize::Sm)
                            .variant(Variant::Secondary)
                            .on_click(move |_, _, cx: &mut App| {
                                let _ = compact_entity.update(cx, |p, cx| {
                                    p.run_prompt("/compact", cx);
                                });
                            }),
                    )
                    .into_any_element()
            };
        popover(
            div()
                .id("context-ring")
                .flex()
                .items_center()
                .justify_center()
                .size(px(size::CONTROL))
                .rounded(px(radius::R_LG))
                .cursor_pointer()
                .hover(|s| s.bg(t.bubble()))
                .tooltip(crate::components::tooltip::tip("Context usage", None))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Context, cx)))
                .child(ring14(t, ratio))
                // Read-back for scenarios: the ring's value is the context percent.
                .automation(
                    crate::automation::Meta::labelled(crate::automation::Role::Chip, "Context")
                        .value(percent.map(|p| format!("{p}%")).unwrap_or_default()),
                ),
            self.menu == Some(PaneMenu::Context),
            Placement::AboveEnd,
            panel,
            close,
        )
        .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "inner footer with its menus")]
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
        // Model chip: opens the model-picker modal (round5 #1) — click or ⌘M.
        let fusion = self
            .model
            .topic("unipi:fusion:status")
            .filter(|f| f.get("leadKey").and_then(JsonValue::as_str).is_some());
        let model_label = match fusion {
            Some(f) => format!(
                "{} ◆ {}",
                f.get("leadName")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("Lead"),
                f.get("sidekickName")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("Sidekick"),
            ),
            None => self.model.model_label(),
        };
        let model_label_text = model_label.clone();
        let level = self.model.thinking;
        let model_el = capsule_pill(t, "model-trigger")
            .bg(t.card())
            .text_color(t.fg())
            .hover(|s| s.bg(t.bubble()))
            .tooltip(crate::components::tooltip::tip(
                "Model — picker",
                Some("⌘M".into()),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Model, cx)))
            .when(!narrow, |el| {
                el.child(
                    div()
                        .max_w(px(size::MENU_MIN * 1.4))
                        .truncate()
                        .child(SharedString::from(model_label)),
                )
            })
            .child(icon(IconName::ChevronDown, size::ICON_SM, t.fg_subtlest()))
            // Read-back for scenarios: the model (or fusion pair) and thinking level.
            .automation(
                crate::automation::Meta::labelled(crate::automation::Role::Chip, model_label_text)
                    .value(level_label(level).to_lowercase()),
            );
        // Thought chip: the thinking-level menu of a reasoning model.
        let reasoning = self.model.model.as_ref().is_some_and(|m| m.reasoning);
        let thought_open = reasoning && self.menu == Some(PaneMenu::Thinking);
        let thought_el = reasoning.then(|| {
            let e = cx.entity().downgrade();
            popover(
                capsule_pill(t, "thought-trigger")
                    .bg(t.card())
                    .text_color(t.fg_subtle())
                    .hover(|s| s.bg(t.bubble()))
                    .tooltip(crate::components::tooltip::tip(
                        "Thinking level",
                        Some("alt+t".into()),
                    ))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Thinking, cx)),
                    )
                    .child(icon(IconName::Lightbulb, size::ICON_MD, t.fg_subtle()))
                    .when(!narrow, |el| el.child(level_label(level).to_lowercase()))
                    .child(icon(IconName::ChevronDown, size::ICON_SM, t.fg_subtlest()))
                    .automation(
                        crate::automation::Meta::labelled(
                            crate::automation::Role::Chip,
                            "Thinking",
                        )
                        .value(level_label(level).to_lowercase()),
                    ),
                thought_open,
                Placement::AboveEnd,
                move || {
                    menu(
                        "thinking-menu",
                        t,
                        size::MENU_MIN,
                        LEVELS
                            .iter()
                            .map(|l| {
                                let e = e.clone();
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
                    this.submit(SubmitKey::Enter, cx);
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
                                    p.submit(SubmitKey::Enter, cx);
                                });
                            })
                            .keys("↵"),
                            MenuEntry::item("Queue after this run", move |_, cx| {
                                let _ = e2.update(cx, |p, cx| {
                                    p.menu = None;
                                    p.submit(SubmitKey::FollowUp, cx);
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
        // Inner footer (round5 #3): mode switcher left; context ring, model, thought,
        // stop/send right — all inside the one card.
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::S1_5))
            .child(attach)
            .child(crate::components::mode_switcher::switcher(
                self, narrow, t, cx,
            ))
            .child(div().flex_1())
            .child(self.render_context_ring(t, cx))
            .child(model_el)
            .children(thought_el)
            .when(narrow && reasoning, |el| el.child(thinking_bars(t, level)))
            .children(choice)
            .child(send)
            .into_any_element()
    }

    /// Completion menu (A2.11): a `sheet` popover, radius 16, 360 wide, rows 40 radius 10,
    /// at most 8 visible; section labels; package tags as colour pills (GA-4). Section
    /// titles are never raw extension ids (round5 #2).
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

/// Percent of `total`, clamped to 100.
fn pct(part: u64, total: u64) -> u8 {
    u8::try_from(part.saturating_mul(100) / total.max(1)).unwrap_or(100)
}

/// Flex-grow portion of `part` over `total` (never collapses a visible segment).
#[allow(
    clippy::cast_precision_loss,
    reason = "token counts as ratios; truncation is irrelevant"
)]
fn grow(part: u64, total: u64) -> f32 {
    #[allow(clippy::cast_precision_loss, reason = "token counts as ratios")]
    let ratio = part as f32 / total.max(1) as f32;
    ratio.max(0.01)
}
