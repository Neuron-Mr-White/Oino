//! M8 Settings (design.md §6.8): iOS/Android-style stacked list pages with 56-high rows —
//! Appearance, Models & Providers, Sessions, Permissions, Extensions, Extension settings
//! (namespace list → fields, scope toggle on top), Remote (M10), About. The hub reads and
//! writes through the active session's frontend commands (`_oino/get_settings_schema`,
//! `get_extension_settings`, `set_extension_setting`, `auth_providers`, `login`, `logout`).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use oino_ai::JsonValue;
use oino_client::protocol::SettingsScope;
use oino_client::{FrontendApi as _, SessionBackend};
use oino_core::extensions::settings::SettingsLayer;
use oino_core::session::AuthProviderInfo;
use oino_view::settings_hub::{self, FieldKind, HubNamespace};

use crate::components::controls::{segmented, switch};
use crate::components::toast::ToastKind;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::tokens::touch;
use crate::design::{TextRole, radius, size, space};
use crate::host::{ReduceMotion, ThemePref, ThinkingBlocks, ToolDensity};
use crate::mobile::shell::{MobileShell, Page, SettingsPage, Sheet};
use crate::rt::Tokio;

/// Extension settings hub state (loaded when Settings opens).
#[derive(Default)]
pub(crate) struct Hub {
    pub(crate) schema: Vec<JsonValue>,
    pub(crate) namespaces: Vec<HubNamespace>,
    pub(crate) values: HashMap<String, JsonValue>,
    pub(crate) scope_project: bool,
    pub(crate) providers: Vec<AuthProviderInfo>,
}

impl MobileShell {
    fn backend(&self, cx: &Context<Self>) -> Option<Arc<dyn SessionBackend>> {
        self.active_pane()
            .or_else(|| self.panes.first().map(|p| &p.entity))
            .map(|p| Arc::clone(p.read(cx).backend()))
    }

    /// Loads the hub schema and provider list (Settings opened).
    pub(crate) fn load_hub(&self, cx: &mut Context<Self>) {
        let Some(backend) = self.backend(cx) else {
            return;
        };
        let b = Arc::clone(&backend);
        let task = Tokio::spawn(cx, async move {
            (b.settings_schema().await, b.auth_providers().await)
        });
        cx.spawn(async move |this, cx| {
            if let Ok((schema, providers)) = task.await {
                let _ = this.update(cx, |this, cx| {
                    if let Ok(schema) = schema {
                        this.hub.namespaces = settings_hub::namespaces(&schema);
                        this.hub.schema = schema;
                    }
                    if let Ok(p) = providers {
                        this.hub.providers = p;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn load_values(&self, ns: &str, cx: &mut Context<Self>) {
        let Some(backend) = self.backend(cx) else {
            return;
        };
        let layer = if self.hub.scope_project {
            SettingsLayer::Project
        } else {
            SettingsLayer::Global
        };
        let ns = ns.to_owned();
        let key = ns.clone();
        let task = Tokio::spawn(
            cx,
            async move { backend.extension_settings(&ns, layer).await },
        );
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(values)) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.hub.values.insert(key, values);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Writes one hub field and reloads its namespace.
    pub(crate) fn write_field(
        &self,
        ns: &str,
        key: &str,
        value: Option<JsonValue>,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.backend(cx) else {
            return;
        };
        let scope = if self.hub.scope_project {
            SettingsScope::Project
        } else {
            SettingsScope::Global
        };
        let (n, k) = (ns.to_owned(), key.to_owned());
        let task = Tokio::spawn(cx, async move {
            backend.set_extension_setting(&n, &k, value, scope).await
        });
        let ns = ns.to_owned();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(Ok(())) => this.load_values(&ns, cx),
                Ok(Err(e)) => this.toast(ToastKind::Error, None, e.to_string(), cx),
                Err(e) => this.toast(ToastKind::Error, None, e.to_string(), cx),
            });
        })
        .detach();
    }

    fn auth(&mut self, provider: &str, logout: bool, cx: &mut Context<Self>) {
        let Some(backend) = self.backend(cx) else {
            return;
        };
        let p = provider.to_owned();
        let task = Tokio::spawn(cx, async move {
            if logout {
                backend.logout(&p).await
            } else {
                backend.login(&p).await
            }
        });
        let msg = if logout {
            format!("Signing out of {provider}…")
        } else {
            format!("Signing in to {provider}… answer the prompts in the session")
        };
        self.toast(ToastKind::Info, None, msg, cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(Err(e)) = result {
                    this.toast(ToastKind::Error, None, e.to_string(), cx);
                }
                this.load_hub(cx);
            });
        })
        .detach();
    }

