//! D12 Extensions (design.md A2.17): `PageHeader` "Extensions" (meta "32 loaded · 1 failed",
//! tools Open folder, Install from file…, crab) and one grouped card per kind (Providers /
//! unipi / User). Rows 64: icon tile, name 15/500 + id mono 12, version, state capsule,
//! enable switch, `⋮` menu (Settings, Readme, Reveal folder).

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use gpui_kit::component::text::TextView;

use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::components::page_header::{PageHeader, header_crab};
use crate::components::toast::ToastKind;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::host::ExtensionInfo;
use crate::rt::Tokio;
use crate::views::shell::AppShell;

impl AppShell {
    /// Readme of `ext` (GA-19): its package README when the folder has one, else the
    /// updater's README panel (`/readme <package>`), which also knows bundled packages. The
    /// file is read on tokio, never in render.
    fn open_readme(&mut self, ext: &ExtensionInfo, cx: &mut Context<Self>) {
        let info = ext.clone();
        let task = Tokio::spawn(cx, async move { info.readme() });
        let package = ext.id.trim_start_matches("unipi.").to_owned();
        let id = ext.id.clone();
        cx.spawn(async move |this, cx| {
            let text = task.await.ok().flatten();
            let _ = this.update(cx, |ws, cx| {
                if let Some(text) = text {
                    ws.ext_readme = Some((id, text));
                } else {
                    // The README panel opens in the visible session.
                    ws.with_pane(cx, |p, cx| p.run_prompt(&format!("/readme {package}"), cx));
                    ws.route = crate::views::shell::Route::Sessions;
                    ws.detail_open = ws.focused.is_some();
                }
                cx.notify();
            });
        })
        .detach();
    }

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
            // On the tokio runtime: the host's future uses `spawn_blocking`, which panics on
            // the GPUI main thread.
            let task = this.update(cx, |_, cx| Tokio::spawn(cx, host.install_extension(path)));
            let Ok(task) = task else {
                return;
            };
            let result = task
                .await
                .unwrap_or_else(|e| Err(crate::host::HostError::Io(e.to_string())));
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

