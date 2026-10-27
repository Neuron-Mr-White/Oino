//! D13 Onboarding (design.md A2.17): a full-window centred column (max 560) with the crab 88,
//! "Welcome to Oino" 30/36 and the steps as a grouped card of numbered rows (prepare,
//! connect a model, choose a folder, start). Pill buttons. Skippable.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
    px,
};

use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::unicrab::unicrab;
use crate::design::brand::CrabState;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::views::shell::{AppShell, Route};

impl AppShell {
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
        let folder = !self.settings.recent_cwds.is_empty();
        let steps = [
            (
                "Preparing extensions",
                if ready {
                    "Ready".to_owned()
                } else {
                    "Compiling the extension cache (about 7 s, once)…".to_owned()
                },
                ready,
            ),
            (
                "Connect a model",
                if models {
                    "A provider is set up".to_owned()
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
                folder,
            ),
            ("Start", "Open your first session".to_owned(), false),
        ];
        let n = steps.len();
        let finish = cx.listener(|this, _, window, cx| {
            this.update_settings(|s| s.onboarded = true, window, cx);
            this.route = Route::Sessions;
            cx.notify();
        });
        let rows: Vec<AnyElement> = steps
            .into_iter()
            .enumerate()
            .map(|(i, (title, line, done))| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S3_5))
                    .min_h(px(size::ROW))
                    .px(px(space::S4))
                    .py(px(space::S2_5))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(size::SETTING_TILE))
                            .rounded(px(radius::R_FULL))
                            .bg(if done { t.success_surface() } else { t.chip() })
                            .text_role(&t, TextRole::Sm)
                            .weight_semibold()
                            .text_color(if done {
                                t.success_surface_fg()
                            } else {
                                t.fg_subtle()
                            })
                            .when(done, |el| {
                                el.child(icon(IconName::Check, size::ICON_MD, t.success()))
                            })
                            .when(!done, |el| {
                                el.child(SharedString::from((i + 1).to_string()))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_role(&t, TextRole::Prose)
                                    .line_height(t.text(TextRole::Base).1)
                                    .weight_medium()
                                    .text_color(t.fg())
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_role(&t, TextRole::Caption)
                                    .text_color(if done { t.success() } else { t.fg_subtlest() })
                                    .child(SharedString::from(line)),
                            ),
                    )
                    .when(i + 1 == n, |el| {
                        el.child(icon(IconName::ChevronRight, size::ICON_MD, t.fg_subtlest()))
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .p(px(size::PAGE_GUTTER))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .max_w(px(size::ONBOARDING))
                    .gap(px(space::S2_5))
                    .child(unicrab(
                        if ready {
                            CrabState::Idle
                        } else {
                            CrabState::Working
                        },
                        size::CRAB_EMPTY,
                        cx,
                    ))
                    .child(
                        div()
                            .pt(px(space::S2))
                            .text_role(&t, TextRole::Display)
                            .weight_semibold()
                            .text_color(t.fg())
                            .child("Welcome to Oino"),
                    )
                    .child(
                        div()
                            .pb(px(space::S4))
                            .text_role(&t, TextRole::Base)
                            .text_color(t.fg_subtle())
                            .child("A few steps and Oino gets to work in your folder."),
                    )
                    .child(
                        crate::components::card::grouped(&t, rows, size::ROW_DIVIDER_INSET)
                            .w_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .justify_end()
                            .gap(px(space::S2))
                            .pt(px(space::S4))
                            .child(
                                Btn::new("onb-folder")
                                    .label("Choose folder…")
                                    .variant(Variant::Secondary)
                                    .size(BtnSize::Xl)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_folder(window, cx);
                                    })),
                            )
                            .child(
                                Btn::new("onb-models")
                                    .label("Models & providers")
                                    .variant(Variant::Secondary)
                                    .size(BtnSize::Xl)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_settings(Some("models"), window, cx);
                                    })),
                            )
                            .child(
                                Btn::new("onb-start")
                                    .label("Start")
                                    .variant(Variant::Primary)
                                    .size(BtnSize::Xl)
                                    .on_click(finish),
                            ),
                    ),
            )
            .into_any_element()
    }
}
