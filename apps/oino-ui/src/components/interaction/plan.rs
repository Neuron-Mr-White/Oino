//! `PlanReviewCard` (design.md §4.14): workflow plan approval docked above the composer.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_ai::JsonValue;

use super::{Field, InteractionEvent, InteractionView, digit};
use crate::components::button::{Btn, Variant};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, space};

impl InteractionView {
    fn plan_action(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(action) = self.plan.as_ref().and_then(|p| p.actions.get(i)).cloned() else {
            return;
        };
        let feedback = self.field_text(cx);
        let value = (!feedback.trim().is_empty()).then(|| JsonValue::from(feedback.trim()));
        let response = self.dialog.action(&action.action_id, value);
        self.respond(response, cx);
    }

    pub(super) fn plan_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let n = self.plan.as_ref().map_or(0, |p| p.actions.len());
        match event.keystroke.key.as_str() {
            // Esc = keep planning (ZCode ExitPlanMode semantics): the second action.
            "escape" => {
                match self
                    .plan
                    .as_ref()
                    .and_then(|p| p.actions.iter().position(|a| a.action_id == "keep"))
                {
                    Some(i) => self.plan_action(i, cx),
                    None => self.cancel(cx),
                }
                true
            }
            "enter" => {
                let primary = self
                    .plan
                    .as_ref()
                    .and_then(|p| p.actions.iter().position(|a| a.primary))
                    .unwrap_or(0);
                self.plan_action(primary, cx);
                true
            }
            _ => match digit(event).filter(|i| *i < n) {
                Some(i)
                    if !matches!(self.field, Field::Line(_)) || event.keystroke.modifiers.alt =>
                {
                    self.plan_action(i, cx);
                    true
                }
                _ => false,
            },
        }
    }

    pub(super) fn render_plan(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = window;
        let t = OinoTheme::get(cx);
        let Some(plan) = self.plan.clone() else {
            return div().into_any_element();
        };
        let shell = self.shell(&t, IconName::ListChecks, "Plan ready for review".into(), cx);
        let field = match &self.field {
            Field::Line(s) => Some(s.clone()),
            _ => None,
        };
        let max_h = self.max_height;
        shell
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S3))
                    .p(px(space::S4))
                    .child(
                        div()
                            .id("plan-body")
                            .max_h(max_h)
                            .overflow_y_scroll()
                            .text_role(&t, TextRole::Base)
                            .child(
                                TextView::markdown("plan-md", plan.markdown.clone())
                                    .selectable(true),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .when_some(plan.path.clone(), |el, p| {
                                el.child(
                                    div()
                                        .mono_text(&t, TextRole::Sm)
                                        .text_color(t.fg_subtle())
                                        .truncate()
                                        .child(SharedString::from(p)),
                                )
                            })
                            .child(
                                Btn::new("plan-open")
                                    .label("Open in side pane")
                                    .variant(Variant::Link)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(InteractionEvent::OpenPlan);
                                    })),
                            ),
                    )
                    .when_some(field, |el, s| el.child(Input::new(&s)))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(space::S2))
                            .children(plan.actions.iter().enumerate().map(|(i, a)| {
                                let variant = if a.primary {
                                    Variant::Primary
                                } else if a.danger {
                                    Variant::GhostDanger
                                } else if i == 1 {
                                    Variant::Outline
                                } else {
                                    Variant::Ghost
                                };
                                Btn::new(ElementId::Name(format!("plan-{}", a.action_id).into()))
                                    .label(SharedString::from(a.label.clone()))
                                    .kbd((i + 1).to_string())
                                    .variant(variant)
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.plan_action(i, cx)),
                                    )
                            })),
                    ),
            )
            .into_any_element()
    }
}
