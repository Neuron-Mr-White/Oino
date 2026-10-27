//! D8 Settings (design.md A2.16): `PageHeader` "Settings" with a search tool (⌘F), then a
//! 200 px section nav and the content (max 760): each section is a section label with an
//! icon over a grouped card of rows (icon tile 28, label 15/500, description, control).
//! General and Sessions edit core settings over `_oino/get_settings` / `set_setting`
//! (GA-37); Models & Providers signs in over `_oino/auth_providers|login|logout`; Keyboard
//! shortcuts lists app and extension shortcuts (GA-16); the extension settings hub runs over
//! `_oino/get_settings_schema` / `get_extension_settings` / `set_extension_setting`
//! (ui-protocol §6.6); Permissions is the `unipi.workflow` hub namespace (GA-33).

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::Focusable as _;

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
use crate::components::page_header::PageHeader;
use crate::components::section_label::with_icon;
use crate::components::settings::{group, row, row_icon};
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
    /// Permissions (the hub namespace of extension `unipi.workflow`).
    Permissions,
    /// Keyboard shortcuts.
    Shortcuts,
    /// Extension settings namespace.
    Extension(String),
    /// Unicrab (mascot window, crab size).
    Unicrab,
    /// About & updates.
    About,
}

impl SettingsSection {
    /// Nav label and icon.
    fn label(&self) -> (&'static str, IconName) {
        match self {
            Self::General => ("General", IconName::SlidersHorizontal),
            Self::Appearance => ("Appearance", IconName::Palette),
            Self::Unicrab => ("Unicrab", IconName::Sparkles),
            Self::Models => ("Models & providers", IconName::Plug),
            Self::Sessions => ("Sessions", IconName::MessageSquare),
            Self::Permissions => ("Permissions", IconName::Shield),
            Self::Shortcuts => ("Keyboard shortcuts", IconName::Keyboard),
            Self::Extension(_) => ("Extension", IconName::Blocks),
            Self::About => ("About & updates", IconName::Info),
        }
    }
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
    /// Open the Extensions page (D12).
    OpenExtensions,
}

/// Thinking levels of Settings → General (`defaultThinkingLevel`).
const LEVELS: [&str; 4] = ["off", "low", "medium", "high"];

