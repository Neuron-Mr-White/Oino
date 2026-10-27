//! `ApprovalCard` (design.md §4.12).

use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, Focusable as _, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use super::{Field, InteractionView, digit};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::ous::{self, Ous};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::dialog::{ApprovalKind, DialogKind};

impl InteractionView {
    fn approve_at(&mut self, i: usize, cx: &mut Context<Self>) {
        let DialogKind::Approval { options, .. } = &self.dialog.kind else {
            return;
        };
        let Some(option) = options.get(i) else {
            return;
        };
        let feedback = self.field_text(cx);
        let response = self.dialog.approve(&option.option_id, Some(&feedback));
        self.respond(response, cx);
    }

    pub(super) fn approval_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let n = match &self.dialog.kind {
            DialogKind::Approval { options, .. } => options.len(),
            _ => 0,
        };
        let in_field = match &self.field {
            Field::Line(s) => s.read(cx).focus_handle(cx).contains_focused(window, cx),
            _ => false,
        };
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.cancel(cx);
            return true;
        }
        if key == "enter" && crate::components::secondary(&event.keystroke.modifiers) {
            // ⌘Enter with feedback = reject with feedback.
            if let DialogKind::Approval { options, .. } = &self.dialog.kind
                && let Some(i) = options.iter().position(|o| !o.kind.is_allow())
            {
                self.approve_at(i, cx);
            }
            return true;
        }
        if in_field {
            return false;
        }
        if let Some(i) = digit(event).filter(|i| *i < n) {
            self.approve_at(i, cx);
            return true;
        }
        match key {
            "enter" => {
                self.approve_at(self.highlighted, cx);
                true
            }
            "left" | "up" if n > 0 => {
                self.highlighted = (self.highlighted + n - 1) % n;
                true
            }
            "right" | "down" | "tab" if n > 0 => {
                self.highlighted = (self.highlighted + 1) % n;
                true
            }
            _ => false,
        }
    }

    pub(super) fn render_approval(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let DialogKind::Approval {
            tool_name,
            summary,
            detail,
            options,
            ..
        } = self.dialog.kind.clone()
        else {
            return div().into_any_element();
        };
        let title = SharedString::from(if self.dialog.title.is_empty() {
            "Allow this action?".to_owned()
        } else {
            self.dialog.title.clone()
        });
        let detail_el = detail.map(|d| {
            let mut o = Ous {
                t,
                on_action: ous::no_action(),
                inputs: None,
                cols: 100,
                roomy: false,
            };
            div()
                .id("approval-detail")
                .max_h(px(size::APPROVAL_DETAIL_MAX))
                .overflow_y_scroll()
                .rounded(px(radius::R_TOOL))
                .bg(t.card())
                .p(px(space::S2))
                .child(ous::render(&d, "approval", &mut o, window, cx))
                .into_any_element()
        });
        let field = match &self.field {
            Field::Line(s) => Some(Self::field_box(&t, Input::new(s).appearance(false))),
            _ => None,
        };
        let highlighted = self.highlighted;
        // Touch: 44 pt buttons, stacked full width when there are more than two (design.md §6.3).
        let touch = t.touch();
        let stacked = touch && options.len() > 2;
        // A2.11: allow-once → primary, always → secondary, reject → danger pill.
        let buttons: Vec<(AnyElement, bool)> = options
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let variant = match o.kind {
                    ApprovalKind::AllowOnce => Variant::Primary,
                    ApprovalKind::AllowAlways => Variant::Secondary,
                    ApprovalKind::RejectOnce | ApprovalKind::RejectAlways => Variant::DangerSoft,
                };
                let b = Btn::new(ElementId::Name(format!("ap-{}", o.option_id).into()))
                    .label(SharedString::from(o.label.clone()))
                    .kbd_lead((i + 1).to_string())
                    .variant(variant)
                    .size(if touch { BtnSize::Touch } else { BtnSize::Md })
                    .when(stacked, Btn::full)
                    .selected(i == highlighted && variant != Variant::Primary && !touch)
                    .on_click(cx.listener(move |this, _, _, cx| this.approve_at(i, cx)))
                    .into_any_element();
                (b, o.kind.is_allow())
            })
            .collect();
        let (allow, reject): (Vec<_>, Vec<_>) = buttons.into_iter().partition(|(_, a)| *a);
        let footer = if stacked {
            div()
                .flex()
                .flex_col()
                .gap(px(space::S2))
                .children(allow.into_iter().map(|(b, _)| b))
                .children(reject.into_iter().map(|(b, _)| b))
        } else {
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap(px(space::S2))
                .children(reject.into_iter().map(|(b, _)| b))
                .children(allow.into_iter().rev().map(|(b, _)| b))
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::S2_5))
            .when(self.dangerous, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .px(px(space::S3))
                        .py(px(space::S2))
                        .rounded(px(radius::R_INPUT))
                        .bg(t.warning_surface())
                        .text_color(t.warning_surface_fg())
                        .text_role(&t, TextRole::Sm)
                        .weight_medium()
                        .child(icon(IconName::TriangleAlert, size::ICON_MD, t.warning()))
                        .child("Destructive command — review before allowing"),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .min_w_0()
                    .when_some(tool_name, |el, tool| {
                        el.child(
                            div()
                                .flex_shrink_0()
                                .mono_text(&t, TextRole::Sm)
                                .text_color(t.working())
                                .child(SharedString::from(tool)),
                        )
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .mono_text(&t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(summary)),
                    ),
            )
            .children(detail_el)
            .children(field)
            .into_any_element();
        self.card(
            &t,
            IconName::ShieldCheck,
            "Approve".into(),
            Some(title),
            body,
            Some(footer.into_any_element()),
            cx,
        )
    }
}
