//! D8 Settings (design.md §5.7): full main-area page with its own nav, App preferences,
//! Models & Providers (sign-in over `_oino/auth_providers|login|logout|set_api_key`), keyboard
//! shortcuts, and the extension settings hub over `_oino/get_settings_schema` /
//! `get_extension_settings` / `set_extension_setting` (schema fields per ui-protocol §6.6).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, EventEmitter,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use oino_ai::{JsonValue, Model};
use oino_client::protocol::SettingsScope;
use oino_client::{FrontendApi as _, SessionBackend};
use oino_core::extensions::settings::SettingsLayer;
use oino_core::session::AuthProviderInfo;
use oino_view::settings_hub::{self, FieldKind, HubField, HubNamespace};

use crate::components::badge::{badge, kbd};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls::{segmented, switch};
use crate::components::empty::empty_state;
use crate::components::settings::{group, page_title, row, section_title};
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::host::{
    AppSettings, ExtensionInfo, HostInfo, ReduceMotion, StatusPanelMode, ThemePref, ThinkingBlocks,
    ToolDensity,
};
use crate::model::ous::Tone;
use crate::model::ui_state::Shortcut;
use crate::rt::Tokio;

/// Settings pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsSection {
    /// General.
    General,
    /// Appearance.
    Appearance,
    /// Models & Providers.
    Models,
    /// Sessions.
    Sessions,
    /// Permissions (workflow rules live in the hub namespace `workflow`).
    Permissions,
    /// Keyboard shortcuts.
    Shortcuts,
    /// Extension settings namespace.
    Extension(String),
    /// Remote access.
    Remote,
    /// About & updates.
    About,
}

/// Emitted to the workspace.
#[derive(Clone, Debug)]
pub enum SettingsEvent {
    /// Back to sessions.
    Back,
    /// App preferences changed.
    Changed(AppSettings),
    /// Run a slash command in the active session.
    Command(String),
    /// Show a toast.
    Toast(String),
}

