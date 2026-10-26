//! D12 Extensions (design.md §5.11): name, id, version, kind, state (error tooltip) and a
//! permission summary per package; Enable / Disable, Settings, Readme per row; Install from
//! file… and Open extensions folder in the header.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use gpui_kit::component::text::TextView;

use crate::components::badge::{badge, section_label};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::settings::page_title;
use crate::components::toast::ToastKind;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::ous::Tone;
use crate::views::shell::Workspace;

impl Workspace {
    fn toggle_extension(&mut self, id: &str, enable: bool, cx: &mut Context<Self>) {
        match self.host.set_extension_enabled(id, enable) {
            Ok(()) => {
                if let Some(e) = self.extensions.iter_mut().find(|e| e.id == id) {
                    e.enabled = enable;
                }
                self.toast(
                    ToastKind::Info,
                    Some(SharedString::from(format!(
                        "{id} {}",
                        if enable { "enabled" } else { "disabled" }
                    ))),
                    "Applies to new sessions",
                    cx,
                );
            }
            Err(e) => self.toast(ToastKind::Error, None, e.to_string(), cx),
        }
        cx.notify();
    }

    fn install_from_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Install extension".into()),
        });
        let host = std::sync::Arc::clone(&self.host);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut picked))) = paths.await else {
                return;
            };
            let Some(path) = picked.pop() else {
                return;
            };
            let result = host.install_extension(path).await;
            let _ = this.update(cx, |ws, cx| {
                match result {
                    Ok(id) => ws.toast(
                        ToastKind::Info,
                        Some(SharedString::from(format!("Installed {id}"))),
                        "Loads in new sessions",
                        cx,
                    ),
                    Err(e) => ws.toast(
                        ToastKind::Error,
                        Some("Install failed".into()),
                        e.to_string(),
                        cx,
                    ),
                }
                ws.load_extensions(cx);
            });
        })
        .detach();
    }

    /// The README dialog (Readme row action).
    pub(crate) fn render_readme(
        &self,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (title, text) = self.ext_readme.clone()?;
        let e = cx.entity().downgrade();
        Some(crate::components::dialog::modal_frame(
            t,
            "ext-readme",
            SharedString::from(title),
            size::DIALOG_LG,
            div()
                .text_role(t, TextRole::Base)
                .child(TextView::markdown("ext-readme-md", text).selectable(true))
                .into_any_element(),
            Vec::new(),
            std::rc::Rc::new(move |_, cx: &mut gpui_kit::App| {
                let _ = e.update(cx, |ws, cx| {
                    ws.ext_readme = None;
                    cx.notify();
                });
            }),
        ))
    }

    /// The extensions page.
    pub(crate) fn render_extensions(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = OinoTheme::get(cx);
        let failed = self.extensions.iter().filter(|e| e.error.is_some()).count();
        let disabled = self.extensions.iter().filter(|e| !e.enabled).count();
        let loaded = self
            .extensions
            .iter()
            .filter(|e| e.error.is_none() && e.enabled)
            .count();
        let col = |w: f32| div().w(px(w)).flex_shrink_0().flex().items_center();
        let header = div()
            .flex()
            .items_center()
            .gap(px(space::S3))
            .h(px(size::CONTROL_LG))
            .px(px(space::S3))
            .border_b_1()
            .border_color(t.border())
            .child(col(size::SIDEBAR).child(section_label(&t, "Extension")))
            .child(col(size::SIDEBAR / 3.0).child(section_label(&t, "Version")))
            .child(col(size::SIDEBAR / 3.0).child(section_label(&t, "Kind")))
            .child(col(size::SIDEBAR / 2.5).child(section_label(&t, "State")))
            .child(div().flex_1().child(section_label(&t, "Permissions")));
        let rows: Vec<AnyElement> = self
            .extensions
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let (glyph, kind) = if e.kind == "provider" {
                    (IconName::Plug, "provider")
                } else if e.id.starts_with("unipi.") {
                    (IconName::Blocks, "unipi")
                } else {
                    (IconName::Box, "user")
                };
                let ns = e.id.trim_start_matches("unipi.").to_owned();
                let id = e.id.clone();
                let enabled = e.enabled;
                let readme = e.readme().map(|r| (e.id.clone(), r));
                let name = if e.name.is_empty() {
                    e.id.clone()
                } else {
                    e.name.clone()
                };
                let state = match (&e.error, e.enabled) {
                    (Some(err), _) => div()
                        .id(("ext-err", i))
                        .tooltip(crate::components::tooltip::tip(
                            SharedString::from(err.clone()),
                            None,
                        ))
                        .child(badge(&t, Tone::Error, "Failed"))
                        .into_any_element(),
                    (None, false) => badge(&t, Tone::Muted, "Disabled").into_any_element(),
                    (None, true) => badge(&t, Tone::Success, "Loaded").into_any_element(),
                };
                div()
                    .id(("ext-row", i))
                    .flex()
                    .items_center()
                    .gap(px(space::S3))
                    .min_h(px(size::SETTING_ROW))
                    .py(px(space::S2))
                    .px(px(space::S3))
                    .border_b_1()
                    .border_color(t.border())
                    .hover(|s| s.bg(t.hover()))
                    .child(
                        col(size::SIDEBAR)
                            .gap(px(space::S2))
                            .child(icon(glyph, size::ICON, t.fg_subtle()))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_role(&t, TextRole::Base)
                                            .weight_medium()
                                            .child(SharedString::from(name)),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .mono_text(&t, TextRole::Xs)
                                            .text_color(t.fg_subtle())
                                            .child(SharedString::from(e.id.clone())),
                                    ),
                            ),
                    )
                    .child(
                        col(size::SIDEBAR / 3.0)
                            .mono_text(&t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(e.version.clone())),
                    )
                    .child(col(size::SIDEBAR / 3.0).child(badge(&t, Tone::Muted, kind)))
                    .child(
                        col(size::SIDEBAR / 2.5)
                            .gap(px(space::S1))
                            .child(state)
                            .when(e.bundled, |el| {
                                el.child(badge(&t, Tone::Default, "bundled"))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(&t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .child(SharedString::from(if e.permissions.is_empty() {
                                "—".to_owned()
                            } else {
                                e.permissions.join(" · ")
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .flex_shrink_0()
                            .child(
                                Btn::new(("ext-toggle", i))
                                    .label(if enabled { "Disable" } else { "Enable" })
                                    .icon(IconName::Power)
                                    .size(BtnSize::Sm)
                                    .variant(Variant::Outline)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_extension(&id, !enabled, cx);
                                    })),
                            )
                            .child(
                                Btn::icon_only(("ext-settings", i), IconName::Settings, "Settings")
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_settings(Some(&ns), window, cx);
                                    })),
                            )
                            .child(
                                Btn::icon_only(("ext-readme", i), IconName::BookOpen, "Readme")
                                    .size(BtnSize::Sm)
                                    .disabled(readme.is_none())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.ext_readme.clone_from(&readme);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let folder = self.host.extensions_dir();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.frame())
            .border_1()
            .border_color(t.border())
            .rounded(px(radius::R_CARD))
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
                            .max_w(px(size::DIALOG_LG + space::S12 * 6.0))
                            .mx_auto()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::S2))
                                    .child(page_title(&t, "Extensions"))
                                    .child(div().flex_1())
                                    .child(
                                        Btn::new("ext-folder")
                                            .label("Open extensions folder")
                                            .icon(IconName::FolderOpen)
                                            .size(BtnSize::Sm)
                                            .variant(Variant::Ghost)
                                            .disabled(folder.is_none())
                                            .on_click(move |_, _, cx| {
                                                if let Some(f) = &folder {
                                                    let _ = std::fs::create_dir_all(f);
                                                    cx.open_with_system(f);
                                                }
                                            }),
                                    )
                                    .child(
                                        Btn::new("ext-install")
                                            .label("Install from file…")
                                            .icon(IconName::Download)
                                            .size(BtnSize::Sm)
                                            .variant(Variant::Outline)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.install_from_file(cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .pb(px(space::S3))
                                    .text_role(&t, TextRole::Sm)
                                    .text_color(t.fg_subtle())
                                    .child(format!(
                                        "{loaded} loaded · {failed} failed · {disabled} disabled"
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .rounded(px(radius::R_CARD))
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
