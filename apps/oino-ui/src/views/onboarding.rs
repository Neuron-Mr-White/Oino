//! D13 Onboarding (design.md §5.12): first-run stepper (prepare, connect a model, choose a
//! folder, start). Skippable after step 2.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    px,
};

use crate::components::button::{Btn, Variant};
use crate::components::unicrab::oino_mark;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::views::shell::{Route, Workspace};

impl Workspace {
    /// The onboarding page.
    pub(crate) fn render_onboarding(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let ready = !self.panes.is_empty();
        let models = self
            .active_pane()
            .is_some_and(|p| !p.read(cx).models.is_empty());
        let steps = [
            (
                "Preparing extensions",
                if ready {
                    "Ready".to_owned()
                } else {
                    "Compiling the extension cache (≈ 7 s once)…".to_owned()
                },
                ready,
            ),
            (
                "Connect a model",
                if models {
                    "A provider is configured".to_owned()
                } else {
                    "Sign in or add an API key".to_owned()
                },
                models,
            ),
            (
                "Choose a folder",
                self.settings
                    .recent_cwds
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Pick where sessions start".into()),
                !self.settings.recent_cwds.is_empty(),
            ),
            ("Start", "Open your first session".to_owned(), false),
        ];
        let finish = cx.listener(|this, _, window, cx| {
            this.update_settings(|s| s.onboarded = true, window, cx);
            this.route = Route::Sessions;
            cx.notify();
        });
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .gap(px(space::S6))
            .child(oino_mark(&t, size::CRAB.1, 1.0))
            .child(
                div()
                    .text_role(&t, TextRole::Display)
                    .weight_semibold()
                    .child("Welcome to Oino"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(size::DIALOG_MD))
                    .p(px(space::S5))
                    .gap(px(space::S4))
                    .rounded(px(radius::R_2XL))
                    .bg(t.card())
                    .border_1()
                    .border_color(t.card_border())
                    .children(
                        steps
                            .into_iter()
                            .enumerate()
                            .map(|(i, (title, line, done))| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S3))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(size::CONTROL))
                                            .rounded(px(radius::R_FULL))
                                            .bg(if done {
                                                t.success_surface()
                                            } else {
                                                t.surface()
                                            })
                                            .text_color(if done {
                                                t.success_surface_fg()
                                            } else {
                                                t.fg_subtle()
                                            })
                                            .when(done, |el| {
                                                el.child(icon(
                                                    IconName::Check,
                                                    size::ICON_MD,
                                                    t.success_surface_fg(),
                                                ))
                                            })
                                            .when(!done, |el| {
                                                el.child(SharedString::from((i + 1).to_string()))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .text_role(&t, TextRole::Base)
                                                    .weight_medium()
                                                    .child(title),
                                            )
                                            .child(
                                                div()
                                                    .text_role(&t, TextRole::Sm)
                                                    .text_color(t.fg_subtle())
                                                    .child(SharedString::from(line)),
                                            ),
                                    )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(space::S2))
                            .child(
                                Btn::new("onb-models")
                                    .label("Models & Providers")
                                    .variant(Variant::Outline)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings(Some("models"), window, cx);
                                    })),
                            )
                            .child(
                                Btn::new("onb-start")
                                    .label("Start")
                                    .variant(Variant::Primary)
                                    .on_click(finish),
                            ),
                    ),
            )
            .into_any_element()
    }
}
