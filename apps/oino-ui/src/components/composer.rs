//! `Composer` (design.md §4.11): `R_2XL` shell with optional context header, the token-aware
//! editor, toolbar (attach, mode chip, model, thinking, context ring, send/stop) and the
//! completion menu. Editor-replace panels (work tray, btw) take the shell's place (D11).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Textarea;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_ai::JsonValue;

use crate::components::badge::kbd;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::{LEVELS, context_ring, level_label, thinking_bars};
use crate::components::menu::{MenuEntry, Placement, menu, popover};
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
        let busy = self.model.is_busy();
        let narrow = self.width < px(COMPRESS_BELOW);
        let empty = self.composer.read(cx).value().is_empty();
        let blocked = self.model.ui.needs_attention();
        let frame_top: Vec<_> = self.model.ui.frame().top;
        let header = (welcome || !frame_top.is_empty())
            .then(|| self.render_context_header(welcome, &frame_top, &t, cx));
        let completion = (!self.completion.rows.is_empty()).then(|| self.render_completion(&t, cx));
        let editor = div()
            .id("composer")
            .w_full()
            .min_h(px(size::CONTROL_LG))
            .text_role(&t, TextRole::Base)
            // Touch: editable text is 16 (no zoom-on-focus sizes, design.md §6).
            .when(t.touch(), |el| {
                el.text_size(px(crate::design::tokens::touch::EDIT_FONT))
            })
            .text_color(t.fg())
            .capture_key_down(cx.listener(Self::on_composer_key))
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
                        .rounded(px(radius::R_MD))
                        .bg(bg)
                        .text_color(fg)
                        .text_role(&t, TextRole::Sm)
                        .child(icon(glyph, size::ICON_SM, fg))
                        .child(ctx.token().label().clone())
                }
            }));
        let toolbar = self.render_toolbar(busy, empty, blocked, narrow, &t, cx);
        div()
            .relative()
            .w_full()
            .children(completion)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap(px(space::S2))
                    .p(px(space::S3))
                    .rounded(px(radius::R_2XL))
                    .bg(t.input())
                    .border_1()
                    .border_color(t.input_border())
                    .hover(|s| s.border_color(t.input_border_hover()))
                    .children(header)
                    .child(editor)
                    .child(toolbar),
            )
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
                .size(BtnSize::Sm)
                .radius(radius::R_MD)
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
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(space::S1))
            .when(welcome, |el| el.child(project_btn))
            .when_some(branch.filter(|_| welcome), |el, b| {
                el.child(
                    Btn::new("ctx-branch")
                        .label(SharedString::from(b))
                        .icon(IconName::GitBranch)
                        .size(BtnSize::Sm)
                        .radius(radius::R_MD),
                )
            })
            .children(
                top.iter()
                    .filter(|s| s.id.as_deref() != Some("branch") || !welcome)
                    .map(|s| {
                        div()
                            .px(px(space::S1))
                            .text_role(t, TextRole::Sm)
                            .text_color(t.tone_fg(s.tone))
                            .child(SharedString::from(s.text.clone()))
                    }),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "toolbar layout with its menus")]
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
        let attach = popover(
            Btn::icon_only("attach", IconName::Plus, "Attach")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Attach, cx))),
            self.menu == Some(PaneMenu::Attach),
            Placement::AboveStart,
            || {
                let e1 = entity.clone();
                let e2 = entity.clone();
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
                        MenuEntry::item("Mention a file (@)", move |_, cx| {
                            let _ = e2.update(cx, |p, cx| {
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
        let mode = self.mode();
        let mode_el = mode.map(|m| {
            let entity = cx.entity().downgrade();
            popover(
                {
                    let b = Btn::new("mode-chip")
                        .icon(mode_icon(m))
                        .size(BtnSize::Default)
                        .tooltip("Permission mode", Some("alt+p"))
                        .on_click(
                            cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Mode, cx)),
                        );
                    if narrow {
                        b
                    } else {
                        b.label(m.label()).trailing(IconName::ChevronDown)
                    }
                },
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
        let model_label = self.model.model_label();
        let reasoning = self.model.model.as_ref().is_some_and(|m| m.reasoning);
        let model_el = popover(
            {
                let b = Btn::new("model-trigger")
                    .icon(IconName::Box)
                    .tooltip("Model", Some("⌘M"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Model, cx)));
                if narrow {
                    b
                } else {
                    b.label(SharedString::from(model_label))
                        .trailing(IconName::ChevronDown)
                }
            },
            self.menu == Some(PaneMenu::Model),
            Placement::AboveEnd,
            || crate::components::model_menu::model_menu(self, t, cx),
            close.clone(),
        );
        let level = self.model.thinking;
        let thinking_el = reasoning.then(|| {
            let entity = cx.entity().downgrade();
            popover(
                Btn::new("thinking-trigger")
                    .tooltip("Thinking", Some("⌘⇧M"))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Thinking, cx)),
                    )
                    .icon(IconName::Brain)
                    .when(!narrow, |b| {
                        b.label(level_label(level)).trailing(IconName::ChevronDown)
                    }),
                self.menu == Some(PaneMenu::Thinking),
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
        let ratio = self.model.context_percent().map(|p| f32::from(p) / 100.0);
        let ring = ratio.map(|r| {
            let window_k = self
                .model
                .model
                .as_ref()
                .map_or(0, |m| m.context_window / 1000);
            let used_k = self.model.usage.context_tokens / 1000;
            div()
                .id("context-ring")
                .cursor_pointer()
                .tooltip(crate::components::tooltip::tip(
                    format!("{used_k}k / {window_k}k tokens · Compact now"),
                    None,
                ))
                .on_click(cx.listener(|this, _, _, cx| this.compact(cx)))
                .child(context_ring(t, r))
        });
        let stop = busy && empty;
        let send = Btn::new("send")
            .icon(if stop {
                IconName::Square
            } else {
                IconName::ArrowUp
            })
            .variant(if (empty || blocked) && !stop {
                Variant::Secondary
            } else {
                Variant::Primary
            })
            // Send waits for the pending card; Stop always stays available (§4.12).
            .disabled(blocked && !stop)
            .tooltip(
                if stop {
                    "Stop"
                } else if busy {
                    "Steer (Enter) · Follow-up (Alt+Enter)"
                } else {
                    "Send"
                },
                Some(if stop { "esc" } else { "↵" }),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if stop {
                    this.abort(cx);
                } else {
                    this.submit(crate::model::composer::SubmitKey::Enter, cx);
                }
            }));
        let send = div()
            .child(send)
            .size(px(size::CONTROL))
            .rounded(px(radius::R_BUTTON));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(space::S1))
            .child(attach)
            .children(mode_el)
            .child(div().flex_1())
            .child(model_el)
            .children(thinking_el)
            .when(narrow && reasoning, |el| el.child(thinking_bars(t, level)))
            .children(ring)
            .child(send)
            .into_any_element()
    }

    fn render_completion(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let mut last_section: Option<&str> = None;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, row) in self.completion.rows.iter().enumerate() {
            if last_section != Some(row.section.as_str()) {
                last_section = Some(row.section.as_str());
                rows.push(
                    div()
                        .px(px(space::S2))
                        .pt(px(space::S1))
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(row.section.clone()))
                        .into_any_element(),
                );
            }
            let active = i == self.completion.active;
            let tag_tone = row.tone.as_deref().map(crate::model::ous::Tone::parse);
            rows.push(
                div()
                    .id(ElementId::NamedInteger(
                        "completion".into(),
                        u64::try_from(i).unwrap_or(0),
                    ))
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .min_h(px(size::CONTROL_LG))
                    .px(px(space::S2))
                    .rounded(px(radius::R_MD))
                    .cursor_pointer()
                    .when(active, |el| el.bg(t.menu_hover()))
                    .hover(|s| s.bg(t.menu_hover()))
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
                    .when_some(row.description.clone(), |el, d| {
                        el.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_role(t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .child(SharedString::from(d)),
                        )
                    })
                    .when(row.description.is_none(), |el| el.child(div().flex_1()))
                    .when_some(row.tag.clone(), |el, tag| {
                        let tone = tag_tone.unwrap_or(crate::model::ous::Tone::Default);
                        el.child(crate::components::badge::badge(t, tone, tag))
                    })
                    .into_any_element(),
            );
        }
        div()
            .absolute()
            .bottom_full()
            .left_0()
            .mb(px(space::S1))
            .child(
                crate::components::menu::shell(t, size::COMPLETION)
                    .id("completion-menu")
                    .max_h(px(size::CONTROL_LG * 9.0))
                    .overflow_y_scroll()
                    .children(rows)
                    .child(
                        div()
                            .flex()
                            .gap(px(space::S2))
                            .px(px(space::S2))
                            .py(px(space::S1))
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
        div()
            .flex()
            .flex_col()
            .w_full()
            .p(px(space::S3))
            .gap(px(space::S2))
            .rounded(px(radius::R_2XL))
            .bg(t.input())
            .border_1()
            .border_color(t.input_border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .child(
                        div()
                            .flex_1()
                            .text_role(t, TextRole::Base)
                            .weight_medium()
                            .child(SharedString::from(panel.title.clone())),
                    )
                    .children(keys.into_iter().map(|k| {
                        let a = Rc::clone(&on_action);
                        let action = k.action_id.clone();
                        Btn::new(ElementId::Name(format!("pk-{}", k.action_id).into()))
                            .label(SharedString::from(k.label.clone()))
                            .kbd(SharedString::from(k.key.clone()))
                            .size(BtnSize::Sm)
                            .on_click(move |_, _, cx| a(&action, JsonValue::Null, cx))
                    }))
                    .child(
                        div()
                            .text_role(t, TextRole::Xs)
                            .text_color(t.fg_subtlest())
                            .child("Esc to return"),
                    )
                    .child(
                        Btn::icon_only("editor-panel-close", IconName::X, "Close")
                            .size(BtnSize::Sm)
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