/// A `−  n  +` stepper for small integer ranges (font sizes).
fn stepper(
    id: &'static str,
    t: &OinoTheme,
    value: u8,
    (lo, hi): (u8, u8),
    on_change: Rc<dyn Fn(u8, &mut App)>,
) -> AnyElement {
    let (dec, inc) = (Rc::clone(&on_change), on_change);
    div()
        .flex()
        .items_center()
        .gap(px(space::S1))
        .child(
            Btn::icon_only(
                ElementId::Name(format!("{id}-dec").into()),
                IconName::Minus,
                "Smaller",
            )
            .size(BtnSize::Sm)
            .variant(Variant::Outline)
            .disabled(value <= lo)
            .on_click(move |_, _, cx| dec(value.saturating_sub(1).max(lo), cx)),
        )
        .child(
            div()
                .w(px(size::CONTROL))
                .flex()
                .justify_center()
                .mono_text(t, TextRole::Base)
                .child(SharedString::from(value.to_string())),
        )
        .child(
            Btn::icon_only(
                ElementId::Name(format!("{id}-inc").into()),
                IconName::Plus,
                "Larger",
            )
            .size(BtnSize::Sm)
            .variant(Variant::Outline)
            .disabled(value >= hi)
            .on_click(move |_, _, cx| inc(value.saturating_add(1).min(hi), cx)),
        )
        .into_any_element()
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
    /// Provider whose "Log out" waits for confirmation (app-convention: important actions).
    confirm_logout: Option<String>,
    /// Merged core settings (`_oino/get_settings`).
    core: JsonValue,
    /// The default-model menu is open.
    model_menu: bool,
    focus: gpui_kit::FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl gpui_kit::Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> gpui_kit::FocusHandle {
        self.focus.clone()
    }
}

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
            confirm_logout: None,
            core: JsonValue::Null,
            model_menu: false,
            focus: cx.focus_handle(),
            _subscription: sub,
        };
        view.load_schema(cx);
        view.load_providers(cx);
        view.load_core(cx);
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

    fn load_core(&self, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let task = Tokio::spawn(cx, async move { backend.settings().await });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(core)) = task.await {
                let _ = this.update(cx, |v, cx| {
                    v.core = core;
                    v.inputs.retain(|k, _| !k.starts_with("core/"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The core setting at dotted `key`.
    fn core_at(&self, key: &str) -> Option<&JsonValue> {
        key.split('.').try_fold(&self.core, |v, part| v.get(part))
    }

    /// Writes core setting `key` (global scope) and reloads.
    fn set_core(&mut self, key: &str, value: Option<JsonValue>, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            cx.emit(SettingsEvent::Toast("Open a session to change this".into()));
            return;
        };
        let key = key.to_owned();
        let task = Tokio::spawn(cx, async move {
            backend
                .set_setting(&key, value, SettingsScope::Global)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(Ok(())) => {
                    cx.emit(SettingsEvent::Toast(
                        "Saved · applies to new sessions".into(),
                    ));
                    v.load_core(cx);
                }
                Ok(Err(e)) => cx.emit(SettingsEvent::Toast(e.to_string())),
                Err(e) => cx.emit(SettingsEvent::Toast(e.to_string())),
            });
        })
        .detach();
    }

    /// The hub namespace of the workflow extension (GA-33: matched by extension id, the
    /// namespace itself is named after the page).
    fn permissions_ns(&self) -> Option<String> {
        self.namespaces
            .iter()
            .find(|n| n.extension_id == "unipi.workflow" || n.namespace == "workflow")
            .map(|n| n.namespace.clone())
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
        if section == SettingsSection::Permissions
            && let Some(ns) = self.permissions_ns()
        {
            self.load_values(&ns, cx);
        }
        self.section = section;
        cx.notify();
    }

    /// Sections in nav order (core pages, then extension namespaces).
    fn sections(&self) -> Vec<SettingsSection> {
        let mut out = vec![
            SettingsSection::General,
            SettingsSection::Appearance,
            SettingsSection::Unicrab,
            SettingsSection::Models,
            SettingsSection::Sessions,
            SettingsSection::Permissions,
            SettingsSection::Shortcuts,
        ];
        let permissions = self.permissions_ns();
        out.extend(
            self.namespaces
                .iter()
                .filter(|n| Some(&n.namespace) != permissions.as_ref())
                .map(|n| SettingsSection::Extension(n.namespace.clone())),
        );
        out.push(SettingsSection::About);
        out
    }

    /// Settings keys (design.md §4 D8): ⌘F search, ↑/↓ previous / next page, Esc back.
    fn on_key(&mut self, e: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if crate::components::secondary(&k.modifiers) && k.key == "f" {
            self.search.update(cx, |s, cx| s.focus(window, cx));
            cx.stop_propagation();
            return;
        }
        if self.search.read(cx).focus_handle(cx).is_focused(window)
            || self
                .inputs
                .values()
                .any(|(i, _)| i.read(cx).focus_handle(cx).is_focused(window))
        {
            return;
        }
        let all = self.sections();
        let i = all.iter().position(|s| *s == self.section).unwrap_or(0);
        let to = match k.key.as_str() {
            "down" | "j" => (i + 1).min(all.len() - 1),
            "up" | "k" => i.saturating_sub(1),
            "escape" => {
                cx.emit(SettingsEvent::Back);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.set_section(all[to].clone(), cx);
        cx.stop_propagation();
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
                    glyph: Option<IconName>,
                    section: SettingsSection,
                    active: bool,
                    cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(space::S2_5))
                .h(px(size::NAV_ITEM))
                .px(px(space::S2_5))
                .rounded(px(radius::R_ROW))
                .cursor_pointer()
                .text_role(t, TextRole::Base)
                .weight_medium()
                .text_color(if active { t.fg() } else { t.fg_subtle() })
                .when(active, |el| {
                    el.bg(t.selected())
                        .border_1()
                        .border_color(t.selected_border())
                })
                .when(!active, |el| el.hover(|s| s.bg(t.hover())))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_section(section.clone(), cx);
                }))
                .when_some(glyph, |el, g| {
                    el.child(crate::design::icons::icon(
                        g,
                        size::ICON,
                        if active { t.fg() } else { t.fg_subtlest() },
                    ))
                })
                .child(div().flex_1().min_w_0().truncate().child(label))
                .into_any_element()
        };
        let s = &self.section;
        let query = self.search.read(cx).value().to_lowercase();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, section) in [
            SettingsSection::General,
            SettingsSection::Appearance,
            SettingsSection::Unicrab,
            SettingsSection::Models,
            SettingsSection::Sessions,
            SettingsSection::Permissions,
            SettingsSection::Shortcuts,
        ]
        .into_iter()
        .enumerate()
        {
            let (label, glyph) = section.label();
            let active = *s == section;
            rows.push(item(
                ElementId::NamedInteger("set-core".into(), i as u64),
                label.into(),
                Some(glyph),
                section,
                active,
                cx,
            ));
        }
        let permissions = self.permissions_ns();
        let namespaces: Vec<HubNamespace> = self
            .namespaces
            .iter()
            .filter(|n| Some(&n.namespace) != permissions.as_ref())
            .filter(|n| {
                query.is_empty()
                    || n.title.to_lowercase().contains(&query)
                    || n.namespace.contains(&query)
            })
            .cloned()
            .collect();
        rows.push(
            div()
                .px(px(space::S2_5))
                .child(crate::components::section_label::section_row(
                    t,
                    "Extension settings",
                    false,
                    None,
                ))
                .into_any_element(),
        );
        if namespaces.is_empty() {
            rows.push(
                div()
                    .px(px(space::S2_5))
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
                None,
                SettingsSection::Extension(ns.namespace.clone()),
                active,
                cx,
            ));
        }
        rows.push(
            div()
                .id("set-extensions")
                .flex()
                .items_center()
                .gap(px(space::S2))
                .h(px(size::NAV_ITEM))
                .px(px(space::S2_5))
                .rounded(px(radius::R_ROW))
                .cursor_pointer()
                .text_role(t, TextRole::Base)
                .text_color(t.fg_subtle())
                .hover(|s| s.bg(t.hover()))
                .on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(SettingsEvent::OpenExtensions)),
                )
                .child(div().flex_1().child("Manage extensions"))
                .child(crate::design::icons::icon(
                    IconName::ChevronRight,
                    size::ICON_SM,
                    t.fg_subtlest(),
                ))
                .into_any_element(),
        );
        rows.push(div().h(px(space::S4)).into_any_element());
        let (about, about_icon) = SettingsSection::About.label();
        rows.push(item(
            ElementId::Name("set-about".into()),
            about.into(),
            Some(about_icon),
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
            .pl(px(size::PAGE_GUTTER - space::S2_5))
            .pb(px(space::S4))
            .gap(px(space::HALF))
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    }

    /// A section head: section label with an icon (A2.16).
    fn head(t: &OinoTheme, section: &SettingsSection, first: bool) -> AnyElement {
        let (label, glyph) = section.label();
        with_icon(t, glyph, label, first).into_any_element()
    }

    /// Settings → General (GA-37): default model and thinking, default folder, startup.
    fn general(
        &mut self,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider = self
            .core_at("defaultProvider")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let model = self
            .core_at("defaultModel")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let current = match (&provider, &model) {
            (Some(p), Some(m)) => self
                .models
                .iter()
                .find(|x| x.provider == *p && x.id == *m)
                .map_or_else(
                    || format!("{p}/{m}"),
                    |x| {
                        if x.name.is_empty() {
                            x.id.clone()
                        } else {
                            x.name.clone()
                        }
                    },
                ),
            (None, Some(m)) => m.clone(),
            _ => "Router default".to_owned(),
        };
        let e = cx.entity().downgrade();
        let picker = crate::components::menu::popover(
            Btn::new("set-default-model")
                .label(SharedString::from(current))
                .trailing(IconName::ChevronDown)
                .variant(Variant::Secondary)
                .size(BtnSize::Md)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.model_menu = !this.model_menu;
                    cx.notify();
                })),
            self.model_menu,
            crate::components::menu::Placement::BelowEnd,
            || {
                let entries = self
                    .models
                    .iter()
                    .take(40)
                    .map(|m| {
                        let (p, id) = (m.provider.clone(), m.id.clone());
                        let e = e.clone();
                        let checked = provider.as_deref() == Some(m.provider.as_str())
                            && model.as_deref() == Some(m.id.as_str());
                        crate::components::menu::MenuEntry::item(
                            SharedString::from(if m.name.is_empty() {
                                m.id.clone()
                            } else {
                                m.name.clone()
                            }),
                            move |_, cx| {
                                let (p, id) = (p.clone(), id.clone());
                                let _ = e.update(cx, |v, cx| {
                                    v.model_menu = false;
                                    v.set_core("defaultProvider", Some(JsonValue::String(p)), cx);
                                    v.set_core("defaultModel", Some(JsonValue::String(id)), cx);
                                });
                            },
                        )
                        .detail(SharedString::from(m.provider.clone()))
                        .checked(checked)
                    })
                    .collect();
                crate::components::menu::menu("default-model-menu", t, size::MODEL_MENU, entries)
                    .max_h(size::DIALOG_MD)
                    .into_any_element()
            },
            {
                let e = cx.entity().downgrade();
                move |_: &mut Window, cx: &mut App| {
                    let _ = e.update(cx, |v, cx| {
                        v.model_menu = false;
                        cx.notify();
                    });
                }
            },
        );
        let level = self
            .core_at("defaultThinkingLevel")
            .and_then(JsonValue::as_str)
            .unwrap_or("off")
            .to_owned();
        let level_ix = LEVELS.iter().position(|l| *l == level).unwrap_or(0);
        let e2 = cx.entity().downgrade();
        let thinking = segmented(
            "set-default-thinking",
            t,
            &["Off", "Low", "Medium", "High"],
            level_ix,
            Rc::new(move |i, _, cx| {
                let _ = e2.update(cx, |v, cx| {
                    v.set_core(
                        "defaultThinkingLevel",
                        Some(JsonValue::String(LEVELS[i].to_owned())),
                        cx,
                    );
                });
            }),
        );
        let folder = self
            .settings
            .default_cwd
            .clone()
            .unwrap_or_else(|| self.info.default_cwd.display().to_string());
        let folder_input = self.input_for(
            "app/default-cwd",
            &folder,
            false,
            |this, text, cx| {
                let text = text.trim().to_owned();
                this.change(|s| s.default_cwd = (!text.is_empty()).then_some(text), cx);
                cx.emit(SettingsEvent::Toast("Default folder saved".into()));
            },
            window,
            cx,
        );
        let e3 = cx.entity().downgrade();
        let reopen = switch(
            "set-reopen",
            t,
            self.settings.reopen_last,
            move |on, _, cx| {
                let _ = e3.update(cx, |v, cx| v.change(|s| s.reopen_last = on, cx));
            },
        );
        let rows = vec![
            row_icon(
                t,
                IconName::Sparkles,
                "Default model",
                Some("New sessions start with this model.".into()),
                model.is_some(),
                picker.into_any_element(),
            ),
            row_icon(
                t,
                IconName::Brain,
                "Default thinking",
                None,
                level != "off",
                thinking.into_any_element(),
            ),
            row_icon(
                t,
                IconName::FolderOpen,
                "Default folder",
                Some("Where new sessions start. Press Enter to save.".into()),
                self.settings.default_cwd.is_some(),
                div()
                    .w(px(size::SIDEBAR + space::S10))
                    .child(Input::new(&folder_input))
                    .into_any_element(),
            ),
            row_icon(
                t,
                IconName::RotateCcw,
                "Reopen the last session",
                Some("At startup, instead of a new empty session.".into()),
                !self.settings.reopen_last,
                reopen.into_any_element(),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .child(Self::head(t, &SettingsSection::General, true))
            .child(group(t, rows))
            .into_any_element()
    }

    /// Settings → Sessions (GA-37): session folder, queue delivery, compaction, retry.
    fn sessions_page(
        &mut self,
        t: &OinoTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dir = self
            .core_at("sessionDir")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let shown = dir
            .clone()
            .unwrap_or_else(|| self.info.agent_dir.join("sessions").display().to_string());
        let dir_input = self.input_for(
            "core/sessionDir",
            &shown,
            false,
            |this, text, cx| {
                let text = text.trim().to_owned();
                this.set_core(
                    "sessionDir",
                    (!text.is_empty()).then_some(JsonValue::String(text)),
                    cx,
                );
            },
            window,
            cx,
        );
        let queue = |key: &'static str, this: &Self| {
            let all = this.core_at(key).and_then(JsonValue::as_str) == Some("all");
            let e = cx.entity().downgrade();
            segmented(
                format!("set-{key}"),
                t,
                &["One at a time", "All at once"],
                usize::from(all),
                Rc::new(move |i, _, cx| {
                    let _ = e.update(cx, |v, cx| {
                        v.set_core(
                            key,
                            Some(JsonValue::String(
                                if i == 1 { "all" } else { "one-at-a-time" }.into(),
                            )),
                            cx,
                        );
                    });
                }),
            )
            .into_any_element()
        };
        let flag = |key: &'static str, id: &'static str, this: &Self| {
            let on = this
                .core_at(key)
                .and_then(JsonValue::as_bool)
                .unwrap_or(true);
            let e = cx.entity().downgrade();
            switch(id, t, on, move |v, _, cx| {
                let _ = e.update(cx, |this, cx| {
                    this.set_core(key, Some(JsonValue::Bool(v)), cx);
                });
            })
            .into_any_element()
        };
        let reveal = PathBuf::from(&shown);
        let rows = vec![
            row_icon(
                t,
                IconName::FolderOpen,
                "Session folder",
                Some("Where session files are written. Press Enter to save.".into()),
                dir.is_some(),
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S1))
                    .child(div().w(px(size::SIDEBAR)).child(Input::new(&dir_input)))
                    .child(
                        Btn::icon_only("set-reveal-sessions", IconName::FolderSearch, "Reveal")
                            .on_click(move |_, _, cx| cx.reveal_path(&reveal)),
                    )
                    .into_any_element(),
            ),
            row_icon(
                t,
                IconName::CornerDownRight,
                "Steering messages",
                Some("How messages typed during a run are delivered.".into()),
                self.core_at("steeringMode").is_some(),
                queue("steeringMode", self),
            ),
            row_icon(
                t,
                IconName::ListTree,
                "Follow-ups",
                Some("How queued follow-ups are delivered after a run.".into()),
                self.core_at("followUpMode").is_some(),
                queue("followUpMode", self),
            ),
            row_icon(
                t,
                IconName::Archive,
                "Auto-compact",
                Some("Summarise old turns when the context fills up.".into()),
                self.core_at("compaction.enabled").is_some(),
                flag("compaction.enabled", "set-compaction", self),
            ),
            row_icon(
                t,
                IconName::RefreshCw,
                "Retry on errors",
                Some("Retry provider errors such as rate limits.".into()),
                self.core_at("retry.enabled").is_some(),
                flag("retry.enabled", "set-retry", self),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .child(Self::head(t, &SettingsSection::Sessions, true))
            .child(group(t, rows))
            .into_any_element()
    }

    /// Settings → About & updates (GA-37): version, Check for updates, licences, a lore line.
    fn about(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let value = |text: String| {
            div()
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtle())
                .max_w(px(size::SIDE_PANE_MIN))
                .truncate()
                .child(SharedString::from(text))
                .into_any_element()
        };
        let rows = vec![
            row_icon(
                t,
                IconName::Info,
                SharedString::from(format!("Oino {}", self.info.version)),
                Some(SharedString::from(format!(
                    "Backend: {}",
                    self.info.backend
                ))),
                false,
                Btn::new("about-check")
                    .label("Check for updates")
                    .icon(IconName::CircleArrowDown)
                    .variant(Variant::Primary)
                    .size(BtnSize::Md)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(SettingsEvent::Command("/update-check".into()));
                        cx.emit(SettingsEvent::Toast("Checking for updates…".into()));
                    }))
                    .into_any_element(),
            ),
            row_icon(
                t,
                IconName::BookOpen,
                "Readme and changelog",
                Some("Package READMEs, including Oino's own.".into()),
                false,
                Btn::new("about-readme")
                    .label("Open")
                    .variant(Variant::Secondary)
                    .size(BtnSize::Md)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(SettingsEvent::Command("/readme".into()));
                        cx.emit(SettingsEvent::Back);
                    }))
                    .into_any_element(),
            ),
            row_icon(
                t,
                IconName::FolderOpen,
                "Agent folder",
                None,
                false,
                value(self.info.agent_dir.display().to_string()),
            ),
            row_icon(
                t,
                IconName::Blocks,
                "Extensions",
                None,
                false,
                value(self.extensions.len().to_string()),
            ),
            row_icon(
                t,
                IconName::FileText,
                "Licences",
                Some(
                    "Lucide (ISC), Inter (OFL 1.1), JetBrains Mono (OFL 1.1), gpui-kit \
                     (Apache-2.0)"
                        .into(),
                ),
                false,
                div().into_any_element(),
            ),
        ];
        div()
            .flex()
            .flex_col()
            .child(Self::head(t, &SettingsSection::About, true))
            .child(group(t, rows))
            .child(
                div()
                    .pt(px(space::S5))
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtlest())
                    .child("Unicrab walks sideways so it can read your diffs from both ends."),
            )
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
        let rows = vec![
            row_icon(
                t,
                IconName::Palette,
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
            row_icon(
                t,
                IconName::ALargeSmall,
                "Interface size",
                Some("UI text size in px (12–18).".into()),
                s.ui_font_size != 14,
                stepper("set-ui-size", t, s.ui_font_size, (12, 18), {
                    let e = e.clone();
                    Rc::new(move |v, cx: &mut App| {
                        let _ = e.update(cx, |view, cx| view.change(|s| s.ui_font_size = v, cx));
                    })
                }),
            ),
            row_icon(
                t,
                IconName::Code,
                "Code font size",
                Some("Code, diffs, logs and the terminal (10–20).".into()),
                s.code_font_size != 12,
                stepper("set-code-size", t, s.code_font_size, (10, 20), {
                    let e = e.clone();
                    Rc::new(move |v, cx: &mut App| {
                        let _ = e.update(cx, |view, cx| view.change(|s| s.code_font_size = v, cx));
                    })
                }),
            ),
            row_icon(
                t,
                IconName::PanelLeft,
                "Sidebar density",
                None,
                s.sidebar_compact,
                segmented(
                    "set-sidebar-density",
                    t,
                    &["Comfortable", "Compact"],
                    usize::from(s.sidebar_compact),
                    pick(|s, i| s.sidebar_compact = i == 1),
                )
                .into_any_element(),
            ),
            row_icon(
                t,
                IconName::Zap,
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
            row_icon(
                t,
                IconName::Wrench,
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
            row_icon(
                t,
                IconName::Brain,
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
            row_icon(
                t,
                IconName::PanelRight,
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
            .child(Self::head(t, &SettingsSection::Appearance, true))
            .child(group(t, rows))
            .into_any_element()
    }

    /// Settings → Unicrab (A2.13): mascot window, crab size; native Wayland on Linux.
    fn unicrab(&self, t: &OinoTheme, cx: &mut Context<Self>) -> AnyElement {
        let s = self.settings.clone();
        let e = cx.entity().downgrade();
        let pick = |f: fn(&mut AppSettings, usize)| {
            let e = e.clone();
            Rc::new(move |i: usize, _: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |v, cx| v.change(|s| f(s, i), cx));
            }) as Rc<dyn Fn(usize, &mut Window, &mut App)>
        };
        let toggle = |f: fn(&mut AppSettings, bool)| {
            let e = e.clone();
            move |on: bool, _: &mut Window, cx: &mut App| {
                let _ = e.update(cx, |v, cx| v.change(|s| f(s, on), cx));
            }
        };
        let mut rows = vec![
            row_icon(
                t,
                IconName::AppWindow,
                "Mascot window",
                Some("A floating Unicrab above other windows. It shows the overall state and pops a bubble when a session needs you or finishes.".into()),
                s.mascot_window,
                switch("set-mascot", t, s.mascot_window, toggle(|s, on| s.mascot_window = on))
                    .into_any_element(),
            ),
            row_icon(
                t,
                IconName::Maximize,
                "Crab size",
                None,
                s.mascot_size != 96,
                segmented(
                    "set-mascot-size",
                    t,
                    &["Small", "Medium", "Large"],
                    match s.mascot_size {
                        0..=80 => 0,
                        81..=110 => 1,
                        _ => 2,
                    },
                    pick(|s, i| s.mascot_size = [72, 96, 128][i]),
                )
                .into_any_element(),
            ),
        ];
        if cfg!(target_os = "linux") {
            rows.push(row_icon(
                t,
                IconName::Monitor,
                "Native Wayland",
                Some("Run on Wayland instead of XWayland. Turns off the embedded browser (WebKitGTK embeds into X11 windows only). Takes effect after a restart.".into()),
                s.native_wayland,
                switch("set-wayland", t, s.native_wayland, toggle(|s, on| s.native_wayland = on))
                    .into_any_element(),
            ));
        }
        div()
            .flex()
            .flex_col()
            .child(Self::head(t, &SettingsSection::Unicrab, true))
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
                        if self.confirm_logout.as_deref() == Some(id2.as_str()) {
                            let id3 = id2.clone();
                            el.child(
                                div()
                                    .text_role(t, TextRole::Sm)
                                    .text_color(t.fg_subtle())
                                    .child("Sign out?"),
                            )
                            .child(
                                Btn::new(ElementId::NamedInteger("logout-no".into(), i as u64))
                                    .label("Cancel")
                                    .variant(Variant::Ghost)
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirm_logout = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Btn::new(ElementId::NamedInteger("logout-yes".into(), i as u64))
                                    .label("Log out")
                                    .icon(IconName::LogOut)
                                    .variant(Variant::Danger)
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirm_logout = None;
                                        this.auth(&id3, "logout", cx);
                                    })),
                            )
                        } else {
                            el.child(
                                Btn::new(ElementId::NamedInteger("logout".into(), i as u64))
                                    .label("Log out")
                                    .variant(Variant::Ghost)
                                    .size(BtnSize::Sm)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirm_logout = Some(id2.clone());
                                        cx.notify();
                                    })),
                            )
                        }
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
            .child(Self::head(t, &SettingsSection::Models, true))
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
            .child(crate::components::section_label::section_row(
                t,
                "Available models",
                false,
                Some(crate::components::section_label::count(
                    t,
                    self.models.len(),
                )),
            ))
            .child(group(t, models))
            .into_any_element()
    }

    /// Settings → Keyboard shortcuts: app keys, then extension shortcuts (`setShortcuts`,
    /// GA-16) with their action id.
    fn shortcuts_page(&self, t: &OinoTheme) -> AnyElement {
        let app = [
            ("⌘K", "Search and commands"),
            ("⌘O", "Switch session"),
            ("⌘N", "New session"),
            ("⌘⇧O", "New session in folder"),
            ("⌘1", "Sessions"),
            ("⌘2", "Board"),
            ("⌘3", "Extensions"),
            ("⌘,", "Settings"),
            ("⌘M", "Model menu"),
            ("⌘⇧M", "Thinking menu"),
            ("⌘B", "Fold the session list, then the sidebar"),
            ("⌘J", "Side pane"),
            ("⌘`", "Terminal"),
            ("⌘F", "Find in conversation"),
            ("⌘L", "Focus composer"),
            ("⌘.", "Stop the run (Esc in an empty composer)"),
            ("⌘⇧T", "Session tree"),
            ("⌘⇧S", "Status panel"),
            ("Enter", "Send · steer while running"),
            ("Alt+Enter", "Queue a follow-up"),
            ("Shift+Enter", "New line"),
            ("Ctrl+Alt+Shift+F", "Frame-time overlay"),
        ];
        let rows: Vec<AnyElement> = app
            .iter()
            .map(|(k, d)| row(t, *d, None, false, kbd(t, *k).into_any_element()))
            .collect();
        let ext: Vec<AnyElement> = self
            .shortcuts
            .iter()
            .map(|s| {
                row(
                    t,
                    SharedString::from(s.description.clone()),
                    Some(SharedString::from(s.id.clone())),
                    false,
                    kbd(t, crate::components::sidebar::pi_keys(&s.keys)).into_any_element(),
                )
            })
            .collect();
        let n = ext.len();
        div()
            .flex()
            .flex_col()
            .child(Self::head(t, &SettingsSection::Shortcuts, true))
            .child(group(t, rows))
            .child(crate::components::section_label::section_row(
                t,
                "Extensions",
                false,
                Some(crate::components::section_label::count(t, n)),
            ))
            .child(if ext.is_empty() {
                div()
                    .text_role(t, TextRole::Caption)
                    .text_color(t.fg_subtlest())
                    .child(if self.backend.is_some() {
                        "No extension shortcuts in this session."
                    } else {
                        "Open a session to list extension shortcuts."
                    })
                    .into_any_element()
            } else {
                group(t, ext).into_any_element()
            })
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
                    .pb(px(space::S2_5))
                    .child(with_icon(t, IconName::Blocks, title, true))
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
                        el.child(crate::components::section_label::section_row(
                            t,
                            SharedString::from(title),
                            false,
                            None,
                        ))
                    })
                    .child(group(t, rows))
            }))
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let nav = self.nav(&t, cx);
        let body = match self.section.clone() {
            SettingsSection::General => self.general(&t, window, cx),
            SettingsSection::Appearance => self.appearance(&t, cx),
            SettingsSection::Unicrab => self.unicrab(&t, cx),
            SettingsSection::Models => self.models_page(&t, cx),
            SettingsSection::Shortcuts => self.shortcuts_page(&t),
            SettingsSection::Extension(ns) => self.hub(&ns, &t, window, cx),
            SettingsSection::Sessions => self.sessions_page(&t, window, cx),
            SettingsSection::Permissions => match self.permissions_ns() {
                Some(ns) => self.hub(&ns, &t, window, cx),
                None => div()
                    .flex()
                    .flex_col()
                    .child(Self::head(&t, &SettingsSection::Permissions, true))
                    .child(empty_state(
                        &t,
                        IconName::Shield,
                        if self.backend.is_some() {
                            "Workflow is off"
                        } else {
                            "Open a session"
                        },
                        Some(if self.backend.is_some() {
                            "Turn on the unipi.workflow extension to set permission modes and rules."
                                .into()
                        } else {
                            "Permissions are read from the session's extensions.".into()
                        }),
                        Vec::new(),
                    ))
                    .into_any_element(),
            },
            SettingsSection::About => self.about(&t, cx),
        };
        let header = PageHeader::new("Settings")
            .tool(
                div()
                    .w(px(size::SIDEBAR))
                    .child(Input::new(&self.search).prefix(crate::design::icons::icon(
                        IconName::Search,
                        size::ICON,
                        t.fg_subtlest(),
                    ))),
            )
            .window_controls(window, &t)
            .render(&t);
        div()
            .id("settings-page")
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(header)
            .child(
                div().flex().flex_row().flex_1().min_h_0().child(nav).child(
                    div()
                        .id("settings-content")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_y_scroll()
                        .px(px(size::PAGE_GUTTER + space::S2))
                        .pb(px(space::S8))
                        .child(
                            div()
                                .max_w(px(size::SETTINGS_CONTENT))
                                .flex()
                                .flex_col()
                                .child(body),
                        ),
                ),
            )
    }
}
