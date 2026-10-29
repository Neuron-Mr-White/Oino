//! Model picker modal (round5 #1): a real modal — not a popover — shaped like unipi's
//! `ModelPickerSheet` and opened from the composer model chip (click / Ctrl+M) and the
//! header pill's global action. Search always visible on desktop, filtering over the model
//! `name` **and** the full `provider/id`; 54 px rows with name + mono `provider/id` sub;
//! picked state (14 % accent bg + 3 px accent bar); sticky footer Cancel / Apply.
//! **Pins** (Devin/Zed pattern): every row carries a pin toggle; pinned models form the
//! first section and persist in `desktop.json` `favorite_models`.
//!
//! **Fusion tab:** Lead and Sidekick lists contain *every* model (`unipi` only lists its
//! curated preset) plus per-side effort selects. Apply drives the exact `/fusion` dialog
//! path the extension owns: the pane submits `/fusion` and auto-answers the dialog
//! sequence ([`FusionScript`]), so the extension's own apply runs — pair runtime, status
//! topic, and optionally the startup default (`control.set-default-model`, R4).

use std::collections::VecDeque;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use oino_ai::{JsonValue, Model};

use crate::automation::{Automatable as _, Meta, Role};
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::controls;
use crate::components::dialog;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::design::composer as geom;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::dialog::{Dialog, DialogKind};
use crate::model::ui_state::UiEffect;
use crate::views::session_pane::{PaneEvent, SessionPane};
use oino_core::ui::UiResponse;

/// The fusion status topic: `{leadKey, sidekickKey, leadEffort, sideEffort, …}`.
const FUSION_TOPIC: &str = "unipi:fusion:status";

/// Row height of the single tab's model rows (`unipi` `.model-row`).
const ROW_H: f32 = 54.0;
/// Row height of the fusion columns' model rows (`unipi` `.cc-fusion-list`).
const FUSION_ROW_H: f32 = 40.0;

