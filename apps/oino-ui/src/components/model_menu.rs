//! `ModelMenu` (design.md §4.26, `ZCode` `chat-model-select-menu`): the current provider is
//! pinned at the top and rendered directly (group label + its models, check on the selected
//! one), every other provider is a submenu row that opens that provider's model list with a
//! "All providers" row to come back. A single provider skips the provider level entirely
//! (`ZCode`: "只有一个 provider 时，一级菜单直接展示 model 列表"). Capability badges stay on the
//! model rows, "Manage models…" is the footer.
//!
//! The submenu is a level inside the same menu (drill-down) instead of a side flyout: GPUI's
//! menu shell is one focus-taking popover, and a nested panel anchored to the right of it
//! would be clipped by the scroll container on short windows. Keys: → or Enter opens a
//! provider, ← or Esc goes back one level, Esc at the top level closes the menu.

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

/// Capability / context badges of one model (`Vision`, `1M`, `200k`).
fn badges(m: &oino_ai::Model) -> String {
    let mut out = Vec::new();
    if m.input
        .iter()
        .any(|i| format!("{i:?}").to_lowercase().contains("image"))
    {
        out.push("Vision".to_owned());
    }
    if m.context_window >= 1_000_000 {
        out.push("1M".to_owned());
    } else {
        out.push(format!("{}k", m.context_window / 1000));
    }
    out.join(" · ")
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
    let manage = {
        let e = entity.clone();
        MenuEntry::item("Manage models…", move |_, cx| {
            let _ = e.update(cx, |p, cx| {
                p.menu = None;
                p.menu_sub = None;
                cx.emit(PaneEvent::Effect(
                    crate::model::ui_state::UiEffect::OpenSettings {
                        namespace: Some("models".into()),
                    },
                ));
            });
        })
        .icon(IconName::SlidersHorizontal)
    };
    let mut entries = Vec::new();

    // Level 2: one provider's models, opened from the provider row.
    if let Some(open) = pane.menu_sub.clone() {
        let back = entity.clone();
        entries.push(
            MenuEntry::item("All providers", move |_, cx| {
                let _ = back.update(cx, |p, cx| p.open_model_sub(None, cx));
            })
            .icon(IconName::ChevronLeft),
        );
        entries.push(MenuEntry::Separator);
        let count = pane.models.iter().filter(|m| m.provider == open).count();
        entries.push(MenuEntry::Label(
            SharedString::from(provider_title(&open)),
            Some(SharedString::from(format!("{count}"))),
        ));
        push_models(
            pane,
            t,
            &entity,
            current.as_ref(),
            &mut entries,
            Some(&open),
        );
        entries.push(MenuEntry::Separator);
        entries.push(manage);
        return menu("model-menu", t, size::MODEL_MENU, entries)
            .max_h(size::DIALOG_MAX_H * 0.7)
            .into_any_element();
    }

    let single = providers.len() <= 1;
    if single {
        // One provider: no provider level (ZCode). Its models go straight into the menu.
        push_models(pane, t, &entity, current.as_ref(), &mut entries, None);
    } else {
        // The provider in use is pinned: a group label plus its models, checked.
        if let Some(first) = providers.first() {
            let count = pane.models.iter().filter(|m| &m.provider == first).count();
            entries.push(MenuEntry::Label(
                SharedString::from(provider_title(first)),
                Some(SharedString::from(if count == 1 {
                    "1 model".to_owned()
                } else {
                    format!("{count} models")
                })),
            ));
            push_models(
                pane,
                t,
                &entity,
                current.as_ref(),
                &mut entries,
                Some(first),
            );
            entries.push(MenuEntry::Separator);
        }
        for provider in providers.iter().skip(1) {
            let e = entity.clone();
            let p = provider.clone();
            let count = pane
                .models
                .iter()
                .filter(|m| &m.provider == provider)
                .count();
            let has_current = current.as_ref().is_some_and(|c| &c.provider == provider);
            entries.push(
                MenuEntry::item(provider_title(provider), move |_, cx| {
                    let p = p.clone();
                    let _ = e.update(cx, |pane, cx| pane.open_model_sub(Some(p), cx));
                })
                .keys(SharedString::from(format!("{count}")))
                .checked(has_current)
                .submenu(),
            );
        }
    }
    if entries.is_empty() {
        entries.push(MenuEntry::Label("No models with credentials".into(), None));
    }
    entries.push(MenuEntry::Separator);
    entries.push(manage);
    menu("model-menu", t, size::MODEL_MENU, entries)
        .max_h(size::DIALOG_MAX_H * 0.7)
        .into_any_element()
}

/// Appends the model rows of `provider` (all models when `None`).
fn push_models(
    pane: &SessionPane,
    _t: &OinoTheme,
    entity: &gpui_kit::WeakEntity<SessionPane>,
    current: Option<&oino_ai::Model>,
    entries: &mut Vec<MenuEntry>,
    provider: Option<&str>,
) {
    for m in pane
        .models
        .iter()
        .filter(|m| provider.is_none_or(|p| m.provider == p))
    {
        let e = entity.clone();
        let (prov, id) = (m.provider.clone(), m.id.clone());
        let checked = current.is_some_and(|c| c.id == m.id && c.provider == m.provider);
        let label = if m.name.is_empty() {
            m.id.clone()
        } else {
            m.name.clone()
        };
        entries.push(
            MenuEntry::item(label, move |_, cx| {
                let _ = e.update(cx, |p, cx| p.set_model(prov.clone(), id.clone(), cx));
            })
            .keys(SharedString::from(badges(m)))
            .checked(checked),
        );
    }
}