    /// One 64 px extension row.
    fn render_extension_row(
        &self,
        i: usize,
        e: &ExtensionInfo,
        t: &OinoTheme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let glyph = if e.kind == "provider" {
            IconName::Plug
        } else if e.id.starts_with("unipi.") {
            IconName::Blocks
        } else {
            IconName::Box
        };
        let ns = e.id.trim_start_matches("unipi.").to_owned();
        let id = e.id.clone();
        let enabled = e.enabled;
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
                .child(crate::components::chip::status(t, t.danger(), "Failed"))
                .into_any_element(),
            (None, false) => {
                crate::components::chip::status(t, t.fg_subtlest(), "Off").into_any_element()
            }
            (None, true) => {
                crate::components::chip::status(t, t.success(), "Loaded").into_any_element()
            }
        };
        let menu_open = self.ext_menu.as_deref() == Some(e.id.as_str());
        let ext = e.clone();
        let root = e.root.clone();
        let weak = cx.entity().downgrade();
        let more = popover(
            Btn::icon_only(("ext-more", i), IconName::EllipsisVertical, "More")
                .size(BtnSize::Md)
                .on_click(cx.listener({
                    let id = id.clone();
                    move |this, _, _, cx| {
                        this.ext_menu = Some(id.clone());
                        cx.notify();
                    }
                })),
            menu_open,
            Placement::BelowEnd,
            || {
                let (w1, w2, w3) = (weak.clone(), weak.clone(), weak.clone());
                let ns = ns.clone();
                let ext = ext.clone();
                let root = root.clone();
                let no_root = root.is_none();
                menu(
                    "ext-menu",
                    t,
                    size::MENU_MIN,
                    vec![
                        MenuEntry::item("Settings", move |window, cx| {
                            let ns = ns.clone();
                            let _ = w1.update(cx, |ws, cx| {
                                ws.ext_menu = None;
                                ws.open_settings(Some(&ns), window, cx);
                            });
                        })
                        .icon(IconName::Settings),
                        MenuEntry::item("Readme", move |_, cx| {
                            let ext = ext.clone();
                            let _ = w2.update(cx, |ws, cx| {
                                ws.ext_menu = None;
                                ws.open_readme(&ext, cx);
                            });
                        })
                        .icon(IconName::BookOpen),
                        MenuEntry::item("Reveal folder", move |_, cx| {
                            if let Some(r) = &root {
                                cx.reveal_path(r);
                            }
                            let _ = w3.update(cx, |ws, cx| {
                                ws.ext_menu = None;
                                cx.notify();
                            });
                        })
                        .icon(IconName::FolderSearch)
                        .disabled(no_root),
                    ],
                )
                .into_any_element()
            },
            {
                let e = cx.entity().downgrade();
                move |_: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
                    let _ = e.update(cx, |ws, cx| {
                        ws.ext_menu = None;
                        cx.notify();
                    });
                }
            },
        );
        let detail = if e.description.is_empty() {
            e.id.clone()
        } else {
            format!("{} · {}", e.id, e.description)
        };
        div()
            .id(("ext-row", i))
            .flex()
            .items_center()
            .gap(px(space::S3))
            .min_h(px(size::ROW))
            .py(px(space::S2))
            .px(px(space::S3_5))
            .hover(|s| s.bg(t.hover()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .size(px(size::TILE_ROW))
                    .rounded(px(radius::R_ROW))
                    .bg(t.chip())
                    .child(icon(glyph, size::ICON, t.fg_subtle())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(space::HALF))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::S2))
                            .child(
                                div()
                                    .truncate()
                                    .text_role(t, TextRole::Prose)
                                    .line_height(t.text(TextRole::Base).1)
                                    .weight_medium()
                                    .child(SharedString::from(name)),
                            )
                            .when(e.bundled, |el| {
                                el.child(
                                    div()
                                        .text_role(t, TextRole::Xs)
                                        .text_color(t.fg_subtlest())
                                        .child("bundled"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .truncate()
                            .mono_text(t, TextRole::Sm)
                            .text_color(t.fg_subtlest())
                            .child(SharedString::from(detail)),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .mono_text(t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(e.version.clone())),
            )
            .child(state)
            .child(crate::components::controls::switch(
                ("ext-toggle", i),
                t,
                enabled,
                {
                    let weak = cx.entity().downgrade();
                    move |on, _, cx| {
                        let id = id.clone();
                        let _ = weak.update(cx, |this, cx| this.toggle_extension(&id, on, cx));
                    }
                },
            ))
            .child(more)
            .into_any_element()
    }

    /// The extensions page.
    pub(crate) fn render_extensions(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let failed = self.extensions.iter().filter(|e| e.error.is_some()).count();
        let disabled = self.extensions.iter().filter(|e| !e.enabled).count();
        let loaded = self
            .extensions
            .iter()
            .filter(|e| e.error.is_none() && e.enabled)
            .count();
        let mut meta = vec![format!("{loaded} loaded")];
        if failed > 0 {
            meta.push(format!("{failed} failed"));
        }
        if disabled > 0 {
            meta.push(format!("{disabled} off"));
        }
        let folder = self.host.extensions_dir();
        let crab = self.crab(cx);
        let needs = self.sessions.needs_you().len();
        let header = PageHeader::new("Extensions")
            .meta(meta.join(" · "))
            .tool(
                Btn::new("ext-folder")
                    .label("Open folder")
                    .icon(IconName::FolderOpen)
                    .size(BtnSize::Md)
                    .disabled(folder.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(f) = &folder {
                            let _ = std::fs::create_dir_all(f);
                            cx.open_with_system(f);
                        }
                    }),
            )
            .tool(
                Btn::new("ext-install")
                    .label("Install from file…")
                    .icon(IconName::Download)
                    .size(BtnSize::Md)
                    .variant(Variant::Secondary)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.install_from_file(cx);
                    })),
            )
            .tool(header_crab(crab, needs, cx))
            .window_controls(window, &t)
            .render(&t);
        let kinds: [(&str, fn(&ExtensionInfo) -> bool); 3] = [
            ("Providers", |e| e.kind == "provider"),
            ("unipi", |e| {
                e.kind != "provider" && e.id.starts_with("unipi.")
            }),
            ("User", |e| {
                e.kind != "provider" && !e.id.starts_with("unipi.")
            }),
        ];
        let exts = self.extensions.clone();
        let mut sections: Vec<AnyElement> = Vec::new();
        let mut i = 0usize;
        for (title, pick) in kinds {
            let list: Vec<&ExtensionInfo> = exts.iter().filter(|e| pick(e)).collect();
            if list.is_empty() {
                continue;
            }
            let n = list.len();
            let rows: Vec<AnyElement> = list
                .into_iter()
                .map(|e| {
                    let r = self.render_extension_row(i, e, &t, cx);
                    i += 1;
                    r
                })
                .collect();
            sections.push(
                crate::components::section_label::section_row(
                    &t,
                    title,
                    sections.is_empty(),
                    Some(crate::components::section_label::count(&t, n)),
                )
                .into_any_element(),
            );
            sections.push(
                crate::components::card::grouped(&t, rows, size::ROW_DIVIDER_INSET)
                    .into_any_element(),
            );
        }
        if sections.is_empty() {
            sections.push(
                crate::components::empty::empty_state(
                    &t,
                    IconName::Blocks,
                    "No extensions",
                    Some("Install one from a folder with an extension.json.".into()),
                    Vec::new(),
                )
                .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(
                div()
                    .id("ext-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(size::PAGE_GUTTER))
                    .pb(px(space::S8))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .max_w(px(size::SETTINGS_CONTENT + size::SETTINGS_NAV))
                            .children(sections),
                    ),
            )
            .into_any_element()
    }
}