/// Effort levels of the fusion pair (`unipi` `EFFORT_LEVELS`, `preset.rs`).
pub const EFFORTS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Effort label as the fusion dialogs spell them (`unipi` `preset::effort_label`:
/// `"off"` → `None`, `"xhigh"` → `XHigh`, the rest capitalised).
#[must_use]
pub fn effort_label(effort: &str) -> String {
    match effort {
        "off" => "None".to_owned(),
        "xhigh" => "XHigh".to_owned(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|f| f.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

/// One picker tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickerTab {
    /// Pick the session's single model.
    #[default]
    Single,
    /// Pick a fusion lead + sidekick pair.
    Fusion,
}

/// Picker state, kept on the pane ([`crate::views::session_pane::PaneMenu::Model`] open
/// state lives in `pane.menu`).
#[derive(Default)]
pub struct ModelPicker {
    /// Active tab.
    pub tab: PickerTab,
    /// Search query input (created when the picker first opens).
    pub search: Option<Entity<InputState>>,
    /// Picked single model (`provider/id`), applied by the footer.
    pub picked: Option<String>,
    /// Fusion lead (`provider/id`).
    pub lead: Option<String>,
    /// Fusion sidekick (`provider/id`).
    pub sidekick: Option<String>,
    /// Lead effort.
    pub lead_effort: String,
    /// Sidekick effort.
    pub sidekick_effort: String,
    /// Persist the pair as the startup default (`control.set-default-model`, R4).
    pub use_startup: bool,
    /// The `/fusion` dialog auto-answer script, while an Apply runs.
    pub script: Option<FusionScript>,
    /// Whether the picker was open in the previous render (drives the open-time reset).
    pub(crate) was_open: bool,
}

impl ModelPicker {
    /// Resets the picks to the session's current selection.
    fn reset(
        &mut self,
        current: Option<&Model>,
        fusion: Option<&JsonValue>,
        default_effort: &str,
        window: &mut Window,
        cx: &mut Context<SessionPane>,
    ) {
        self.tab = PickerTab::Single;
        self.picked = current.map(model_key);
        self.lead = None;
        self.sidekick = None;
        default_effort.clone_into(&mut self.lead_effort);
        "medium".clone_into(&mut self.sidekick_effort);
        self.use_startup = false;
        if let Some(fusion) =
            fusion.filter(|f| f.get("leadKey").and_then(JsonValue::as_str).is_some())
        {
            self.tab = PickerTab::Fusion;
            self.lead = str_field(fusion, "leadKey");
            self.sidekick = str_field(fusion, "sidekickKey");
            self.lead_effort =
                str_field(fusion, "leadEffort").unwrap_or_else(|| default_effort.to_owned());
            self.sidekick_effort =
                str_field(fusion, "sidekickEffort").unwrap_or_else(|| "medium".to_owned());
        }
        if let Some(search) = &self.search {
            search.update(cx, |s, cx| s.set_value("", window, cx));
        }
    }
}

/// A string field of a JSON object.
fn str_field(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

/// One auto-answered step of the `/fusion` dialog sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ScriptStep {
    /// The `/fusion — type to search` entry dialog: option 0 is always the Fusion row.
    FusionEntry,
    /// A "Fusion — lead" / "Fusion — sidekick" pick: the option label contains `(key)`.
    PickSide { lead: bool, key: String },
    /// An effort dialog ("Lead effort" / "Sidekick effort"): the option starts with the label.
    Effort { effort: String },
    /// The final Confirm dialog.
    Confirm { startup: bool },
}

/// The auto-answer script driving the `/fusion` dialog sequence (Apply → the extension's
/// own pair-set path runs; the GUI never reimplements it).
#[derive(Debug)]
pub struct FusionScript {
    steps: VecDeque<ScriptStep>,
}

impl FusionScript {
    /// The script for a fusion Apply.
    fn new(
        lead: &str,
        sidekick: &str,
        lead_effort: &str,
        sidekick_effort: &str,
        startup: bool,
    ) -> Self {
        Self {
            steps: VecDeque::from(vec![
                ScriptStep::FusionEntry,
                ScriptStep::PickSide {
                    lead: true,
                    key: lead.to_owned(),
                },
                ScriptStep::PickSide {
                    lead: false,
                    key: sidekick.to_owned(),
                },
                ScriptStep::Effort {
                    effort: lead_effort.to_owned(),
                },
                ScriptStep::Effort {
                    effort: sidekick_effort.to_owned(),
                },
                ScriptStep::Confirm { startup },
            ]),
        }
    }

    /// Whether `dialog` is the dialog `step` expects.
    fn matches(step: &ScriptStep, dialog: &Dialog) -> bool {
        match step {
            ScriptStep::FusionEntry => dialog.title.contains("/fusion"),
            ScriptStep::PickSide { lead, .. } => {
                let needle = if *lead {
                    "Fusion — lead"
                } else {
                    "Fusion — sidekick"
                };
                dialog.title.contains(needle)
            }
            ScriptStep::Effort { .. } => dialog.title.to_lowercase().contains("effort"),
            ScriptStep::Confirm { .. } => dialog.title == "Confirm",
        }
    }

    /// The answer for `dialog`, consuming the step it matches; `None` when the dialog is
    /// not the expected next one (the caller then cancels the script).
    fn answer(&mut self, dialog: &Dialog) -> Option<UiResponse> {
        let step = self.steps.front()?.clone();
        if !Self::matches(&step, dialog) {
            return None;
        }
        self.steps.pop_front();
        let DialogKind::Select { options } = &dialog.kind else {
            return None;
        };
        let label = match &step {
            ScriptStep::FusionEntry => options.first().map(|o| o.label.clone()),
            ScriptStep::PickSide { key, .. } => {
                let needle = format!("({key})").to_lowercase();
                options
                    .iter()
                    .find(|o| o.label.to_lowercase().contains(&needle))
                    .map(|o| o.label.clone())
            }
            ScriptStep::Effort { effort } => {
                let wanted = effort_label(effort).to_lowercase();
                options
                    .iter()
                    .find(|o| {
                        let label = o.label.to_lowercase();
                        label == wanted || label.starts_with(&format!("{wanted} ("))
                    })
                    .map(|o| o.label.clone())
            }
            ScriptStep::Confirm { startup } => options
                .iter()
                .find(|o| {
                    o.label
                        == if *startup {
                            "Confirm and use at startup"
                        } else {
                            "Confirm"
                        }
                })
                .map(|o| o.label.clone()),
        }?;
        Some(dialog.text(label))
    }
}

/// `provider/id` of one model.
#[must_use]
pub fn model_key(m: &Model) -> String {
    format!("{}/{}", m.provider, m.id)
}

/// `provider/id` → `(provider, id)`.
#[must_use]
pub fn split_key(key: &str) -> Option<(String, String)> {
    let (provider, id) = key.split_once('/')?;
    (!provider.is_empty() && !id.is_empty()).then(|| (provider.to_owned(), id.to_owned()))
}

impl SessionPane {
    /// The picker's auto-answer hook: called for every dialog reaching the front of the
    /// queue while a fusion Apply runs. Returns the response to send instead of showing
    /// the dialog.
    pub(crate) fn answer_fusion_dialog(
        &mut self,
        dialog: &Dialog,
        cx: &mut Context<Self>,
    ) -> Option<UiResponse> {
        let answer = self
            .picker
            .script
            .as_mut()
            .and_then(|script| script.answer(dialog));
        if answer.is_none() && self.picker.script.is_some() {
            // An unexpected dialog: stop driving and show it to the user.
            self.picker.script = None;
            cx.emit(PaneEvent::Toast(
                "The fusion picker asked something new — answer it yourself".into(),
            ));
            cx.notify();
            return None;
        }
        if self
            .picker
            .script
            .as_ref()
            .is_some_and(|s| s.steps.is_empty())
        {
            self.picker.script = None;
        }
        answer
    }

    /// Applies the picker's selection (the modal's Apply button).
    pub(crate) fn apply_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.picker.tab {
            PickerTab::Single => {
                let Some((provider, id)) = self.picker.picked.as_deref().and_then(split_key) else {
                    return;
                };
                self.menu = None;
                self.set_model(provider.clone(), id.clone(), cx);
            }
            PickerTab::Fusion => {
                let (Some(lead), Some(sidekick)) =
                    (self.picker.lead.clone(), self.picker.sidekick.clone())
                else {
                    cx.emit(PaneEvent::Toast("Pick a lead and a sidekick first".into()));
                    return;
                };
                self.picker.script = Some(FusionScript::new(
                    &lead,
                    &sidekick,
                    &self.picker.lead_effort,
                    &self.picker.sidekick_effort,
                    self.picker.use_startup,
                ));
                self.menu = None;
                self.run_prompt("/fusion", cx);
            }
        }
        dialog::forget_owned_focus("model-picker", cx);
        self.restore_focus(window, cx);
    }

    /// Toggles the pin of `key` (`desktop.json` `favorite_models`).
    pub(crate) fn toggle_model_pin(&mut self, key: &str, cx: &mut Context<Self>) {
        let pinned = self.settings.favorite_models.iter().any(|k| k == key);
        self.settings.favorite_models.retain(|k| k != key);
        if !pinned {
            self.settings.favorite_models.insert(0, key.to_owned());
        }
        cx.emit(PaneEvent::Effect(UiEffect::PinModel {
            key: key.to_owned(),
            pinned: !pinned,
        }));
        cx.notify();
    }
}

/// The "Now" row: what the session runs today (`unipi` `.mp-current`).
fn now_row(pane: &SessionPane, t: &OinoTheme) -> AnyElement {
    let fusion = pane
        .model
        .topic(FUSION_TOPIC)
        .filter(|f| f.get("leadKey").and_then(JsonValue::as_str).is_some());
    let label = match fusion {
        Some(f) => format!(
            "{} ◆ {}",
            f.get("leadName")
                .and_then(JsonValue::as_str)
                .unwrap_or("Lead"),
            f.get("sidekickName")
                .and_then(JsonValue::as_str)
                .unwrap_or("Sidekick"),
        ),
        None => pane.model.model_label(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(space::S2))
        .pb(px(space::S2))
        .child(
            div()
                .text_role(t, TextRole::Caption)
                .text_color(t.fg_subtlest())
                .child("Now"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .h(px(geom::NOW_PILL))
                .px(px(space::S2))
                .rounded(px(radius::R_FULL))
                .bg(t.bubble())
                .text_role(t, TextRole::Xs)
                .weight_medium()
                .text_color(t.fg_subtle())
                .child(SharedString::from(label.to_uppercase())),
        )
        .into_any_element()
}

/// One model row (name + mono `provider/id`, pin, picked state).
#[allow(clippy::too_many_arguments, reason = "plain row parameters")]
fn model_row(
    id: &str,
    m: &Model,
    row_h: f32,
    pinned: bool,
    picked: bool,
    show_pin: bool,
    pick: Rc<dyn Fn(&str, &mut App)>,
    t: &OinoTheme,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let key = model_key(m);
    let name = if m.name.is_empty() {
        m.id.clone()
    } else {
        m.name.clone()
    };
    let pin_id = ElementId::Name(format!("{id}-pin").into());
    let pin_entity = cx.entity().downgrade();
    let pin_key = key.clone();
    let pick = Rc::clone(&pick);
    let pick_key = key.clone();
    let name_label = name.clone();
    let key_label = key.clone();
    div()
        .id(ElementId::Name(id.into()))
        .flex()
        .items_center()
        .gap(px(space::S2_5))
        .h(px(row_h))
        .px(px(space::S3))
        .rounded(px(radius::R_ROW))
        .when(picked, |el| {
            el.bg(t.primary().opacity(0.14))
                .border_l_3()
                .border_color(t.primary())
        })
        .when(!picked, |el| el.hover(|s| s.bg(t.bubble())))
        .cursor_pointer()
        .when(show_pin, |el| {
            el.child(
                Btn::icon_only(
                    pin_id,
                    IconName::Pin,
                    if pinned { "Unpin" } else { "Pin to top" },
                )
                .size(BtnSize::Xs)
                .variant(if pinned {
                    Variant::Secondary
                } else {
                    Variant::Ghost
                })
                .on_click(move |_, _, cx: &mut App| {
                    let _ = pin_entity.update(cx, |p, cx| p.toggle_model_pin(&pin_key, cx));
                }),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(space::HALF))
                .child(
                    div()
                        .truncate()
                        .text_role(t, TextRole::Base)
                        .weight_medium()
                        .child(SharedString::from(name)),
                )
                .child(
                    div()
                        .truncate()
                        .mono_text(t, TextRole::Xs)
                        .text_color(t.fg_subtlest())
                        .child(SharedString::from(key)),
                ),
        )
        .child(
            div()
                .w(px(size::ICON))
                .flex()
                .justify_center()
                .when(picked, |el| {
                    el.child(icon(IconName::Check, size::ICON, t.primary()))
                }),
        )
        .on_click(move |_: &ClickEvent, _, cx: &mut App| pick(&pick_key, cx))
        // Read-back for scenarios: the row's name and full `provider/id`.
        .automation(
            Meta::labelled(Role::Row, format!("{name_label} {key_label}"))
                .selected(picked)
                .checked(pinned),
        )
        .into_any_element()
}

/// Whether `m` matches the (lowercased) `query` by name or full `provider/id`.
fn matches(m: &Model, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let key = model_key(m).to_lowercase();
    let name = if m.name.is_empty() {
        m.id.clone()
    } else {
        m.name.clone()
    }
    .to_lowercase();
    key.contains(query) || name.contains(query)
}

/// A small section label inside a list.
fn section(t: &OinoTheme, text: &str) -> AnyElement {
    div()
        .px(px(space::S3))
        .pt(px(space::S2))
        .pb(px(space::S1))
        .child(crate::components::section_label::label(
            t,
            SharedString::from(text),
        ))
        .into_any_element()
}

/// The single tab: search, the pinned section, every model.
fn single_body(
    pane: &SessionPane,
    search: &Entity<InputState>,
    t: &OinoTheme,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let query = search.read(cx).value().to_lowercase();
    let favorites = pane.settings.favorite_models.clone();
    let picked = pane.picker.picked.clone();
    let pick = {
        let entity = cx.entity().downgrade();
        Rc::new(move |key: &str, cx: &mut App| {
            let _ = entity.update(cx, |p, cx| {
                p.picker.picked = Some(key.to_owned());
                cx.notify();
            });
        }) as Rc<dyn Fn(&str, &mut App)>
    };
    let pinned_matches: Vec<&Model> = pane
        .models
        .iter()
        .filter(|m| favorites.contains(&model_key(m)) && matches(m, &query))
        .collect();
    let other_matches: Vec<&Model> = pane
        .models
        .iter()
        .filter(|m| !favorites.contains(&model_key(m)) && matches(m, &query))
        .collect();
    let has_pinned = !pinned_matches.is_empty();
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut n = 0usize;
    if has_pinned {
        rows.push(section(t, "Pinned"));
        for m in pinned_matches {
            let key = model_key(m);
            rows.push(model_row(
                &format!("picker-row-{n}"),
                m,
                ROW_H,
                true,
                picked.as_deref() == Some(key.as_str()),
                true,
                Rc::clone(&pick),
                t,
                cx,
            ));
            n += 1;
        }
    }
    rows.push(section(
        t,
        if query.is_empty() {
            "All models"
        } else {
            "Matches"
        },
    ));
    if !has_pinned && other_matches.is_empty() {
        rows.push(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(ROW_H))
                .text_role(t, TextRole::Sm)
                .text_color(t.fg_subtlest())
                .child("No models match")
                .into_any_element(),
        );
    }
    for m in other_matches {
        let key = model_key(m);
        rows.push(model_row(
            &format!("picker-row-{n}"),
            m,
            ROW_H,
            false,
            picked.as_deref() == Some(key.as_str()),
            true,
            Rc::clone(&pick),
            t,
            cx,
        ));
        n += 1;
    }
    div()
        .flex()
        .flex_col()
        .gap(px(space::S2))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(geom::SEARCH_H))
                .px(px(space::S3))
                .rounded(px(geom::PANEL_R))
                .bg(t.bubble())
                .child(Input::new(search))
                // The search field registers for the automation driver.
                .automation(Meta::labelled(Role::Input, "Search models")),
        )
        .child(
            div()
                .id("picker-list")
                .flex()
                .flex_col()
                .rounded(px(radius::R_CARD))
                .bg(t.bubble())
                .max_h(px(geom::LIST_MAX))
                .overflow_y_scroll()
                .children(rows),
        )
        .into_any_element()
}

/// The effort select of one fusion side (a small button opening the effort menu).
fn effort_select(
    pane: &SessionPane,
    side: &str,
    label: &str,
    current: &str,
    t: &OinoTheme,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let id = format!("picker-{side}-effort");
    let set = {
        let entity = cx.entity().downgrade();
        let side = side.to_owned();
        Rc::new(move |effort: String, cx: &mut App| {
            let _ = entity.update(cx, |p, cx| {
                if side == "lead" {
                    p.picker.lead_effort = effort;
                } else {
                    p.picker.sidekick_effort = effort;
                }
                cx.notify();
            });
        })
    };
    let entries: Vec<MenuEntry> = EFFORTS
        .iter()
        .map(|effort| {
            let set = Rc::clone(&set);
            let owned = (*effort).to_owned();
            let checked = effort == &current;
            MenuEntry::item(effort_label(effort), move |_, cx| set(owned.clone(), cx))
                .checked(checked)
        })
        .collect();
    let close = {
        let this = cx.entity().downgrade();
        move |_: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |p, cx| {
                p.picker_effort_menu = None;
                cx.notify();
            });
        }
    };
    let open_with = id.clone();
    popover(
        Btn::new(ElementId::Name(id.clone().into()))
            .label(SharedString::from(effort_label(current)))
            .icon(IconName::SlidersHorizontal)
            .trailing(IconName::ChevronDown)
            .size(BtnSize::Sm)
            .variant(Variant::Secondary)
            .tooltip(format!("{label} effort"), None)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.picker_effort_menu =
                    if this.picker_effort_menu.as_deref() == Some(open_with.as_str()) {
                        None
                    } else {
                        Some(open_with.clone())
                    };
                cx.notify();
            })),
        pane.picker_effort_menu.as_deref() == Some(id.as_str()),
        Placement::AboveEnd,
        || menu("picker-effort-menu", t, size::MENU_MIN, entries).into_any_element(),
        close,
    )
    .into_any_element()
}