    fn nav_row(
        t: &OinoTheme,
        page: SettingsPage,
        glyph: IconName,
        label: &'static str,
        sub: Option<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Self::list_row(
            t,
            ElementId::Name(format!("m-set-{label}").into()),
            Some(glyph),
            label,
            sub,
            Some(Self::chevron(t)),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            if let SettingsPage::Namespace(ns) = &page {
                this.load_values(ns, cx);
            }
            this.stack.push(Page::Settings(page.clone()));
            cx.notify();
        }))
        .into_any_element()
    }

    /// A 56-high setting row: label + description, control on the right.
    fn setting_row(
        t: &OinoTheme,
        label: impl Into<SharedString>,
        description: Option<String>,
        control: AnyElement,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(space::S4))
            .min_h(px(touch::ROW))
            .px(px(space::S4))
            .py(px(space::S2))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_role(t, TextRole::Base)
                            .text_color(t.fg())
                            .child(label.into()),
                    )
                    .when_some(description, |el, d| {
                        el.child(
                            div()
                                .text_role(t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .child(SharedString::from(d)),
                        )
                    }),
            )
            .child(div().flex_shrink_0().child(control))
            .into_any_element()
    }

    /// A grouped card of rows (`R_CARD`, hairlines between rows).
    fn card(t: &OinoTheme, rows: Vec<AnyElement>) -> AnyElement {
        let n = rows.len();
        div()
            .mx(px(space::S4))
            .flex()
            .flex_col()
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .overflow_hidden()
            .children(rows.into_iter().enumerate().map(move |(i, r)| {
                div()
                    .when(i + 1 < n, |el| el.border_b_1().border_color(t.border()))
                    .child(r)
            }))
            .into_any_element()
    }

    fn choice_rows<T: Copy + PartialEq + 'static>(
        t: &OinoTheme,
        id: &'static str,
        options: &[(T, &'static str)],
        current: T,
        on_pick: Rc<dyn Fn(&mut Self, T, &mut Window, &mut Context<Self>)>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        options
            .iter()
            .enumerate()
            .map(|(i, (value, label))| {
                let value = *value;
                let f = Rc::clone(&on_pick);
                Self::list_row(
                    t,
                    ElementId::NamedInteger(id.into(), i as u64),
                    None,
                    *label,
                    None,
                    (value == current)
                        .then(|| icon(IconName::Check, size::ICON, t.primary()).into_any_element()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    f(this, value, window, cx);
                }))
                .into_any_element()
            })
            .collect()
    }

    pub(crate) fn render_settings(
        &mut self,
        page: &SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let title = match page {
            SettingsPage::Root => "Settings".to_owned(),
            SettingsPage::Appearance => "Appearance".to_owned(),
            SettingsPage::Models => "Models & Providers".to_owned(),
            SettingsPage::Sessions => "Sessions".to_owned(),
            SettingsPage::Permissions => "Permissions".to_owned(),
            SettingsPage::Extensions => "Extensions".to_owned(),
            SettingsPage::Hub => "Extension settings".to_owned(),
            SettingsPage::Namespace(ns) => self
                .hub
                .namespaces
                .iter()
                .find(|n| n.namespace == *ns)
                .map_or_else(|| ns.clone(), |n| n.title.clone()),
            SettingsPage::Remote => "Remote".to_owned(),
            SettingsPage::About => "About".to_owned(),
        };
        let bar = Self::app_bar(
            &t,
            Some(Self::back_btn(cx)),
            Self::title(&t, title, None),
            Vec::new(),
        );
        let body: Vec<AnyElement> = match page {
            SettingsPage::Root => self.settings_root(&t, cx),
            SettingsPage::Appearance => self.settings_appearance(&t, cx),
            SettingsPage::Models => self.settings_models(&t, cx),
            SettingsPage::Sessions => self.settings_sessions(&t, cx),
            SettingsPage::Permissions => {
                let ns = self
                    .hub
                    .namespaces
                    .iter()
                    .find(|n| n.namespace.contains("workflow"))
                    .map(|n| n.namespace.clone());
                match ns {
                    Some(ns) => self.settings_namespace(&t, &ns, cx),
                    None => vec![self.needs_session(
                        &t,
                        "Workflow permissions load from a session.",
                        cx,
                    )],
                }
            }
            SettingsPage::Extensions => self.settings_extensions(&t, cx),
            SettingsPage::Hub => self.settings_hub(&t, cx),
            SettingsPage::Namespace(ns) => self.settings_namespace(&t, ns, cx),
            SettingsPage::Remote => self.render_hosts(&t, window, cx),
            SettingsPage::About => self.settings_about(&t, cx),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(
                div()
                    .id("m-settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::S1))
                            .pb(px(space::S8))
                            .children(body),
                    ),
            )
            .into_any_element()
    }

    fn needs_session(
        &self,
        t: &OinoTheme,
        line: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::components::empty::empty_state(
            t,
            IconName::MessageSquareDashed,
            "No session open",
            Some(line.into()),
            vec![
                crate::mobile::shell::pill_btn(
                    "m-set-start",
                    "Start a session",
                    Some(IconName::SquarePen),
                    crate::components::button::Variant::Primary,
                )
                .on_click(cx.listener(|this, _, window, cx| this.new_session(None, window, cx)))
                .into_any_element(),
            ],
        )
        .into_any_element()
    }

    fn settings_root(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = match self.settings.theme {
            ThemePref::System => "System",
            ThemePref::Light => "Light",
            ThemePref::Dark => "Dark",
        };
        let loaded = self.extensions.iter().filter(|e| e.error.is_none()).count();
        let paired = self.hosts.len();
        vec![
            Self::section(t, "App"),
            Self::card(
                t,
                vec![
                    Self::nav_row(
                        t,
                        SettingsPage::Appearance,
                        IconName::Sun,
                        "Appearance",
                        Some(format!("{theme} · {} px", self.settings.ui_font_size)),
                        cx,
                    ),
                    Self::nav_row(
                        t,
                        SettingsPage::Models,
                        IconName::Brain,
                        "Models & Providers",
                        None,
                        cx,
                    ),
                    Self::nav_row(
                        t,
                        SettingsPage::Sessions,
                        IconName::MessageSquareDashed,
                        "Sessions",
                        None,
                        cx,
                    ),
                    Self::nav_row(
                        t,
                        SettingsPage::Permissions,
                        IconName::ShieldCheck,
                        "Permissions",
                        None,
                        cx,
                    ),
                ],
            ),
            Self::section(t, "Extensions"),
            Self::card(
                t,
                vec![
                    Self::nav_row(
                        t,
                        SettingsPage::Extensions,
                        IconName::Blocks,
                        "Extensions",
                        Some(format!("{loaded} loaded")),
                        cx,
                    ),
                    Self::nav_row(
                        t,
                        SettingsPage::Hub,
                        IconName::SlidersHorizontal,
                        "Extension settings",
                        Some(format!("{} namespaces", self.hub.namespaces.len())),
                        cx,
                    ),
                ],
            ),
            Self::section(t, "Device"),
            Self::card(
                t,
                vec![
                    Self::nav_row(
                        t,
                        SettingsPage::Remote,
                        IconName::Monitor,
                        "Remote",
                        Some(format!("This phone · {paired} paired")),
                        cx,
                    ),
                    Self::nav_row(
                        t,
                        SettingsPage::About,
                        IconName::Info,
                        "About",
                        Some(self.host.info().version),
                        cx,
                    ),
                ],
            ),
        ]
    }

    fn settings_appearance(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme_rows = Self::choice_rows(
            t,
            "m-theme",
            &[
                (ThemePref::System, "System"),
                (ThemePref::Light, "Oino Light"),
                (ThemePref::Dark, "Oino Dark"),
            ],
            self.settings.theme,
            Rc::new(MobileShell::set_theme),
            cx,
        );
        let sizes = ["13", "14", "15", "16", "17", "18"];
        let size_ix = usize::from(self.settings.ui_font_size.saturating_sub(13)).min(5);
        let e = cx.entity().downgrade();
        let size_ctl = segmented(
            "m-ui-size",
            t,
            &sizes,
            size_ix,
            Rc::new(move |i, window, cx| {
                let _ = e.update(cx, |this, cx| {
                    let v = 13 + u8::try_from(i).unwrap_or(2);
                    this.update_settings(|s| s.ui_font_size = v, window, cx);
                });
            }),
        )
        .h(px(touch::SEGMENT))
        .into_any_element();
        let reduce = self.settings.reduce_motion == ReduceMotion::On;
        let e = cx.entity().downgrade();
        vec![
            Self::section(t, "Theme"),
            Self::card(t, theme_rows),
            Self::section(t, "Text size"),
            div().px(px(space::S4)).child(size_ctl).into_any_element(),
            Self::section(t, "Motion"),
            Self::card(
                t,
                vec![Self::setting_row(
                    t,
                    "Reduce motion",
                    Some("Static spinners, no crab animation".into()),
                    switch("m-reduce", t, reduce, move |on, window, cx| {
                        let _ = e.update(cx, |this, cx| {
                            this.update_settings(
                                |s| {
                                    s.reduce_motion = if on {
                                        ReduceMotion::On
                                    } else {
                                        ReduceMotion::FollowOs
                                    };
                                },
                                window,
                                cx,
                            );
                        });
                    })
                    .into_any_element(),
                )],
            ),
        ]
    }

    fn settings_sessions(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let thinking = Self::choice_rows(
            t,
            "m-thinking-blocks",
            &[
                (ThinkingBlocks::Collapsed, "Collapsed"),
                (ThinkingBlocks::Expanded, "Expanded"),
                (ThinkingBlocks::Hidden, "Hidden"),
            ],
            self.settings.thinking_blocks,
            Rc::new(|this, v, window, cx| {
                this.update_settings(
                    |s| {
                        s.thinking_blocks = v;
                        s.hide_thinking = v != ThinkingBlocks::Expanded;
                    },
                    window,
                    cx,
                );
            }),
            cx,
        );
        let density = Self::choice_rows(
            t,
            "m-tool-density",
            &[
                (ToolDensity::Simple, "Simple — one line per tool"),
                (ToolDensity::Regular, "Regular — short previews"),
                (ToolDensity::Advanced, "Advanced — details open"),
            ],
            self.settings.tool_density,
            Rc::new(|this, v, window, cx| this.update_settings(|s| s.tool_density = v, window, cx)),
            cx,
        );
        vec![
            Self::section(t, "Thinking blocks"),
            Self::card(t, thinking),
            Self::section(t, "Tool rows"),
            Self::card(t, density),
        ]
    }

    fn settings_models(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.backend(cx).is_none() {
            return vec![self.needs_session(t, "Providers and models load from a session.", cx)];
        }
        let models: Vec<_> = self
            .active_pane()
            .or_else(|| self.panes.first().map(|p| &p.entity))
            .map(|p| p.read(cx).models.clone())
            .unwrap_or_default();
        let providers = self.hub.providers.clone();
        let rows: Vec<AnyElement> = providers
            .iter()
            .map(|p| {
                let n = models.iter().filter(|m| m.provider == p.id).count();
                let (id, configured) = (p.id.clone(), p.configured);
                let status = if configured {
                    format!(
                        "Signed in{} · {n} models",
                        p.stored
                            .as_ref()
                            .map(|s| format!(" ({s})"))
                            .unwrap_or_default()
                    )
                } else if p.oauth {
                    "Not signed in · tap to sign in".to_owned()
                } else {
                    format!(
                        "Set {}",
                        p.env
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "an API key".into())
                    )
                };
                Self::list_row(
                    t,
                    ElementId::Name(format!("m-prov-{id}").into()),
                    Some(if configured {
                        IconName::CircleCheck
                    } else {
                        IconName::Lock
                    }),
                    p.name.clone(),
                    Some(status),
                    Some(
                        div()
                            .text_role(t, TextRole::Sm)
                            .weight_medium()
                            .text_color(if configured {
                                t.fg_subtle()
                            } else {
                                t.primary()
                            })
                            .child(if configured { "Sign out" } else { "Sign in" })
                            .into_any_element(),
                    ),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if configured {
                        let pid = id.clone();
                        this.open_sheet(
                            Sheet::Confirm(crate::mobile::shell::Confirm {
                                title: SharedString::from(format!("Sign out of {pid}?")),
                                body: "Stored credentials for this provider are removed.".into(),
                                action: "Sign out".into(),
                                run: Rc::new(move |this, _, cx| this.auth(&pid, true, cx)),
                            }),
                            cx,
                        );
                    } else {
                        this.auth(&id, false, cx);
                    }
                }))
                .into_any_element()
            })
            .collect();
        vec![
            Self::section(t, "Providers"),
            if rows.is_empty() {
                crate::components::empty::empty_compact(t, IconName::Cloud, "Loading providers…")
            } else {
                Self::card(t, rows)
            },
            Self::section(t, "Models"),
            Self::card(
                t,
                vec![Self::setting_row(
                    t,
                    "Available models",
                    Some(format!(
                        "{} with credentials · pick one from the conversation title",
                        models.len()
                    )),
                    div().into_any_element(),
                )],
            ),
        ]
    }

    fn settings_extensions(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let rows: Vec<AnyElement> = self
            .extensions
            .clone()
            .into_iter()
            .map(|e| {
                let detail = e.error.clone().unwrap_or_else(|| {
                    format!(
                        "{} · {}{}",
                        e.version,
                        e.kind,
                        if e.bundled { " · bundled" } else { "" }
                    )
                });
                let id = e.id.clone();
                let enabled = e.enabled && e.error.is_none();
                let ctl = switch(ElementId::Name(format!("m-ext-{id}").into()), t, enabled, {
                    let ent = cx.entity().downgrade();
                    move |on, _, cx| {
                        let id = id.clone();
                        let _ = ent.update(cx, |this, cx| {
                            match this.host.set_extension_enabled(&id, on) {
                                Ok(()) => {
                                    if let Some(x) = this.extensions.iter_mut().find(|x| x.id == id)
                                    {
                                        x.enabled = on;
                                    }
                                    this.toast(
                                        ToastKind::Info,
                                        None,
                                        "Applies to new sessions",
                                        cx,
                                    );
                                }
                                Err(err) => this.toast(ToastKind::Error, None, err.to_string(), cx),
                            }
                            cx.notify();
                        });
                    }
                })
                .into_any_element();
                Self::setting_row(
                    t,
                    if e.name.is_empty() {
                        e.id.clone()
                    } else {
                        e.name.clone()
                    },
                    Some(detail),
                    ctl,
                )
            })
            .collect();
        vec![
            Self::section(t, format!("{} installed", rows.len())),
            if rows.is_empty() {
                crate::components::empty::empty_compact(t, IconName::Blocks, "Loading extensions…")
            } else {
                Self::card(t, rows)
            },
        ]
    }

    fn settings_hub(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.backend(cx).is_none() {
            return vec![self.needs_session(t, "Extension settings load from a session.", cx)];
        }
        let rows: Vec<AnyElement> = self
            .hub
            .namespaces
            .clone()
            .into_iter()
            .map(|n| {
                let page = SettingsPage::Namespace(n.namespace.clone());
                let label = n.title.clone();
                Self::list_row(
                    t,
                    ElementId::Name(format!("m-ns-{}", n.namespace).into()),
                    Some(IconName::SlidersHorizontal),
                    label,
                    Some(n.extension_id.clone()),
                    Some(Self::chevron(t)),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let SettingsPage::Namespace(ns) = &page {
                        this.load_values(ns, cx);
                    }
                    this.stack.push(Page::Settings(page.clone()));
                    cx.notify();
                }))
                .into_any_element()
            })
            .collect();
        vec![
            Self::section(t, "Namespaces"),
            if rows.is_empty() {
                crate::components::empty::empty_compact(t, IconName::SlidersHorizontal, "Loading…")
            } else {
                Self::card(t, rows)
            },
        ]
    }

    fn settings_namespace(
        &mut self,
        t: &OinoTheme,
        ns: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if !self.hub.values.contains_key(ns) {
            self.load_values(ns, cx);
        }
        let fields = settings_hub::fields(&self.hub.schema, ns);
        let values = self.hub.values.get(ns).cloned().unwrap_or(JsonValue::Null);
        let e = cx.entity().downgrade();
        let ns_s = ns.to_owned();
        let scope = segmented(
            "m-hub-scope",
            t,
            &["Global", "Project"],
            usize::from(self.hub.scope_project),
            Rc::new(move |i, _, cx| {
                let ns = ns_s.clone();
                let _ = e.update(cx, |this, cx| {
                    this.hub.scope_project = i == 1;
                    this.load_values(&ns, cx);
                });
            }),
        )
        .h(px(touch::SEGMENT))
        .into_any_element();
        let mut out = vec![
            div()
                .px(px(space::S4))
                .pt(px(space::S3))
                .child(scope)
                .into_any_element(),
        ];
        let mut section = String::new();
        let mut rows: Vec<AnyElement> = Vec::new();
        for field in fields {
            if field.section != section && !rows.is_empty() {
                out.push(Self::section(t, section.clone()));
                out.push(Self::card(t, std::mem::take(&mut rows)));
            }
            section.clone_from(&field.section);
            let value = settings_hub::value_at(&values, &field.key).cloned();
            let shown = field.display(value.as_ref().or(field.default.as_ref()));
            let (ns_k, key) = (ns.to_owned(), field.key.clone());
            let control: AnyElement =
                match &field.kind {
                    FieldKind::Boolean => {
                        let on = value
                            .as_ref()
                            .or(field.default.as_ref())
                            .and_then(JsonValue::as_bool)
                            .unwrap_or(false);
                        let e = cx.entity().downgrade();
                        switch(
                            ElementId::Name(format!("m-hub-{key}").into()),
                            t,
                            on,
                            move |v, _, cx| {
                                let (ns, key) = (ns_k.clone(), key.clone());
                                let _ = e.update(cx, |this, cx| {
                                    this.write_field(&ns, &key, Some(JsonValue::Bool(v)), cx);
                                });
                            },
                        )
                        .into_any_element()
                    }
                    FieldKind::Action { command } => {
                        let cmd = command.clone();
                        crate::components::button::Btn::new(ElementId::Name(
                            format!("m-hub-{key}").into(),
                        ))
                        .label("Run")
                        .variant(crate::components::button::Variant::Outline)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let cmd = cmd.clone();
                            this.with_pane(cx, |p, cx| p.run_prompt(&cmd, cx));
                        }))
                        .into_any_element()
                    }
                    FieldKind::Enum(_) => {
                        let f = field.clone();
                        let cur = value.clone();
                        div()
                            .id(ElementId::Name(format!("m-hub-{key}").into()))
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .min_h(px(touch::TARGET))
                            .cursor_pointer()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.primary())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let next = f.cycle(cur.as_ref().or(f.default.as_ref()), false);
                                this.write_field(&ns_k, &key, next, cx);
                            }))
                            .child(SharedString::from(shown.clone()))
                            .child(icon(IconName::ChevronsRight, size::ICON_SM, t.primary()))
                            .into_any_element()
                    }
                    FieldKind::Order | FieldKind::MultiSelect(_) => div()
                        .max_w(px(size::MENU_MIN))
                        .truncate()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(shown.clone()))
                        .into_any_element(),
                    FieldKind::Text { .. }
                    | FieldKind::Number { .. }
                    | FieldKind::Secret
                    | FieldKind::Model => {
                        let label = field.label.clone();
                        div()
                            .id(ElementId::Name(format!("m-hub-{key}").into()))
                            .flex()
                            .items_center()
                            .gap(px(space::S1))
                            .max_w(px(size::MENU_MIN))
                            .min_h(px(touch::TARGET))
                            .cursor_pointer()
                            .text_role(t, TextRole::Sm)
                            .text_color(t.fg_subtle())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.open_sheet(
                                    Sheet::Edit {
                                        ns: ns_k.clone(),
                                        key: key.clone(),
                                        label: label.clone(),
                                    },
                                    cx,
                                );
                            }))
                            .child(div().truncate().child(SharedString::from(
                                if shown.is_empty() {
                                    "Not set".to_owned()
                                } else {
                                    shown.clone()
                                },
                            )))
                            .child(icon(IconName::Pencil, size::ICON_SM, t.fg_subtlest()))
                            .into_any_element()
                    }
                };
            rows.push(Self::setting_row(
                t,
                field.label.clone(),
                field.description.clone(),
                control,
            ));
        }
        if !rows.is_empty() {
            out.push(Self::section(t, section));
            out.push(Self::card(t, rows));
        }
        if out.len() == 1 {
            out.push(
                crate::components::empty::empty_compact(
                    t,
                    IconName::SlidersHorizontal,
                    "No settings",
                )
                .into_any_element(),
            );
        }
        out
    }

    fn settings_about(&mut self, t: &OinoTheme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let info = self.host.info();
        let has_updater = self
            .active_pane()
            .is_some_and(|p| p.read(cx).has_command("unipi:update"));
        let mut rows = vec![
            Self::setting_row(
                t,
                "Version",
                None,
                div()
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(info.version.clone()))
                    .into_any_element(),
            ),
            Self::setting_row(
                t,
                "Backend",
                None,
                div()
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(info.backend.clone()))
                    .into_any_element(),
            ),
            Self::setting_row(
                t,
                "Agent directory",
                Some(info.agent_dir.display().to_string()),
                div().into_any_element(),
            ),
        ];
        if has_updater {
            rows.push(
                Self::list_row(
                    t,
                    "m-about-update",
                    Some(IconName::CircleArrowDown),
                    "Check for updates",
                    None,
                    Some(Self::chevron(t)),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.back(window, cx);
                    this.with_pane(cx, |p, cx| p.run_prompt("/unipi:update", cx));
                }))
                .into_any_element(),
            );
        }
        vec![
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(space::S2))
                .py(px(space::S6))
                .child(crate::components::unicrab::unicrab(
                    crate::design::brand::CrabState::Idle,
                    size::CRAB.1,
                    cx,
                ))
                .child(
                    div()
                        .text_role(t, TextRole::Xl)
                        .weight_semibold()
                        .child("Oino"),
                )
                .into_any_element(),
            Self::card(t, rows),
        ]
    }
}
