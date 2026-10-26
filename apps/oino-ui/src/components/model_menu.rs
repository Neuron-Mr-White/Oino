//! `ModelMenu` (design.md §4.26): current provider first (group label + auth badge), then
//! other providers, capability badges, check mark, "Manage models…".

use gpui_kit::assets::IconName;
use gpui_kit::{AnyElement, Context, IntoElement, SharedString};

use crate::components::menu::{MenuEntry, menu};
use crate::design::size;
use crate::design::theme::OinoTheme;
use crate::views::session_pane::{PaneEvent, SessionPane};

fn provider_title(provider: &str) -> String {
    match provider {
        "oino" => "Oino Router".to_owned(),
        "anthropic" => "Anthropic".to_owned(),
        "openai" => "OpenAI".to_owned(),
        "openai-compat" => "OpenAI-compatible".to_owned(),
        "google" => "Google".to_owned(),
        other => other.to_owned(),
    }
}

/// The menu body for `pane`.
pub fn model_menu(pane: &SessionPane, t: &OinoTheme, cx: &mut Context<SessionPane>) -> AnyElement {
    let current = pane.model.model.clone();
    let current_provider = current.as_ref().map(|m| m.provider.clone());
    let mut providers: Vec<String> = Vec::new();
    for m in &pane.models {
        if !providers.contains(&m.provider) {
            providers.push(m.provider.clone());
        }
    }
    if let Some(cp) = &current_provider
        && let Some(i) = providers.iter().position(|p| p == cp)
    {
        let p = providers.remove(i);
        providers.insert(0, p);
    }
    let entity = cx.entity().downgrade();
    let mut entries = Vec::new();
    for (pi, provider) in providers.iter().enumerate() {
        if pi > 0 {
            entries.push(MenuEntry::Separator);
        }
        entries.push(MenuEntry::Label(
            SharedString::from(provider_title(provider)),
            Some("API".into()),
        ));
        for m in pane.models.iter().filter(|m| &m.provider == provider) {
            let e = entity.clone();
            let (prov, id) = (m.provider.clone(), m.id.clone());
            let checked = current
                .as_ref()
                .is_some_and(|c| c.id == m.id && c.provider == m.provider);
            let mut badges = Vec::new();
            if m.input
                .iter()
                .any(|i| format!("{i:?}").to_lowercase().contains("image"))
            {
                badges.push("Vision".to_owned());
            }
            if m.context_window >= 1_000_000 {
                badges.push("1M".to_owned());
            } else {
                badges.push(format!("{}k", m.context_window / 1000));
            }
            let label = if m.name.is_empty() {
                m.id.clone()
            } else {
                m.name.clone()
            };
            entries.push(
                MenuEntry::item(label, move |_, cx| {
                    let _ = e.update(cx, |p, cx| p.set_model(prov.clone(), id.clone(), cx));
                })
                .keys(SharedString::from(badges.join(" · ")))
                .checked(checked),
            );
        }
    }
    if entries.is_empty() {
        entries.push(MenuEntry::Label("No models with credentials".into(), None));
    }
    entries.push(MenuEntry::Separator);
    let e = entity;
    entries.push(
        MenuEntry::item("Manage models…", move |_, cx| {
            let _ = e.update(cx, |p, cx| {
                p.menu = None;
                cx.emit(PaneEvent::Effect(
                    crate::model::ui_state::UiEffect::OpenSettings {
                        namespace: Some("models".into()),
                    },
                ));
            });
        })
        .icon(IconName::SlidersHorizontal),
    );
    menu("model-menu", t, size::MODEL_MENU, entries)
        .max_h(size::DIALOG_MAX_H * 0.7)
        .into_any_element()
}