/// One fusion column: label, every-model list, effort select.
fn fusion_side(
    pane: &SessionPane,
    side: &str,
    label: &str,
    selected: Option<&str>,
    t: &OinoTheme,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let pick = {
        let entity = cx.entity().downgrade();
        let side = side.to_owned();
        Rc::new(move |key: &str, cx: &mut App| {
            let _ = entity.update(cx, |p, cx| {
                if side == "lead" {
                    p.picker.lead = Some(key.to_owned());
                } else {
                    p.picker.sidekick = Some(key.to_owned());
                }
                cx.notify();
            });
        }) as Rc<dyn Fn(&str, &mut App)>
    };
    let rows: Vec<AnyElement> = pane
        .models
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let id = format!("picker-{side}-{i}");
            let picked = selected == Some(model_key(m).as_str());
            model_row(
                &id,
                m,
                FUSION_ROW_H,
                false,
                picked,
                false,
                Rc::clone(&pick),
                t,
                cx,
            )
        })
        .collect();
    let current = if side == "lead" {
        pane.picker.lead_effort.clone()
    } else {
        pane.picker.sidekick_effort.clone()
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(space::S1_5))
        .child(
            div()
                .text_role(t, TextRole::Caption)
                .weight_medium()
                .text_color(t.fg_subtle())
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .id(ElementId::Name(format!("picker-{side}-list").into()))
                .flex()
                .flex_col()
                .rounded(px(radius::R_CARD))
                .bg(t.bubble())
                .max_h(px(geom::FUSION_LIST_MAX))
                .overflow_y_scroll()
                .children(rows),
        )
        .child(effort_select(pane, side, label, &current, t, cx))
        .into_any_element()
}