/// The settings page.
pub struct SettingsView {
    settings: AppSettings,
    backend: Option<Arc<dyn SessionBackend>>,
    models: Vec<Model>,
    shortcuts: Vec<Shortcut>,
    extensions: Vec<ExtensionInfo>,
    info: HostInfo,
    section: SettingsSection,
    schema: Vec<JsonValue>,
    namespaces: Vec<HubNamespace>,
    values: HashMap<String, JsonValue>,
    scope: SettingsScope,
    providers: Vec<AuthProviderInfo>,
    inputs: HashMap<String, (Entity<InputState>, Subscription)>,
    search: Entity<InputState>,
    _subscription: Subscription,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    /// The page, opened on `section`.
    #[allow(clippy::too_many_arguments, reason = "snapshot of shell state")]
    pub fn new(
        settings: AppSettings,
        backend: Option<Arc<dyn SessionBackend>>,
        models: Vec<Model>,
        shortcuts: Vec<Shortcut>,
        extensions: Vec<ExtensionInfo>,
        info: HostInfo,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings"));
        let sub = cx.subscribe(&search, |_, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                cx.notify();
            }
        });
        let view = Self {
            settings,
            backend,
            models,
            shortcuts,
            extensions,
            info,
            section,
            schema: Vec::new(),
            namespaces: Vec::new(),
            values: HashMap::new(),
            scope: SettingsScope::Global,
            providers: Vec::new(),
            inputs: HashMap::new(),
            search,
            _subscription: sub,
        };
        view.load_schema(cx);
        view.load_providers(cx);
        if let SettingsSection::Extension(ns) = view.section.clone() {
            view.load_values(&ns, cx);
        }
        view
    }

    fn load_schema(&self, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let task = Tokio::spawn(cx, async move { backend.settings_schema().await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(schema)) = task.await {
                let _ = this.update(cx, |v, cx| {
                    v.namespaces = settings_hub::namespaces(&schema);
                    v.schema = schema;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn load_providers(&self, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let task = Tokio::spawn(cx, async move { backend.auth_providers().await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(list)) = task.await {
                let _ = this.update(cx, |v, cx| {
                    v.providers = list;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn layer(&self) -> SettingsLayer {
        match self.scope {
            SettingsScope::Global => SettingsLayer::Global,
            SettingsScope::Project => SettingsLayer::Project,
        }
    }

    fn load_values(&self, ns: &str, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let (ns, layer) = (ns.to_owned(), self.layer());
        let key = ns.clone();
        let task = Tokio::spawn(
            cx,
            async move { backend.extension_settings(&ns, layer).await },
        );
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(values)) = task.await {
                let _ = this.update(cx, |v, cx| {
                    v.values.insert(key, values);
                    v.inputs.clear();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn set_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if let SettingsSection::Extension(ns) = &section {
            self.load_values(ns, cx);
        }
        self.section = section;
        cx.notify();
    }

    fn change(&mut self, f: impl FnOnce(&mut AppSettings), cx: &mut Context<Self>) {
        f(&mut self.settings);
        cx.emit(SettingsEvent::Changed(self.settings.clone()));
        cx.notify();
    }

    fn write(&mut self, ns: &str, key: &str, value: Option<JsonValue>, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let (ns_owned, key_owned, scope) = (ns.to_owned(), key.to_owned(), self.scope);
        let task = Tokio::spawn(cx, async move {
            backend
                .set_extension_setting(&ns_owned, &key_owned, value, scope)
                .await
        });
        let ns = ns.to_owned();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(Ok(())) => v.load_values(&ns, cx),
                Ok(Err(e)) => cx.emit(SettingsEvent::Toast(e.to_string())),
                Err(e) => cx.emit(SettingsEvent::Toast(e.to_string())),
            });
        })
        .detach();
    }

    fn auth(&mut self, provider: &str, action: &'static str, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let p = provider.to_owned();
        let task = Tokio::spawn(cx, async move {
            match action {
                "logout" => backend.logout(&p).await,
                _ => backend.login(&p).await,
            }
        });
        cx.emit(SettingsEvent::Toast(if action == "logout" {
            format!("Signing out of {provider}…")
        } else {
            format!("Signing in to {provider}… (answer the prompts in the session)")
        }));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                if let Ok(Err(e)) = result {
                    cx.emit(SettingsEvent::Toast(e.to_string()));
                }
                v.load_providers(cx);
            });
        })
        .detach();
    }

    fn input_for(
        &mut self,
        key: &str,
        initial: &str,
        secret: bool,
        on_enter: impl Fn(&mut Self, String, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((s, _)) = self.inputs.get(key) {
            return s.clone();
        }
        let initial = SharedString::from(initial.to_owned());
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(secret)
                .default_value(initial)
        });
        let sub = cx.subscribe(&state, move |this, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                let text = input.read(cx).value().to_string();
                on_enter(this, text, cx);
            }
        });
        self.inputs.insert(key.to_owned(), (state.clone(), sub));
        state
    }

    fn nav(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: ElementId,
                    label: SharedString,
                    section: SettingsSection,
                    active: bool,
                    cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .h(px(size::CONTROL_LG))
                .px(px(space::S2))
                .rounded(px(radius::R_LG))
                .cursor_pointer()
                .text_role(t, TextRole::Base)
                .text_color(if active { t.fg() } else { t.fg_subtle() })
                .when(active, |el| el.bg(t.selected()))
                .hover(|s| s.bg(t.hover()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_section(section.clone(), cx);
                }))
                .child(label)
                .into_any_element()
        };
        let s = &self.section;
        let fixed = [
            ("set-general", "General", SettingsSection::General),
            ("set-appearance", "Appearance", SettingsSection::Appearance),
            ("set-models", "Models & Providers", SettingsSection::Models),
            ("set-sessions", "Sessions", SettingsSection::Sessions),
            (
                "set-permissions",
                "Permissions",
                SettingsSection::Permissions,
            ),
            (
                "set-shortcuts",
                "Keyboard shortcuts",
                SettingsSection::Shortcuts,
            ),
        ];
        let mut rows: Vec<AnyElement> = fixed
            .into_iter()
            .map(|(id, label, section)| {
                let active = *s == section;
                item(
                    ElementId::Name(id.into()),
                    label.into(),
                    section,
                    active,
                    cx,
                )
            })
            .collect();
        rows.push(
            div()
                .pt(px(space::S4))
                .pb(px(space::S1))
                .px(px(space::S2))
                .text_role(t, TextRole::Xs)
                .weight_medium()
                .text_color(t.fg_subtlest())
                .child("EXTENSION SETTINGS")
                .into_any_element(),
        );
        let query = self.search.read(cx).value().to_lowercase();
        let namespaces: Vec<HubNamespace> = self
            .namespaces
            .iter()
            .filter(|n| {
                query.is_empty()
                    || n.title.to_lowercase().contains(&query)
                    || n.namespace.contains(&query)
            })
            .cloned()
            .collect();
        if namespaces.is_empty() {
            rows.push(
                div()
                    .px(px(space::S2))
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtlest())
                    .child(if self.backend.is_some() {
                        "No extension settings"
                    } else {
                        "Open a session to edit"
                    })
                    .into_any_element(),
            );
        }
        for (i, ns) in namespaces.into_iter().enumerate() {
            let active = matches!(s, SettingsSection::Extension(n) if *n == ns.namespace);
            rows.push(item(
                ElementId::NamedInteger("set-ns".into(), i as u64),
                SharedString::from(ns.title.clone()),
                SettingsSection::Extension(ns.namespace.clone()),
                active,
                cx,
            ));
        }
        rows.push(div().h(px(space::S4)).into_any_element());
        rows.push(item(
            ElementId::Name("set-remote".into()),
            "Remote access".into(),
            SettingsSection::Remote,
            *s == SettingsSection::Remote,
            cx,
        ));
        rows.push(item(
            ElementId::Name("set-about".into()),
            "About & updates".into(),
            SettingsSection::About,
            *s == SettingsSection::About,
            cx,
        ));
        div()
            .id("settings-nav")
            .flex()
            .flex_col()
            .w(px(size::SETTINGS_NAV))
            .flex_shrink_0()
            .h_full()
            .p(px(space::S2))
            .gap(px(space::HALF))
            .overflow_y_scroll()
            .border_r_1()
            .border_color(t.border())
            .child(
                Btn::new("settings-back")
                    .label("Back to sessions")
                    .icon(IconName::ArrowLeft)
                    .full()
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Back))),
            )
            .child(div().py(px(space::S2)).child(Input::new(&self.search)))
            .children(rows)
            .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "appearance rows")]
    fn appearance(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.settings.clone();
        let e = cx.entity().downgrade();
        let pick = |f: fn(&mut AppSettings, usize)| {
            let e = e.clone();
            Rc::new(move |i: usize, _: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |v, cx| v.change(|s| f(s, i), cx));
            }) as Rc<dyn Fn(usize, &mut Window, &mut App)>
        };
        let theme_ix = match s.theme {
            ThemePref::System => 0,
            ThemePref::Light => 1,
            ThemePref::Dark => 2,
        };
        let sizes = ["12", "13", "14", "15", "16", "18"];
        let code_sizes = ["11", "12", "13", "14", "16"];
        let ui_ix = sizes
            .iter()
            .position(|v| v.parse::<u8>().ok() == Some(s.ui_font_size))
            .unwrap_or(2);
        let code_ix = code_sizes
            .iter()
            .position(|v| v.parse::<u8>().ok() == Some(s.code_font_size))
            .unwrap_or(2);
        let rows = vec![
            row(
                t,
                "Theme",
                Some("Follows the system appearance by default.".into()),
                s.theme != ThemePref::System,
                segmented(
                    "set-theme",
                    t,
                    &["System", "Light", "Dark"],
                    theme_ix,
                    pick(|s, i| {
                        s.theme = [ThemePref::System, ThemePref::Light, ThemePref::Dark][i];
                    }),
                )
                .into_any_element(),
            ),
            row(
                t,
                "Interface size",
                Some("UI text size in px.".into()),
                s.ui_font_size != 14,
                segmented(
                    "set-ui-size",
                    t,
                    &sizes,
                    ui_ix,
                    pick(|s, i| s.ui_font_size = [12, 13, 14, 15, 16, 18][i]),
                )
                .into_any_element(),
            ),
            row(
                t,
                "Code font size",
                Some("Code, diffs and logs.".into()),
                s.code_font_size != 13,
                segmented(
                    "set-code-size",
                    t,
                    &code_sizes,
                    code_ix,
                    pick(|s, i| s.code_font_size = [11, 12, 13, 14, 16][i]),
                )
                .into_any_element(),
            ),
            row(
                t,
                "Reduce motion",
                None,
                s.reduce_motion != ReduceMotion::FollowOs,
                segmented(
                    "set-motion",
                    t,
                    &["Follow OS", "On", "Off"],
                    match s.reduce_motion {
                        ReduceMotion::FollowOs => 0,
                        ReduceMotion::On => 1,
                        ReduceMotion::Off => 2,
                    },
                    pick(|s, i| {
                        s.reduce_motion =
                            [ReduceMotion::FollowOs, ReduceMotion::On, ReduceMotion::Off][i];
                    }),
                )
                .into_any_element(),
            ),
            row(
                t,
                "Tool rows",
                Some("How much of each tool call the timeline shows.".into()),
                s.tool_density != ToolDensity::Simple,
                segmented(
                    "set-density",
                    t,
                    &["Simple", "Regular", "Advanced"],
                    match s.tool_density {
                        ToolDensity::Simple => 0,
                        ToolDensity::Regular => 1,
                        ToolDensity::Advanced => 2,
                    },
                    pick(|s, i| {
                        s.tool_density = [
                            ToolDensity::Simple,
                            ToolDensity::Regular,
                            ToolDensity::Advanced,
                        ][i];
                    }),
                )
                .into_any_element(),
            ),
            row(
                t,
                "Thinking blocks",
                None,
                s.thinking_blocks != ThinkingBlocks::Collapsed,
                segmented(
                    "set-thinking",
                    t,
                    &["Collapsed", "Expanded", "Hidden"],
                    match s.thinking_blocks {
                        ThinkingBlocks::Collapsed => 0,
                        ThinkingBlocks::Expanded => 1,
                        ThinkingBlocks::Hidden => 2,
                    },
                    pick(|s, i| {
                        s.thinking_blocks = [
                            ThinkingBlocks::Collapsed,
                            ThinkingBlocks::Expanded,
                            ThinkingBlocks::Hidden,
                        ][i];
                    }),
                )
                .into_any_element(),
            ),
            row(
                t,
                "Status panel",
                None,
                s.status_panel != StatusPanelMode::Auto,
                segmented(
                    "set-status",
                    t,
                    &["Auto", "Expanded", "Mini"],
                    match s.status_panel {
                        StatusPanelMode::Auto => 0,
                        StatusPanelMode::Expanded => 1,
                        StatusPanelMode::Mini => 2,
                    },
                    pick(|s, i| {
                        s.status_panel = [
                            StatusPanelMode::Auto,
                            StatusPanelMode::Expanded,
                            StatusPanelMode::Mini,
                        ][i];
                    }),
                )
                .into_any_element(),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .child(page_title(t, "Appearance"))
            .child(group(t, rows))
            .into_any_element()
    }

    fn models_page(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let providers: Vec<AnyElement> = self
            .providers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let id = p.id.clone();
                let id2 = p.id.clone();
                let state = if p.configured {
                    badge(
                        t,
                        Tone::Success,
                        p.stored.clone().unwrap_or_else(|| "env".into()),
                    )
                } else {
                    badge(t, Tone::Muted, "not configured")
                };
                let control = div()
                    .flex()
                    .gap(px(space::S2))
                    .child(state)
                    .when(p.oauth, |el| {
                        el.child(
                            Btn::new(ElementId::NamedInteger("login".into(), i as u64))
                                .label("Log in")
                                .variant(Variant::Outline)
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.auth(&id, "login", cx);
                                })),
                        )
                    })
                    .when(p.stored.is_some(), |el| {
                        el.child(
                            Btn::new(ElementId::NamedInteger("logout".into(), i as u64))
                                .label("Log out")
                                .variant(Variant::Ghost)
                                .size(BtnSize::Sm)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.auth(&id2, "logout", cx);
                                })),
                        )
                    })
                    .into_any_element();
                row(
                    t,
                    SharedString::from(p.name.clone()),
                    (!p.env.is_empty()).then(|| SharedString::from(p.env.join(", "))),
                    false,
                    control,
                )
            })
            .collect();
        let models: Vec<AnyElement> = self
            .models
            .iter()
            .map(|m| {
                row(
                    t,
                    SharedString::from(if m.name.is_empty() {
                        m.id.clone()
                    } else {
                        m.name.clone()
                    }),
                    Some(SharedString::from(format!(
                        "{} · {}k context",
                        m.provider,
                        m.context_window / 1000
                    ))),
                    false,
                    div()
                        .mono_text(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child(SharedString::from(m.id.clone()))
                        .into_any_element(),
                )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(page_title(t, "Models & Providers"))
            .child(section_title(t, "Providers"))
            .child(if providers.is_empty() {
                empty_state(
                    t,
                    IconName::Plug,
                    "No providers",
                    Some("Open a session to list providers.".into()),
                    Vec::new(),
                )
                .into_any_element()
            } else {
                group(t, providers).into_any_element()
            })
            .child(section_title(t, "Available models"))
            .child(group(t, models))
            .into_any_element()
    }

    fn shortcuts_page(&self, t: &OinoTheme) -> AnyElement {
        let app = [
            ("⌘K", "Command center"),
            ("⌘N", "New session"),
            ("⌘O", "New session in folder"),
            ("⌘M", "Model menu"),
            ("⌘⇧M", "Thinking menu"),
            ("⌘B", "Toggle sidebar"),
            ("⌘J", "Toggle side pane"),
            ("⌘`", "Toggle bottom frame"),
            ("⌘\\", "Split pane"),
            ("⌘F", "Find in conversation"),
            ("⌘L", "Focus composer"),
            ("⌘.", "Abort run (Esc in an empty composer)"),
            ("⌘⇧T", "Session tree"),
            ("⌘⇧S", "Status panel"),
            ("⌘,", "Settings"),
            ("Enter", "Send · steer while running"),
            ("Alt+Enter", "Queue a follow-up"),
            ("Shift+Enter", "New line"),
            ("Ctrl+Alt+Shift+F", "Frame-time overlay"),
        ];
        let mut rows: Vec<AnyElement> = app
            .iter()
            .map(|(k, d)| row(t, *d, None, false, kbd(t, *k).into_any_element()))
            .collect();
        rows.extend(self.shortcuts.iter().map(|s| {
            row(
                t,
                SharedString::from(s.description.clone()),
                Some(SharedString::from(s.id.clone())),
                false,
                kbd(t, s.keys.clone()).into_any_element(),
            )
        }));
        div()
            .flex()
            .flex_col()
            .child(page_title(t, "Keyboard shortcuts"))
            .child(group(t, rows))
            .into_any_element()
    }

    #[allow(clippy::too_many_lines, reason = "one arm per field type")]
    fn hub(
        &mut self,
        ns: &str,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fields: Vec<HubField> = settings_hub::fields(&self.schema, ns);
        let title = self
            .namespaces
            .iter()
            .find(|n| n.namespace == ns)
            .map_or_else(|| ns.to_owned(), |n| n.title.clone());
        let values = self.values.get(ns).cloned().unwrap_or(JsonValue::Null);
        let query = self.search.read(cx).value().to_lowercase();
        let scope_ix = usize::from(self.scope == SettingsScope::Project);
        let e = cx.entity().downgrade();
        let ns_owned = ns.to_owned();
        let scope = segmented(
            "hub-scope",
            t,
            &["Global", "Project"],
            scope_ix,
            Rc::new(move |i, _, cx| {
                let ns = ns_owned.clone();
                let _ = e.update(cx, |v, cx| {
                    v.scope = if i == 0 {
                        SettingsScope::Global
                    } else {
                        SettingsScope::Project
                    };
                    v.load_values(&ns, cx);
                });
            }),
        );
        let mut sections: Vec<(String, Vec<AnyElement>)> = Vec::new();
        for field in fields
            .into_iter()
            .filter(|f| query.is_empty() || f.label.to_lowercase().contains(&query))
        {
            let value = settings_hub::value_at(&values, &field.key).cloned();
            let modified = value.is_some() && value != field.default;
            let key = field.key.clone();
            let ns_s = ns.to_owned();
            let control: AnyElement = match &field.kind {
                FieldKind::Boolean => {
                    let on = value
                        .as_ref()
                        .or(field.default.as_ref())
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false);
                    let e = cx.entity().downgrade();
                    switch(
                        ElementId::Name(format!("hub-{key}").into()),
                        t,
                        on,
                        move |v, _, cx| {
                            let (ns, key) = (ns_s.clone(), key.clone());
                            let _ = e.update(cx, |this, cx| {
                                this.write(&ns, &key, Some(JsonValue::Bool(v)), cx);
                            });
                        },
                    )
                    .into_any_element()
                }
                FieldKind::Enum(options) if options.len() <= 3 => {
                    let current = value
                        .as_ref()
                        .or(field.default.as_ref())
                        .and_then(JsonValue::as_str)
                        .and_then(|c| options.iter().position(|(v, _)| v == c))
                        .unwrap_or(usize::MAX);
                    let labels: Vec<&str> = options.iter().map(|(_, l)| l.as_str()).collect();
                    let values_list: Vec<String> = options.iter().map(|(v, _)| v.clone()).collect();
                    let e = cx.entity().downgrade();
                    segmented(
                        format!("hub-{key}"),
                        t,
                        &labels,
                        current,
                        Rc::new(move |i, _, cx| {
                            let (ns, key) = (ns_s.clone(), key.clone());
                            if let Some(v) = values_list.get(i).cloned() {
                                let _ = e.update(cx, |this, cx| {
                                    this.write(&ns, &key, Some(JsonValue::String(v)), cx);
                                });
                            }
                        }),
                    )
                    .into_any_element()
                }
                FieldKind::Enum(options) => {
                    let current = value
                        .as_ref()
                        .or(field.default.as_ref())
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned);
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(space::S1))
                        .children(options.iter().enumerate().map(|(i, (v, l))| {
                            let (ns, key, v2) = (ns_s.clone(), key.clone(), v.clone());
                            Btn::new(ElementId::Name(format!("hub-{}-{i}", field.key).into()))
                                .label(SharedString::from(l.clone()))
                                .size(BtnSize::Sm)
                                .variant(Variant::Outline)
                                .selected(current.as_deref() == Some(v.as_str()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.write(&ns, &key, Some(JsonValue::String(v2.clone())), cx);
                                }))
                        }))
                        .into_any_element()
                }
                FieldKind::Text { .. }
                | FieldKind::Number { .. }
                | FieldKind::Secret
                | FieldKind::Model => {
                    let secret = matches!(field.kind, FieldKind::Secret);
                    let shown = if secret {
                        String::new()
                    } else {
                        field.display(value.as_ref().or(field.default.as_ref()))
                    };
                    let f2 = field.clone();
                    let input = self.input_for(
                        &format!("{ns}/{}", field.key),
                        &shown,
                        secret,
                        move |this, text, cx| match f2.parse(&text) {
                            Ok(v) => this.write(&ns_s, &f2.key, v, cx),
                            Err(e) => cx.emit(SettingsEvent::Toast(e)),
                        },
                        window,
                        cx,
                    );
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::S2))
                        .w(px(size::SIDEBAR))
                        .child(div().flex_1().child(Input::new(&input)))
                        .when(secret && value.is_some(), |el| {
                            el.child(badge(t, Tone::Success, "Stored"))
                        })
                        .into_any_element()
                }
                FieldKind::Action { command } => {
                    let command = format!("/{}", command.trim_start_matches('/'));
                    Btn::new(ElementId::Name(format!("hub-{key}").into()))
                        .label(SharedString::from(field.label.clone()))
                        .variant(Variant::Outline)
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SettingsEvent::Command(command.clone()));
                        }))
                        .into_any_element()
                }
                FieldKind::MultiSelect(_) | FieldKind::Order => div()
                    .text_role(t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(
                        field.display(value.as_ref().or(field.default.as_ref())),
                    ))
                    .into_any_element(),
            };
            let reset_key = field.key.clone();
            let reset_ns = ns.to_owned();
            let control = div()
                .flex()
                .items_center()
                .gap(px(space::S1))
                .when(modified, |el| {
                    el.child(
                        Btn::icon_only(
                            ElementId::Name(format!("reset-{}", field.key).into()),
                            IconName::RotateCcw,
                            "Reset to default",
                        )
                        .size(BtnSize::Sm)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.write(&reset_ns, &reset_key, None, cx);
                        })),
                    )
                })
                .child(control)
                .into_any_element();
            let el = row(
                t,
                SharedString::from(field.label.clone()),
                field.description.clone().map(SharedString::from),
                modified,
                control,
            );
            match sections.iter_mut().find(|(s, _)| *s == field.section) {
                Some((_, rows)) => rows.push(el),
                None => sections.push((field.section.clone(), vec![el])),
            }
        }
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(page_title(t, title))
                    .child(scope),
            )
            .when(sections.is_empty(), |el| {
                el.child(empty_state(
                    t,
                    IconName::SlidersHorizontal,
                    "Nothing to configure",
                    None,
                    Vec::new(),
                ))
            })
            .children(sections.into_iter().map(|(title, rows)| {
                div()
                    .flex()
                    .flex_col()
                    .when(!title.is_empty(), |el| {
                        el.child(section_title(t, SharedString::from(title)))
                    })
                    .child(group(t, rows))
            }))
            .into_any_element()
    }

    fn simple_page(
        &self,
        t: &OinoTheme,
        title: &'static str,
        rows: Vec<(&'static str, String)>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(page_title(t, title))
            .child(group(
                t,
                rows.into_iter()
                    .map(|(l, v)| {
                        row(
                            t,
                            l,
                            None,
                            false,
                            div()
                                .text_role(t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .child(SharedString::from(v))
                                .into_any_element(),
                        )
                    })
                    .collect(),
            ))
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let nav = self.nav(&t, cx);
        let body = match self.section.clone() {
            SettingsSection::Appearance => self.appearance(&t, cx),
            SettingsSection::Models => self.models_page(&t, cx),
            SettingsSection::Shortcuts => self.shortcuts_page(&t),
            SettingsSection::Extension(ns) => self.hub(&ns, &t, window, cx),
            SettingsSection::General => self.simple_page(&t, "General", vec![
                ("Default folder", self.info.default_cwd.display().to_string()),
                ("Recent folders", self.settings.recent_cwds.len().to_string()),
                ("Language", "English".into()),
                ("Telemetry", "None".into()),
            ]),
            SettingsSection::Sessions => self.simple_page(&t, "Sessions", vec![
                ("Session directory", self.info.agent_dir.join("sessions").display().to_string()),
                ("Keep warm", "30 s after the last view closes".into()),
            ]),
            SettingsSection::Permissions => {
                if self.namespaces.iter().any(|n| n.namespace == "workflow") {
                    self.hub("workflow", &t, window, cx)
                } else {
                    self.simple_page(&t, "Permissions", vec![("Workflow", "unipi.workflow is not loaded".into())])
                }
            }
            SettingsSection::Remote => self.simple_page(&t, "Remote access", vec![("Serve", "Not available in v1.0.0-alpha.1 (`oino serve` pending)".into())]),
            SettingsSection::About => self.simple_page(&t, "About & updates", vec![
                ("Version", format!("Oino Desktop {}", self.info.version)),
                ("Backend", self.info.backend.clone()),
                ("Agent dir", self.info.agent_dir.display().to_string()),
                ("Extensions", self.extensions.len().to_string()),
                ("Licenses", "Lucide (ISC), Inter (OFL 1.1), JetBrains Mono (OFL 1.1), gpui-kit (Apache-2.0)".into()),
            ]),
        };
        div().flex().flex_row().size_full().child(nav).child(
            div()
                .id("settings-content")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .px(px(space::S8))
                .py(px(space::S8))
                .child(
                    div()
                        .max_w(px(size::SETTINGS_CONTENT))
                        .mx_auto()
                        .child(body),
                ),
        )
    }
}
