//! D12 Extensions (design.md §5.11): loaded components with kind, version and state.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::components::badge::badge;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::settings::page_title;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::ous::Tone;
use crate::views::shell::Workspace;

impl Workspace {
    /// The extensions page.
    pub(crate) fn render_extensions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let header = div()
            .flex()
            .items_center()
            .gap(px(space::S2))
            .h(px(size::CONTROL_LG))
            .px(px(space::S3))
            .border_b_1()
            .border_color(t.border())
            .text_role(&t, TextRole::Sm)
            .weight_medium()
            .text_color(t.fg_subtle())
            .child(div().w(px(size::SIDEBAR)).child("Name"))
            .child(div().w(px(size::SIDEBAR / 2.0)).child("Version"))
            .child(div().w(px(size::SIDEBAR / 2.0)).child("Kind"))
            .child(div().flex_1().child("State"));
        let rows = self.extensions.iter().enumerate().map(|(i, e)| {
            let (glyph, kind) = if e.kind == "provider" {
                (IconName::Plug, "provider")
            } else if e.id.starts_with("unipi.") {
                (IconName::Blocks, "unipi")
            } else {
                (IconName::Box, "user")
            };
            let ns = e.id.trim_start_matches("unipi.").to_owned();
            div()
                .id(("ext-row", i))
                .flex()
                .items_center()
                .gap(px(space::S2))
                .min_h(px(size::CONTROL_LG + space::S2))
                .px(px(space::S3))
                .border_b_1()
                .border_color(t.border())
                .hover(|s| s.bg(t.hover()))
                .child(
                    div()
                        .w(px(size::SIDEBAR))
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .child(icon(glyph, size::ICON, t.fg_subtle()))
                        .child(
                            div()
                                .mono_text(&t, TextRole::Sm)
                                .truncate()
                                .child(SharedString::from(e.id.clone())),
                        ),
                )
                .child(
                    div()
                        .w(px(size::SIDEBAR / 2.0))
                        .mono_text(&t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(e.version.clone())),
                )
                .child(
                    div()
                        .w(px(size::SIDEBAR / 2.0))
                        .child(badge(&t, Tone::Muted, kind)),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .child(match &e.error {
                            Some(err) => div()
                                .id(("ext-err", i))
                                .tooltip(crate::components::tooltip::tip(
                                    SharedString::from(err.clone()),
                                    None,
                                ))
                                .child(badge(&t, Tone::Error, "Failed"))
                                .into_any_element(),
                            None => badge(&t, Tone::Success, "Loaded").into_any_element(),
                        })
                        .when(e.bundled, |el| {
                            el.child(badge(&t, Tone::Default, "bundled"))
                        }),
                )
                .child(
                    Btn::new(("ext-settings", i))
                        .label("Settings")
                        .size(BtnSize::Sm)
                        .variant(Variant::Ghost)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_settings(Some(&ns), window, cx);
                        })),
                )
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.frame())
            .border_1()
            .border_color(t.border())
            .rounded(px(radius::R_XL))
            .overflow_hidden()
            .child(
                div()
                    .id("ext-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(space::S8))
                    .py(px(space::S8))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .max_w(px(size::DIALOG_LG + space::S12 * 4.0))
                            .mx_auto()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(page_title(&t, "Extensions"))
                                    .child(
                                        div()
                                            .text_role(&t, TextRole::Sm)
                                            .text_color(t.fg_subtle())
                                            .child(format!("{} loaded", self.extensions.len())),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .rounded(px(radius::R_XL))
                                    .border_1()
                                    .border_color(t.border())
                                    .overflow_hidden()
                                    .child(header)
                                    .children(rows),
                            ),
                    ),
            )
            .into_any_element()
    }
}