/// The fusion tab: Lead and Sidekick lists of *every* model + per-side efforts.
fn fusion_body(pane: &SessionPane, t: &OinoTheme, cx: &mut Context<SessionPane>) -> AnyElement {
    let lead = pane.picker.lead.clone();
    let sidekick = pane.picker.sidekick.clone();
    let use_startup = pane.picker.use_startup;
    div()
        .flex()
        .flex_col()
        .gap(px(space::S2))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(space::S2_5))
                .child(fusion_side(pane, "lead", "Lead", lead.as_deref(), t, cx))
                .child(fusion_side(
                    pane,
                    "sidekick",
                    "Sidekick",
                    sidekick.as_deref(),
                    t,
                    cx,
                )),
        )
        .child(
            div()
                .id("picker-startup")
                .flex()
                .items_center()
                .gap(px(space::S2))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.picker.use_startup = !this.picker.use_startup;
                    cx.notify();
                }))
                .child(controls::check(t, use_startup, false))
                .child(
                    div()
                        .text_role(t, TextRole::Sm)
                        .text_color(t.fg_subtle())
                        .child("Use at startup (default model)"),
                )
                // Read-back for scenarios.
                .automation(Meta::labelled(Role::Checkbox, "Use at startup").checked(use_startup)),
        )
        .child(
            div()
                .text_role(t, TextRole::Xs)
                .text_color(t.fg_subtlest())
                .child("The lead plans and reviews; a persistent sidekick does the hands-on work."),
        )
        .into_any_element()
}

