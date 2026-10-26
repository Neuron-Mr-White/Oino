//! `QuestionCard` (design.md §4.13): stepped `form` questions with `ZCode` elicitation keys.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, Focusable as _, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, px,
};

use super::{InteractionView, digit};
use crate::components::badge::kbd;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::check;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::dialog::DialogKind;

impl InteractionView {
    fn question_count(&self) -> usize {
        match &self.dialog.kind {
            DialogKind::Form { questions, .. } => questions.len(),
            _ => 0,
        }
    }

    fn toggle_option(&mut self, q: usize, o: usize) {
        let multi = match &self.dialog.kind {
            DialogKind::Form { questions, .. } => questions.get(q).is_some_and(|q| q.multi),
            _ => false,
        };
        if let Some(row) = self.form_checks.get_mut(q) {
            let was = row.get(o).copied().unwrap_or(false);
            if !multi {
                for c in row.iter_mut() {
                    *c = false;
                }
            }
            if let Some(c) = row.get_mut(o) {
                *c = !was;
            }
        }
    }

    fn submit_form(&mut self, cx: &mut Context<Self>) {
        let DialogKind::Form { questions, .. } = &self.dialog.kind else {
            return;
        };
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
        let response = self.dialog.form(&answers);
        self.respond(response, cx);
    }

    fn next_or_submit(&mut self, cx: &mut Context<Self>) {
        if self.step + 1 < self.question_count() {
            self.step += 1;
        } else {
            self.submit_form(cx);
        }
    }

    pub(super) fn question_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let in_other = self
            .form_other
            .get(self.step)
            .and_then(Option::as_ref)
            .is_some_and(|s| s.read(cx).focus_handle(cx).contains_focused(window, cx));
        if key == "enter" && crate::components::secondary(&event.keystroke.modifiers) {
            self.submit_form(cx);
            return true;
        }
        if key == "escape" {
            if self.step == 0 {
                self.cancel(cx);
            } else {
                self.step -= 1;
            }
            return true;
        }
        if in_other {
            // Enter in "Other" moves on but never submits the last question.
            if key == "enter" && self.step + 1 < self.question_count() {
                self.step += 1;
                return true;
            }
            return false;
        }
        if let Some(i) = digit(event) {
            self.toggle_option(self.step, i);
            return true;
        }
        if key == "enter" {
            self.next_or_submit(cx);
            return true;
        }
        false
    }

    #[allow(clippy::too_many_lines, reason = "declarative card layout")]
    pub(super) fn render_question(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let DialogKind::Form {
            questions,
            submit_label,
        } = self.dialog.kind.clone()
        else {
            return div().into_any_element();
        };
        let n = questions.len();
        let step = self.step.min(n.saturating_sub(1));
        let title = SharedString::from(if n == 1 {
            "Oino has a question".to_owned()
        } else {
            format!("Oino has {n} questions")
        });
        let shell = self.shell(&t, IconName::MessageCircleQuestionMark, title, cx);
        let chips = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(space::S1))
            .children(questions.iter().enumerate().map(|(i, q)| {
                let done = self
                    .form_checks
                    .get(i)
                    .is_some_and(|r| r.iter().any(|c| *c));
                let label = q.header.clone().unwrap_or_else(|| format!("Q{}", i + 1));
                div()
                    .id(ElementId::NamedInteger(
                        "q-chip".into(),
                        u64::try_from(i).unwrap_or(0),
                    ))
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .px(px(space::S2))
                    .h(px(size::CONTROL_SM))
                    .rounded(px(radius::R_MD))
                    .cursor_pointer()
                    .text_role(&t, TextRole::Sm)
                    .text_color(if i == step { t.fg() } else { t.fg_subtle() })
                    .when(i == step, |el| el.bg(t.selected()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.step = i;
                        cx.notify();
                    }))
                    .when(done, |el| {
                        el.child(icon(IconName::Check, size::ICON_SM, t.success()))
                    })
                    .child(SharedString::from(label))
            }));
        let Some(q) = questions.get(step).cloned() else {
            return shell.into_any_element();
        };
        let other = self.form_other.get(step).cloned().flatten();
        let options = q.options.iter().enumerate().map(|(oi, o)| {
            let checked = self
                .form_checks
                .get(step)
                .and_then(|r| r.get(oi))
                .copied()
                .unwrap_or(false);
            div()
                .id(ElementId::NamedInteger(
                    "q-opt".into(),
                    u64::try_from(oi).unwrap_or(0),
                ))
                .flex()
                .items_center()
                .gap(px(space::S3))
                .min_h(t.option_row())
                .px(px(space::S3))
                .rounded(px(radius::R_INPUT))
                .cursor_pointer()
                .border_1()
                .border_color(if checked {
                    t.border_hover()
                } else {
                    t.border()
                })
                .when(checked, |el| el.bg(t.selected()))
                .hover(|s| s.bg(t.hover()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_option(step, oi);
                    cx.notify();
                }))
                .when(oi < 9 && !t.touch(), |el| {
                    el.child(kbd(&t, (oi + 1).to_string()))
                })
                .child(check(&t, checked, !q.multi))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .py(px(space::S1_5))
                        .child(
                            div()
                                .text_role(&t, TextRole::Base)
                                .child(SharedString::from(o.label.clone())),
                        )
                        .when_some(o.description.clone(), |el, d| {
                            el.child(
                                div()
                                    .text_role(&t, TextRole::Sm)
                                    .text_color(t.fg_subtle())
                                    .child(SharedString::from(d)),
                            )
                        }),
                )
        });
        let last = step + 1 >= n;
        let label = if last {
            submit_label.unwrap_or_else(|| "Submit".to_owned())
        } else {
            "Next".to_owned()
        };
        shell
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S3))
                    .p(px(space::S4))
                    .when(n > 1, |el| el.child(chips))
                    .child(
                        div()
                            .text_role(&t, TextRole::Base)
                            .weight_medium()
                            .child(SharedString::from(q.question.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::S1_5))
                            .children(options),
                    )
                    .when_some(other, |el, s| el.child(Input::new(&s)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .when(step > 0, |el| {
                                el.child(
                                    Btn::new("q-back")
                                        .label("Back")
                                        .variant(Variant::Ghost)
                                        .size(BtnSize::Default)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.step = this.step.saturating_sub(1);
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                Btn::new("q-skip")
                                    .label("Cancel")
                                    .kbd("esc")
                                    .variant(Variant::Ghost)
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                            )
                            .child(
                                Btn::new("q-next")
                                    .label(SharedString::from(label))
                                    .variant(Variant::Primary)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.next_or_submit(cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
