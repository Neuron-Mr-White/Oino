//! `PlanReviewCard` (design.md §4.14): workflow plan approval docked above the composer.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use oino_ai::JsonValue;

use super::{Field, InteractionEvent, InteractionView, digit};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, space};

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
        let field = match &self.field {
            Field::Line(s) => Some(Self::field_box(&t, Input::new(s).appearance(false))),
            _ => None,
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::S2_5))
            .child(
                div()
                    .id("plan-body")
                    .rounded(px(radius::R_TOOL))
                    .bg(t.card())
                    .px(px(space::S3))
                    .py(px(space::S2))
                    .text_role(&t, TextRole::Base)
                    .child(TextView::markdown("plan-md", plan.markdown.clone()).selectable(true)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .when_some(plan.path.clone(), |el, p| {
                        el.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .mono_text(&t, TextRole::Sm)
                                .text_color(t.fg_subtlest())
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
            .children(field)
            .into_any_element();
        let footer = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap(px(space::S2))
            .children(plan.actions.iter().enumerate().rev().map(|(i, a)| {
                let variant = if a.primary {
                    Variant::Primary
                } else if a.danger {
                    Variant::DangerSoft
                } else {
                    Variant::Secondary
                };
                Btn::new(ElementId::Name(format!("plan-{}", a.action_id).into()))
                    .label(SharedString::from(a.label.clone()))
                    .kbd_lead((i + 1).to_string())
                    .variant(variant)
                    .size(BtnSize::Md)
                    .on_click(cx.listener(move |this, _, _, cx| this.plan_action(i, cx)))
            }))
            .into_any_element();
        self.card(
            &t,
            IconName::ListChecks,
            "Plan".into(),
            Some("Plan ready for review".into()),
            body,
            Some(footer),
            cx,
        )
    }
}