/// The picker modal, mounted deferred at window level by the composer.
pub(crate) fn modal(
    pane: &mut SessionPane,
    t: &OinoTheme,
    window: &mut Window,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    // Reset the picks when the picker (re)opens; create the search input lazily (the
    // board search follows the same get-or-insert pattern).
    let just_opened = !pane.picker.was_open;
    pane.picker.was_open = true;
    if just_opened {
        let current = pane.model.model.clone();
        let fusion = pane.model.topic(FUSION_TOPIC).cloned();
        let default_effort = controls::level_label(pane.model.thinking).to_lowercase();
        pane.picker.reset(
            current.as_ref(),
            fusion.as_ref(),
            &default_effort,
            window,
            cx,
        );
    }
    let search = pane
        .picker
        .search
        .get_or_insert_with(|| {
            let state = cx.new(|cx| InputState::new(window, cx).placeholder("Search all models"));
            cx.subscribe(&state, |_, _, _: &InputEvent, cx| cx.notify())
                .detach();
            state
        })
        .clone();

    let focus = dialog::owned_focus("model-picker", window, cx);
    let restore_entity = cx.entity().downgrade();
    let restore = Rc::new(move |w: &mut Window, cx: &mut App| {
        let _ = restore_entity.update(cx, |p, cx| {
            dialog::forget_owned_focus("model-picker", cx);
            p.restore_focus(w, cx);
        });
    });

    let tab_entity = cx.entity().downgrade();
    let tabs = crate::components::chip::segmented(
        t,
        "picker-tabs",
        &["Single", "Fusion"],
        usize::from(pane.picker.tab == PickerTab::Fusion),
        Rc::new(move |i, _, cx| {
            let _ = tab_entity.update(cx, |p, cx| {
                p.picker.tab = if i == 1 {
                    PickerTab::Fusion
                } else {
                    PickerTab::Single
                };
                cx.notify();
            });
        }),
    );
    let body = div()
        .id("model-picker")
        .flex()
        .flex_col()
        .gap(px(space::S2_5))
        .child(now_row(pane, t))
        .child(tabs)
        .child(match pane.picker.tab {
            PickerTab::Single => single_body(pane, &search, t, cx),
            PickerTab::Fusion => fusion_body(pane, t, cx),
        })
        // Read-back for scenarios: the modal itself.
        .automation(Meta::labelled(Role::Card, "Model picker"));
    let apply_label = if pane.picker.tab == PickerTab::Fusion {
        "Use this pair"
    } else {
        "Apply"
    };
    let actions = vec![
        Btn::new("picker-cancel")
            .label("Cancel")
            .size(BtnSize::Md)
            .variant(Variant::Secondary)
            .on_click(cx.listener(|this, _, w, cx| {
                this.menu = None;
                dialog::forget_owned_focus("model-picker", cx);
                this.restore_focus(w, cx);
            }))
            .into_any_element(),
        Btn::new("picker-apply")
            .label(apply_label)
            .size(BtnSize::Md)
            .variant(Variant::Primary)
            .on_click(cx.listener(|this, _, w, cx| this.apply_model_picker(w, cx)))
            .into_any_element(),
    ];
    dialog::modal_owned(
        t,
        "model-picker",
        SharedString::from("Model"),
        size::DIALOG_MD,
        body.into_any_element(),
        actions,
        &focus,
        restore,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dialog::SelectOption;
    use oino_core::ui::UiAnswer;

    fn select_dialog(title: &str, options: &[&str]) -> Dialog {
        Dialog {
            id: "d1".into(),
            method: "select".into(),
            title: title.into(),
            extension_id: None,
            timeout_ms: None,
            kind: DialogKind::Select {
                options: options
                    .iter()
                    .map(|o| SelectOption {
                        label: (*o).to_owned(),
                        description: None,
                    })
                    .collect(),
            },
        }
    }

    /// The string value a `select` answer carries.
    fn answered(response: &UiResponse) -> String {
        match &response.answer {
            UiAnswer::Value(JsonValue::String(label)) => label.clone(),
            other => panic!("not a text answer: {other:?}"),
        }
    }

    #[test]
    fn efforts_label_like_the_dialogs() {
        assert_eq!(effort_label("medium"), "Medium");
        assert_eq!(effort_label("xhigh"), "XHigh");
        assert_eq!(effort_label(""), "");
    }

    #[test]
    fn keys_split_on_the_first_slash() {
        assert_eq!(
            split_key("anthropic/claude-sonnet-4-6"),
            Some(("anthropic".into(), "claude-sonnet-4-6".into()))
        );
        assert_eq!(split_key("oino/"), None);
        assert_eq!(split_key("/id"), None);
        assert_eq!(split_key("naked"), None);
    }

    #[test]
    fn fusion_script_answers_the_dialog_sequence() {
        let mut script = FusionScript::new(
            "anthropic/claude-sonnet-4-6",
            "oino/oino-fast",
            "high",
            "medium",
            true,
        );
        // 1: the entry dialog — option 0 is the Fusion row.
        let d = select_dialog(
            "/fusion — type to search",
            &["Fusion · pick a lead and a sidekick…", "Oino Large · High"],
        );
        assert_eq!(
            answered(&script.answer(&d).expect("entry answered")),
            "Fusion · pick a lead and a sidekick…"
        );
        // 2: lead.
        let d = select_dialog(
            "Fusion — lead",
            &[
                "Claude Sonnet 4.6 (anthropic/claude-sonnet-4-6)",
                "GPT-5.2 (openai/gpt-5.2)",
            ],
        );
        assert_eq!(
            answered(&script.answer(&d).expect("lead answered")),
            "Claude Sonnet 4.6 (anthropic/claude-sonnet-4-6)"
        );
        // 3: sidekick.
        let d = select_dialog(
            "Fusion — sidekick",
            &["Oino Fast (oino/oino-fast)", "GPT-5.2 (openai/gpt-5.2)"],
        );
        assert_eq!(
            answered(&script.answer(&d).expect("sidekick answered")),
            "Oino Fast (oino/oino-fast)"
        );
        // 4: lead effort ("(current)" and the "off" → "None" spelling must not break it).
        let d = select_dialog("Lead effort", &["None (current)", "High", "Medium"]);
        assert_eq!(
            answered(&script.answer(&d).expect("effort answered")),
            "High"
        );
        // 5: sidekick effort.
        let d = select_dialog("Sidekick effort", &["Medium (current)", "High"]);
        assert_eq!(
            answered(&script.answer(&d).expect("effort answered")),
            "Medium (current)"
        );
        // 6: confirm with startup.
        let d = select_dialog("Confirm", &["Confirm", "Confirm and use at startup"]);
        assert_eq!(
            answered(&script.answer(&d).expect("confirm answered")),
            "Confirm and use at startup"
        );
        assert!(script.steps.is_empty());
    }

    #[test]
    fn fusion_script_stops_on_unexpected_dialogs() {
        let mut script = FusionScript::new("a/b", "c/d", "low", "low", false);
        let d = select_dialog("Approve file edit", &["Yes", "No"]);
        assert!(script.answer(&d).is_none());
        // The next expected dialog is still answerable.
        let d = select_dialog("/fusion — type to search", &["Fusion · pick"]);
        assert!(script.answer(&d).is_some());
    }
}
