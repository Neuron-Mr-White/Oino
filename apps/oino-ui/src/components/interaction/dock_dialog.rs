//! `DockDialog` (design.md §4.15) for select / confirm / input / editor / multiSelect, and the
//! modal body of other `custom` dialogs.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use super::{Field, InteractionView, digit};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::check;
use crate::components::ous::{self, Ous};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::dialog::DialogKind;

impl InteractionView {
    fn option_count(&self) -> usize {
        match &self.dialog.kind {
            DialogKind::Select { options } => options.len(),
            DialogKind::MultiSelect { options, .. } => options.len(),
            _ => 0,
        }
    }

    fn filter_text(&self, cx: &App) -> String {
        self.filter
            .as_ref()
            .map(|f| f.read(cx).value().to_lowercase())
            .unwrap_or_default()
    }

    fn submit_dock(&mut self, cx: &mut Context<Self>) {
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
            _ => return,
        };
        self.respond(response, cx);
    }

    fn pick(&mut self, i: usize, cx: &mut Context<Self>) {
        match &self.dialog.kind {
            DialogKind::Select { .. } => {
                self.highlighted = i;
                self.submit_dock(cx);
            }
            DialogKind::MultiSelect { .. } => {
                if let Some(c) = self.checks.get_mut(i) {
                    *c = !*c;
                }
                self.highlighted = i;
            }
            _ => {}
        }
    }

    pub(super) fn dock_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let n = self.option_count();
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                self.cancel(cx);
                true
            }
            "enter"
                if matches!(self.field, Field::Area(_))
                    && !crate::components::secondary(&event.keystroke.modifiers) =>
            {
                false
            }
            "enter" => {
                self.submit_dock(cx);
                true
            }
            "down" if n > 0 => {
                self.highlighted = (self.highlighted + 1) % n;
                true
            }
            "up" if n > 0 => {
                self.highlighted = (self.highlighted + n - 1) % n;
                true
            }
            "space" if matches!(self.dialog.kind, DialogKind::MultiSelect { .. }) => {
                self.pick(self.highlighted, cx);
                true
            }
            "y" if matches!(self.dialog.kind, DialogKind::Confirm { .. }) => {
                self.submit_dock(cx);
                true
            }
            "n" if matches!(self.dialog.kind, DialogKind::Confirm { .. }) => {
                let r = self.dialog.confirm(false);
                self.respond(r, cx);
                true
            }
            _ => match digit(event).filter(|i| *i < n && self.filter.is_none()) {
                Some(i) if matches!(self.field, Field::None) => {
                    self.pick(i, cx);
                    true
                }
                _ => false,
            },
        }
    }

    #[allow(clippy::too_many_lines, reason = "one arm per dialog kind")]
    pub(super) fn render_dock(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let (glyph, eyebrow) = match &self.dialog.kind {
            DialogKind::Confirm { .. } => (IconName::CircleAlert, "Confirm"),
            DialogKind::Input { .. } | DialogKind::Editor { .. } => {
                (IconName::Pencil, "Your answer")
            }
            DialogKind::MultiSelect { .. } => (IconName::ListChecks, "Pick any"),
            _ => (IconName::ListChecks, "Pick one"),
        };
        let filter = self.filter_text(cx);
        let highlighted = self.highlighted;
        let rows = |options: Vec<(usize, String, Option<String>, Option<bool>)>,
                    this: &mut Self,
                    cx: &mut Context<Self>| {
            let total = options.len();
            let list = div()
                .id("dock-options")
                .flex()
                .flex_col()
                .gap(px(space::S1_5))
                .children(options.into_iter().map(|(i, label, description, mark)| {
                    Self::option_row(
                        &t,
                        ElementId::NamedInteger("dock-opt".into(), u64::try_from(i).unwrap_or(0)),
                        (i < 9 && this.filter.is_none()).then_some(i),
                        &label,
                        description.as_deref(),
                        i == highlighted || mark == Some(true),
                        mark.map(|m| check(&t, m, false).into_any_element()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.pick(i, cx);
                        cx.notify();
                    }))
                    .into_any_element()
                }));
            let _ = total;
            list.into_any_element()
        };
        let body: AnyElement = match self.dialog.kind.clone() {
            DialogKind::Select { options } => {
                let opts = options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| filter.is_empty() || o.label.to_lowercase().contains(&filter))
                    .map(|(i, o)| (i, o.label.clone(), o.description.clone(), None))
                    .collect();
                rows(opts, self, cx)
            }
            DialogKind::MultiSelect { options, .. } => {
                let checks = self.checks.clone();
                let opts = options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| filter.is_empty() || o.label.to_lowercase().contains(&filter))
                    .map(|(i, o)| {
                        (
                            i,
                            o.label.clone(),
                            o.description.clone(),
                            Some(checks.get(i).copied().unwrap_or(false)),
                        )
                    })
                    .collect();
                rows(opts, self, cx)
            }
            DialogKind::Confirm { message } => div()
                .text_role(&t, TextRole::Base)
                .text_color(t.fg_subtle())
                .child(SharedString::from(message))
                .into_any_element(),
            DialogKind::Input { .. } => match &self.field {
                Field::Line(s) => Self::field_box(&t, Input::new(s).appearance(false)),
                _ => div().into_any_element(),
            },
            DialogKind::Editor { language, .. } => match &self.field {
                Field::Area(s) => div()
                    .flex()
                    .flex_col()
                    .gap(px(space::S1))
                    .child(
                        div()
                            .rounded(px(radius::R_OPTION))
                            .bg(t.card())
                            .border_1()
                            .border_color(t.border())
                            .p(px(space::S2))
                            .code_text(&t)
                            .child(Textarea::new(s)),
                    )
                    .child(
                        div()
                            .text_role(&t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(format!(
                                "{} · {} to submit",
                                language.unwrap_or_else(|| "text".into()),
                                crate::components::badge::platform_keys("⌘Enter")
                            )),
                    )
                    .into_any_element(),
                _ => div().into_any_element(),
            },
            _ => div().into_any_element(),
        };
        let confirm = matches!(self.dialog.kind, DialogKind::Confirm { .. });
        let select = matches!(self.dialog.kind, DialogKind::Select { .. });
        let filter_el = self
            .filter
            .clone()
            .map(|f| Self::field_box(&t, Input::new(&f).appearance(false)));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::S2))
            .children(filter_el)
            .child(body)
            .when_some(self.error.clone(), |el, e| {
                el.child(
                    div()
                        .text_role(&t, TextRole::Sm)
                        .text_color(t.danger_surface_fg())
                        .child(SharedString::from(e)),
                )
            })
            .into_any_element();
        let footer = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(space::S2))
            .child(
                Btn::new("dock-cancel")
                    .label(if confirm { "No" } else { "Cancel" })
                    .kbd(if confirm { "n" } else { "esc" })
                    .variant(Variant::Secondary)
                    .size(BtnSize::Md)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if confirm {
                            let r = this.dialog.confirm(false);
                            this.respond(r, cx);
                        } else {
                            this.cancel(cx);
                        }
                    })),
            )
            .when(!select, |el| {
                el.child(
                    Btn::new("dock-primary")
                        .label(if confirm { "Yes" } else { "Submit" })
                        .kbd(if confirm { "y" } else { "↵" })
                        .variant(Variant::Primary)
                        .size(BtnSize::Md)
                        .on_click(cx.listener(|this, _, _, cx| this.submit_dock(cx))),
                )
            })
            .into_any_element();
        let title = SharedString::from(self.dialog.title.clone());
        self.card(
            &t,
            glyph,
            eyebrow.into(),
            (!title.is_empty()).then_some(title),
            body,
            Some(footer),
            cx,
        )
    }

    /// Modal body of a `custom` dialog (OUS) or an unsupported method.
    pub(super) fn render_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let body = match self.dialog.kind.clone() {
            DialogKind::Custom { spec } => {
                let entity = cx.entity().downgrade();
                let dialog = self.dialog.clone();
                let on_action: ous::ActionFn = Rc::new(move |action, value, cx| {
                    let response = dialog.action(action, Some(value));
                    let _ = entity.update(cx, |this, cx| this.respond(response, cx));
                });
                let mut o = Ous {
                    t,
                    on_action,
                    inputs: None,
                    cols: 90,
                    roomy: true,
                };
                ous::render(&spec, "custom", &mut o, window, cx)
            }
            _ => div()
                .text_role(&t, TextRole::Base)
                .text_color(t.fg_subtle())
                .child(format!(
                    "“{}” is not supported in Oino Desktop yet.",
                    self.dialog.method
                ))
                .into_any_element(),
        };
        crate::components::dialog::modal_frame(
            &t,
            "custom-dialog",
            SharedString::from(self.dialog.title.clone()),
            size::DIALOG_MD,
            body,
            vec![
                Btn::new("custom-cancel")
                    .label("Cancel")
                    .variant(Variant::Outline)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                    .into_any_element(),
            ],
            Rc::new({
                let entity = cx.entity().downgrade();
                move |_, cx| {
                    let _ = entity.update(cx, InteractionView::cancel);
                }
            }),
        )
    }
}
