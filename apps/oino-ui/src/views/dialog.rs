//! Extension dialogs as modals (gpui-app §5.4): select / confirm / input / editor,
//! multiSelect, form (questionnaire), approval (ZCode-style options + feedback) and custom
//! OUS. Escape cancels, Enter submits.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oino_core::ui::UiResponse;

use crate::model::dialog::{ApprovalKind, Dialog, DialogKind};
use crate::views::ous;
use crate::views::theme;

/// Emitted when the user answers.
#[derive(Clone, Debug)]
pub enum DialogEvent {
    /// Send this response.
    Respond(UiResponse),
}

enum Field {
    None,
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

/// The modal for one dialog.
pub struct DialogView {
    dialog: Dialog,
    field: Field,
    highlighted: usize,
    checks: Vec<bool>,
    form_checks: Vec<Vec<bool>>,
    form_other: Vec<Option<Entity<InputState>>>,
    error: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<DialogEvent> for DialogView {}

impl Focusable for DialogView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DialogView {
    /// The modal for `dialog`; focuses its first field.
    pub fn new(dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = match &dialog.kind {
            DialogKind::Input {
                placeholder,
                prefill,
                secret,
            } => {
                let state = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(SharedString::from(placeholder.clone().unwrap_or_default()))
                        .masked(*secret)
                        .default_value(SharedString::from(prefill.clone().unwrap_or_default()))
                });
                Field::Line(state)
            }
            DialogKind::Approval {
                feedback: Some(placeholder),
                ..
            } => Field::Line(cx.new(|cx| {
                InputState::new(window, cx).placeholder(SharedString::from(placeholder.clone()))
            })),
            DialogKind::Editor { prefill, .. } => Field::Area(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(12)
                    .default_value(SharedString::from(prefill.clone().unwrap_or_default()))
            })),
            _ => Field::None,
        };
        let checks = match &dialog.kind {
            DialogKind::MultiSelect { options, .. } => options.iter().map(|o| o.selected).collect(),
            _ => Vec::new(),
        };
        let (form_checks, form_other) = match &dialog.kind {
            DialogKind::Form { questions, .. } => (
                questions
                    .iter()
                    .map(|q| q.options.iter().map(|o| o.selected).collect())
                    .collect(),
                questions
                    .iter()
                    .map(|q| {
                        q.other
                            .then(|| cx.new(|cx| InputState::new(window, cx).placeholder("Other…")))
                    })
                    .collect(),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        let focus = cx.focus_handle();
        match &field {
            Field::Line(state) => state.update(cx, |s, cx| s.focus(window, cx)),
            Field::Area(state) => state.update(cx, |s, cx| s.focus(window, cx)),
            Field::None => window.focus(&focus, cx),
        }
        Self {
            dialog,
            field,
            highlighted: 0,
            checks,
            form_checks,
            form_other,
            error: None,
            focus,
        }
    }

    /// Request id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.dialog.id
    }

    fn field_text(&self, cx: &App) -> String {
        match &self.field {
            Field::Line(s) => s.read(cx).value().to_string(),
            Field::Area(s) => s.read(cx).value().to_string(),
            Field::None => String::new(),
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "emits from this entity; keeps call sites uniform"
    )]
    fn respond(&mut self, response: UiResponse, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Respond(response));
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let response = self.dialog.cancel();
        self.respond(response, cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let response = match &self.dialog.kind {
            DialogKind::Select { options } => match options.get(self.highlighted) {
                Some(o) => self.dialog.text(o.label.clone()),
                None => return,
            },
            DialogKind::Confirm { .. } => self.dialog.confirm(true),
            DialogKind::Input { .. } | DialogKind::Editor { .. } => {
                self.dialog.text(self.field_text(cx))
            }
            DialogKind::MultiSelect { options, .. } => {
                let selected: Vec<String> = options
                    .iter()
                    .zip(&self.checks)
                    .filter(|(_, c)| **c)
                    .map(|(o, _)| o.value.clone())
                    .collect();
                match self.dialog.multi(&selected) {
                    Ok(r) => r,
                    Err(e) => {
                        self.error = Some(e);
                        cx.notify();
                        return;
                    }
                }
            }
            DialogKind::Form { questions, .. } => {
                let answers: Vec<(String, Vec<String>, Option<String>)> = questions
                    .iter()
                    .enumerate()
                    .map(|(qi, q)| {
                        let selected = q
                            .options
                            .iter()
                            .zip(self.form_checks.get(qi).into_iter().flatten())
                            .filter(|(_, c)| **c)
                            .map(|(o, _)| o.value.clone())
                            .collect();
                        let other = self
                            .form_other
                            .get(qi)
                            .and_then(Option::as_ref)
                            .map(|s| s.read(cx).value().to_string());
                        (q.id.clone(), selected, other)
                    })
                    .collect();
                self.dialog.form(&answers)
            }
            DialogKind::Approval { options, .. } => {
                match options
                    .iter()
                    .find(|o| o.kind.is_allow())
                    .or(options.first())
                {
                    Some(o) => {
                        let feedback = self.field_text(cx);
                        self.dialog.approve(&o.option_id, Some(&feedback))
                    }
                    None => return,
                }
            }
            DialogKind::Custom { .. } | DialogKind::Unsupported => return,
        };
        self.respond(response, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let count = match &self.dialog.kind {
            DialogKind::Select { options } => options.len(),
            _ => 0,
        };
        match key {
            "escape" => {
                self.cancel(cx);
                cx.stop_propagation();
            }
            "enter"
                if !matches!(self.field, Field::Area(_))
                    || event.keystroke.modifiers.secondary() =>
            {
                self.submit(cx);
                cx.stop_propagation();
            }
            "down" if count > 0 => {
                self.highlighted = (self.highlighted + 1) % count;
                cx.notify();
                cx.stop_propagation();
            }
            "up" if count > 0 => {
                self.highlighted = (self.highlighted + count - 1) % count;
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    #[allow(
        clippy::unused_self,
        reason = "view helper kept as a method for uniform call sites"
    )]
    fn option_row(
        &self,
        id: SharedString,
        label: String,
        description: Option<String>,
        active: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .id(id)
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .when(active, |el| {
                el.bg(theme.list_active)
                    .border_1()
                    .border_color(theme.list_active_border)
            })
            .hover(|s| s.bg(theme.list_hover))
            .child(div().text_sm().child(SharedString::from(label)))
            .when_some(description, |el, d| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(SharedString::from(d)),
                )
            })
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| on_click(this, window, cx)),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "one arm per dialog kind")]
    fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        match self.dialog.kind.clone() {
            DialogKind::Select { options } => v_flex()
                .gap_1()
                .children(options.into_iter().enumerate().map(|(i, o)| {
                    self.option_row(
                        SharedString::from(format!("opt{i}")),
                        o.label,
                        o.description,
                        i == self.highlighted,
                        cx,
                        move |this, _, cx| {
                            this.highlighted = i;
                            this.submit(cx);
                        },
                    )
                }))
                .into_any_element(),
            DialogKind::Confirm { message } => div()
                .text_sm()
                .child(SharedString::from(message))
                .into_any_element(),
            DialogKind::Input { .. } => match &self.field {
                Field::Line(s) => Input::new(s).into_any_element(),
                _ => div().into_any_element(),
            },
            DialogKind::Editor { .. } => match &self.field {
                Field::Area(s) => v_flex()
                    .gap_1()
                    .child(Textarea::new(s).h(px(260.)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Ctrl+Enter to submit"),
                    )
                    .into_any_element(),
                _ => div().into_any_element(),
            },
            DialogKind::MultiSelect { options, .. } => v_flex()
                .gap_1()
                .children(options.into_iter().enumerate().map(|(i, o)| {
                    let checked = self.checks.get(i).copied().unwrap_or(false);
                    h_flex()
                        .gap_2()
                        .child(
                            Checkbox::new(SharedString::from(format!("chk{i}")))
                                .checked(checked)
                                .label(SharedString::from(o.label))
                                .on_click(cx.listener(move |this, value: &bool, _, cx| {
                                    if let Some(c) = this.checks.get_mut(i) {
                                        *c = *value;
                                    }
                                    cx.notify();
                                })),
                        )
                        .when_some(o.description, |el, d| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(SharedString::from(d)),
                            )
                        })
                }))
                .into_any_element(),
            DialogKind::Form { questions, .. } => v_flex()
                .gap_4()
                .children(questions.into_iter().enumerate().map(|(qi, q)| {
                    let other = self.form_other.get(qi).cloned().flatten();
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .when_some(q.header, |el, h| {
                                    el.child(
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .text_xs()
                                            .bg(cx.theme().secondary)
                                            .child(SharedString::from(h)),
                                    )
                                })
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(SharedString::from(q.question)),
                                ),
                        )
                        .children(q.options.into_iter().enumerate().map(|(oi, o)| {
                            let checked = self
                                .form_checks
                                .get(qi)
                                .and_then(|v| v.get(oi))
                                .copied()
                                .unwrap_or(false);
                            let multi = q.multi;
                            Checkbox::new(SharedString::from(format!("q{qi}o{oi}")))
                                .checked(checked)
                                .label(SharedString::from(match o.description {
                                    Some(d) => format!("{} — {d}", o.label),
                                    None => o.label,
                                }))
                                .on_click(cx.listener(move |this, value: &bool, _, cx| {
                                    if let Some(row) = this.form_checks.get_mut(qi) {
                                        if !multi {
                                            for c in row.iter_mut() {
                                                *c = false;
                                            }
                                        }
                                        if let Some(c) = row.get_mut(oi) {
                                            *c = *value;
                                        }
                                    }
                                    cx.notify();
                                }))
                        }))
                        .when_some(other, |el, s| el.child(Input::new(&s).small()))
                }))
                .into_any_element(),
            DialogKind::Approval {
                tool_name,
                summary,
                detail,
                options,
                ..
            } => {
                let detail_el = detail.map(|d| ous::render(&d, "approval", &ous::no_action(), cx));
                let buttons = options.into_iter().map(|o| {
                    let option_id = o.option_id.clone();
                    let mut b = Button::new(SharedString::from(format!("ap-{}", o.option_id)))
                        .label(SharedString::from(o.label))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let feedback = this.field_text(cx);
                            let response = this.dialog.approve(&option_id, Some(&feedback));
                            this.respond(response, cx);
                        }));
                    b = match o.kind {
                        ApprovalKind::AllowOnce => b.primary(),
                        ApprovalKind::AllowAlways => b.outline(),
                        ApprovalKind::RejectOnce | ApprovalKind::RejectAlways => b.danger(),
                    };
                    b
                });
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .when_some(tool_name, |el, t| {
                                el.child(
                                    div()
                                        .px_1p5()
                                        .rounded_sm()
                                        .text_xs()
                                        .bg(cx.theme().secondary)
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(SharedString::from(t)),
                                )
                            })
                            .child(div().text_sm().child(SharedString::from(summary))),
                    )
                    .children(detail_el)
                    .when_some(
                        match &self.field {
                            Field::Line(s) => Some(s.clone()),
                            _ => None,
                        },
                        |el, s| el.child(Input::new(&s)),
                    )
                    .child(h_flex().gap_2().justify_end().children(buttons))
                    .into_any_element()
            }
            DialogKind::Custom { spec } => {
                let entity = cx.entity().downgrade();
                let dialog = self.dialog.clone();
                let on_action: ous::ActionFn = Rc::new(move |action, value, _, cx| {
                    let response = dialog.action(action, Some(value));
                    let _ = entity.update(cx, |this, cx| this.respond(response, cx));
                });
                ous::render(&spec, "custom", &on_action, cx)
            }
            DialogKind::Unsupported => div()
                .text_sm()
                .text_color(muted)
                .child(format!(
                    "“{}” is not supported in Oino Desktop yet.",
                    self.dialog.method
                ))
                .into_any_element(),
        }
    }

    fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let (primary, secondary) = match &self.dialog.kind {
            DialogKind::Select { .. } | DialogKind::Approval { .. } | DialogKind::Custom { .. } => {
                return h_flex()
                    .justify_end()
                    .child(
                        Button::new("dlg-cancel")
                            .ghost()
                            .small()
                            .label("Cancel (Esc)")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                    .into_any_element();
            }
            DialogKind::Confirm { .. } => ("Yes", "No"),
            DialogKind::Form { .. } => ("Submit", "Cancel"),
            DialogKind::Unsupported => ("", "Close"),
            _ => ("OK", "Cancel"),
        };
        let is_confirm = matches!(self.dialog.kind, DialogKind::Confirm { .. });
        let label = match &self.dialog.kind {
            DialogKind::Form {
                submit_label: Some(l),
                ..
            } => l.clone(),
            _ => primary.to_owned(),
        };
        h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("dlg-secondary")
                    .outline()
                    .label(secondary)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if is_confirm {
                            let r = this.dialog.confirm(false);
                            this.respond(r, cx);
                        } else {
                            this.cancel(cx);
                        }
                    })),
            )
            .when(!primary.is_empty(), |el| {
                el.child(
                    Button::new("dlg-primary")
                        .primary()
                        .label(SharedString::from(label))
                        .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                )
            })
            .into_any_element()
    }
}

impl Render for DialogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(cx);
        let footer = self.footer(cx);
        let theme = cx.theme().clone();
        let width = match &self.dialog.kind {
            DialogKind::Editor { .. } | DialogKind::Custom { .. } | DialogKind::Approval { .. } => {
                px(680.)
            }
            _ => px(520.),
        };
        div()
            .id("dialog-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(
                v_flex()
                    .id("dialog")
                    .w(width)
                    .max_h(px(720.))
                    .overflow_y_scroll()
                    .p_4()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .shadow_lg()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(self.dialog.title.clone())),
                            )
                            .when_some(self.dialog.extension_id.clone(), |el, ext| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(SharedString::from(ext)),
                                )
                            }),
                    )
                    .child(body)
                    .when_some(self.error.clone(), |el, e| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(theme.danger)
                                .child(SharedString::from(e)),
                        )
                    })
                    .child(footer),
            )
    }
}
