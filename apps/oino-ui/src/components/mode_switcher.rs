//! Agent-mode switcher (round5 #2, after `ZCode` `V4ComposerModeControls`): a ghost trigger
//! (icon + label + chevron, `h-7`) in the composer's inner footer opening an upward 256 px
//! menu of the agent modes — Regular, Plan, Goal, Graph, Ralph — each with icon, name and a
//! one-line description. It drives the exact commands the modes' own slash commands use
//! (`/plan`, `/goal`, `/graph`, `/ralph`, `/regular`) plus the sticky
//! `unipi:workflow:status` / `unipi:long-horizon:state` topics; nothing new harness-side.
//! The permission chip (Ask/Plan/Auto/Full) is gone from the composer: approvals live in
//! the conversation and the level stays settable via `/permission`.
//!
//! Long-horizon modes arm instead of running: [`AgentMode::arms_prompt`] explains why.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::automation::Automatable as _;
use crate::components::menu::{MenuEntry, Placement, menu, popover};
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};
use crate::model::composer::AgentMode;
use crate::views::session_pane::{PaneMenu, SessionPane, WORKFLOW_TOPIC};

/// The long-horizon state topic (`{mode, owner, paused}`, the `mode` of the switcher).
const LH_STATE_TOPIC: &str = "unipi:long-horizon:state";

/// Lucide icon of one mode.
fn mode_icon(mode: AgentMode) -> IconName {
    match mode {
        AgentMode::Regular => IconName::MessageSquare,
        AgentMode::Plan => IconName::ListChecks,
        AgentMode::Goal => IconName::Zap,
        AgentMode::Graph => IconName::GitFork,
        AgentMode::Ralph => IconName::RotateCw,
    }
}

/// The mode's tint (per-mode colours, `ZCode` `V4ComposerModeControls`): plan gold, armed
/// modes accent, regular subtle.
fn mode_tint(t: &OinoTheme, mode: AgentMode) -> gpui_kit::Hsla {
    match mode {
        AgentMode::Regular => t.fg_subtle(),
        AgentMode::Plan => t.gold(),
        _ => t.accent(),
    }
}

/// The pane's current agent mode: an armed pick first, then the sticky topics.
pub(crate) fn current(pane: &SessionPane) -> AgentMode {
    if let Some(armed) = pane.mode_armed {
        return armed;
    }
    let plan = pane
        .model
        .topic(WORKFLOW_TOPIC)
        .and_then(|d| d.get("planMode"))
        .and_then(oino_ai::JsonValue::as_bool)
        == Some(true);
    let lh = pane
        .model
        .topic(LH_STATE_TOPIC)
        .and_then(|d| d.get("mode"))
        .and_then(oino_ai::JsonValue::as_str);
    AgentMode::from_topics(plan, lh)
}

impl SessionPane {
    /// Picks `mode` from the switcher: runs its commands, arms long-horizon modes.
    pub(crate) fn pick_agent_mode(&mut self, mode: AgentMode, cx: &mut Context<Self>) {
        self.menu = None;
        let commands = mode.commands_from(current(self));
        if mode.arms_prompt() {
            self.mode_armed = Some(mode);
            cx.emit(crate::views::session_pane::PaneEvent::Toast(format!(
                "{} armed — your next prompt starts it",
                mode.label()
            )));
            cx.notify();
            return;
        }
        self.mode_armed = None;
        for command in commands {
            self.run_prompt(&command, cx);
        }
        cx.notify();
    }

    /// Ctrl+Shift+M (composer focused): steps to the next mode.
    pub(crate) fn cycle_agent_mode(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let next = current(self).cycle();
        self.pick_agent_mode(next, cx);
    }
}

/// The switcher trigger + its upward menu.
pub(crate) fn switcher(
    pane: &SessionPane,
    narrow: bool,
    t: &OinoTheme,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let mode = current(pane);
    let tint = mode_tint(t, mode);
    let label_id = format!("agent-mode-label-{}", mode.label().to_lowercase());
    let close = {
        let this = cx.entity().downgrade();
        move |_: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |p, cx| {
                p.menu = None;
                cx.notify();
            });
        }
    };
    popover(
        div()
            .id("agent-mode")
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(px(space::S1))
            .h(px(size::CONTROL))
            .pl(px(space::S2))
            .pr(px(space::S1_5))
            .rounded(px(radius::R_LG))
            .cursor_pointer()
            .text_role(t, TextRole::Sm)
            .weight_medium()
            .text_color(tint)
            .tooltip(crate::components::tooltip::tip(
                "Agent mode",
                Some("ctrl+shift+m".into()),
            ))
            .hover(|s| s.bg(t.bubble()))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(PaneMenu::Mode, cx)))
            .child(icon(mode_icon(mode), size::ICON_MD, tint))
            // Rolling label swap (`ZCode` `RollingToolbarLabel`): the label re-enters with
            // the standard fade + rise keyed by the mode.
            .child(crate::design::motion::enter(
                t,
                gpui_kit::ElementId::Name(label_id.into()),
                div().when(!narrow, |el| el.child(mode.label())),
            ))
            .child(icon(IconName::ChevronDown, size::ICON_SM, tint))
            // Read-back for scenarios: the agent mode the switcher shows.
            .automation(
                crate::automation::Meta::labelled(crate::automation::Role::Chip, mode.label())
                    .value(mode.command()),
            ),
        pane.menu == Some(PaneMenu::Mode),
        Placement::AboveStart,
        || {
            let entity = cx.entity().downgrade();
            menu(
                "agent-mode-menu",
                t,
                256.0,
                AgentMode::ALL
                    .iter()
                    .map(|choice| {
                        let e = entity.clone();
                        let choice = *choice;
                        MenuEntry::item(choice.label(), move |_, cx| {
                            let _ = e.update(cx, |p, cx| p.pick_agent_mode(choice, cx));
                        })
                        .icon(mode_icon(choice))
                        .detail(choice.description())
                        .checked(choice == mode)
                    })
                    .collect(),
            )
            .into_any_element()
        },
        close,
    )
    .into_any_element()
}
