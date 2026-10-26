//! Docked interactions (design.md §4.12–§4.15): one view per pending dialog. Approvals,
//! questions, plan reviews and small dialogs dock above the composer; other `custom` dialogs
//! render as a modal (the pane asks [`InteractionView::is_modal`]).

mod approval;
mod dock_dialog;
mod plan;
mod question;

use std::time::Instant;

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    Styled as _, Window, div, px,
};
use oino_core::ui::UiResponse;

use crate::components::badge::pill;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::ticker::Ticker;
use crate::design::{TextRole, radius, size, space};
use crate::model::dialog::{Dialog, DialogKind};
use crate::model::interaction::{self, InteractionKind, PlanReview};

/// Emitted when the user answers.
#[derive(Clone, Debug)]
pub enum InteractionEvent {
    /// Send this response.
    Respond(UiResponse),
    /// Open the plan in the side pane.
    OpenPlan,
}

/// Text fields of a dialog.
pub(crate) enum Field {
    None,
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

/// The view for one pending dialog.
pub struct InteractionView {
    pub(crate) dialog: Dialog,
    pub(crate) kind: InteractionKind,
    pub(crate) plan: Option<PlanReview>,
    pub(crate) field: Field,
    pub(crate) highlighted: usize,
    pub(crate) checks: Vec<bool>,
    pub(crate) step: usize,
    pub(crate) form_checks: Vec<Vec<bool>>,
    pub(crate) form_other: Vec<Option<Entity<InputState>>>,
    pub(crate) filter: Option<Entity<InputState>>,
    pub(crate) error: Option<String>,
    pub(crate) dangerous: bool,
    pub(crate) count: usize,
    pub(crate) opened: Instant,
    pub(crate) max_height: gpui_kit::Pixels,
    pub(crate) focus: FocusHandle,
    /// Touch: the docked card (with the dock around it) took more than half the screen, so
    /// it moved into a bottom sheet for good (design.md §6.3).
    pub(crate) promoted: bool,
}

impl EventEmitter<InteractionEvent> for InteractionView {}

impl Focusable for InteractionView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl InteractionView {
    /// The view for `dialog` (`count` pending in total); focuses its first field.
    ///
    /// `typing`: the composer has focus. Approval and plan-review cards then leave focus in
    /// the composer, so a keystroke meant for the draft never resolves the card (design.md
    /// §4.12); ↑ in an empty composer or Shift+Tab moves to the card.
    pub fn new(
        dialog: Dialog,
        count: usize,
        typing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kind = interaction::kind(&dialog);
        let plan = interaction::plan_review(&dialog);
        let field = match &dialog.kind {
            DialogKind::Input {
                placeholder,
                prefill,
                secret,
            } => Field::Line(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(SharedString::from(placeholder.clone().unwrap_or_default()))
                    .masked(*secret)
                    .default_value(SharedString::from(prefill.clone().unwrap_or_default()))
            })),
            DialogKind::Approval {
                feedback: Some(placeholder),
                ..
            } => Field::Line(cx.new(|cx| {
                InputState::new(window, cx).placeholder(SharedString::from(placeholder.clone()))
            })),
            DialogKind::Custom { .. } if plan.is_some() => Field::Line(cx.new(|cx| {
                InputState::new(window, cx).placeholder("Feedback for the agent (optional)")
            })),
            DialogKind::Editor { prefill, .. } => Field::Area(cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(8)
                    .default_value(SharedString::from(prefill.clone().unwrap_or_default()))
            })),
            _ => Field::None,
        };
        let checks = match &dialog.kind {
            DialogKind::MultiSelect { options, .. } => options.iter().map(|o| o.selected).collect(),
            _ => Vec::new(),
        };
        let (form_checks, form_other) = match &dialog.kind {
            DialogKind::Form { questions, .. } => (
                questions
                    .iter()
                    .map(|q| q.options.iter().map(|o| o.selected).collect())
                    .collect(),
                questions
                    .iter()
                    .map(|q| {
                        q.other
                            .then(|| cx.new(|cx| InputState::new(window, cx).placeholder("Other…")))
                    })
                    .collect(),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        let many = matches!(&dialog.kind, DialogKind::Select { options } if options.len() > 8)
            || matches!(&dialog.kind, DialogKind::MultiSelect { options, .. } if options.len() > 8);
        let filter = many.then(|| cx.new(|cx| InputState::new(window, cx).placeholder("Filter…")));
        let dangerous = interaction::approval_is_dangerous(&dialog);
        let highlighted = match &dialog.kind {
            DialogKind::Approval { options, .. } if dangerous => {
                options.iter().position(|o| !o.kind.is_allow()).unwrap_or(0)
            }
            _ => 0,
        };
        let focus = cx.focus_handle();
        match &field {
            Field::Line(s)
                if !matches!(
                    kind,
                    InteractionKind::Approval | InteractionKind::PlanReview
                ) =>
            {
                s.update(cx, |s, cx| s.focus(window, cx));
            }
            Field::Area(s) => s.update(cx, |s, cx| s.focus(window, cx)),
            _ if typing
                && matches!(
                    kind,
                    InteractionKind::Approval | InteractionKind::PlanReview
                ) => {}
            _ => window.focus(&focus, cx),
        }
        Self {
            dialog,
            kind,
            plan,
            field,
            highlighted,
            checks,
            step: 0,
            form_checks,
            form_other,
            filter,
            error: None,
            dangerous,
            count,
            opened: Instant::now(),
            max_height: px(size::DETAIL_MAX),
            focus,
            promoted: false,
        }
    }

    /// The dialog.
    #[must_use]
    pub fn dialog(&self) -> &Dialog {
        &self.dialog
    }

    /// Whether this dialog renders as a modal (not docked).
    #[must_use]
    pub fn is_modal(&self) -> bool {
        matches!(
            self.kind,
            InteractionKind::Modal | InteractionKind::Unsupported
        )
    }

    /// Touch layouts (design.md §6.3): whether the card shows in a bottom sheet instead of
    /// docking above the composer. Small dialogs and app-level `custom` dialogs always do;
    /// plan reviews (long markdown) and questions with more than four options (taller than
    /// half a phone screen) too. Approvals stay docked.
    #[must_use]
    pub fn touch_sheet(&self) -> bool {
        match self.kind {
            InteractionKind::Dock
            | InteractionKind::Modal
            | InteractionKind::Unsupported
            | InteractionKind::PlanReview => true,
            InteractionKind::Question => match &self.dialog.kind {
                DialogKind::Form { questions, .. } => {
                    questions
                        .get(self.step)
                        .is_some_and(|q| q.options.len() > 4 || q.question.len() > 280)
                        || self.promoted
                }
                _ => false,
            },
            InteractionKind::Approval => self.promoted,
        }
    }

    /// Moves the card into a sheet (touch layouts; the dock measured it too tall).
    pub fn promote(&mut self, cx: &mut Context<Self>) {
        if !self.promoted {
            self.promoted = true;
            cx.notify();
        }
    }

    /// Whether Back / a scrim tap may cancel it (small dialogs only, design.md §6.3).
    #[must_use]
    pub fn touch_dismissible(&self) -> bool {
        matches!(self.kind, InteractionKind::Dock)
    }

    /// Cancels the dialog (the shell's Back on a small dialog sheet).
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
    }

    /// The plan markdown of a plan review.
    #[must_use]
    pub fn plan_markdown(&self) -> Option<String> {
        self.plan.as_ref().map(|p| p.markdown.clone())
    }

    /// Caps scrollable bodies at half the conversation frame.
    pub fn set_max_height(&mut self, h: gpui_kit::Pixels) {
        self.max_height = h;
    }

    /// Updates the pending count ("1 of N").
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
    }

    pub(crate) fn field_text(&self, cx: &App) -> String {
        match &self.field {
            Field::Line(s) => s.read(cx).value().to_string(),
            Field::Area(s) => s.read(cx).value().to_string(),
            Field::None => String::new(),
        }
    }

    pub(crate) fn respond(&mut self, response: UiResponse, cx: &mut Context<Self>) {
        cx.emit(InteractionEvent::Respond(response));
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        let response = self.dialog.cancel();
        self.respond(response, cx);
    }

    /// Card shell shared by every docked kind (`R_XL`, E2, header bg).
    pub(crate) fn shell(
        &self,
        t: &OinoTheme,
        glyph: gpui_kit::assets::IconName,
        title: SharedString,
        cx: &mut App,
    ) -> Div {
        let timeout = interaction::countdown(
            self.dialog.timeout_ms,
            u64::try_from(self.opened.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        if timeout.is_some() {
            Ticker::frame(cx);
        }
        let draining = timeout.as_ref().is_some_and(|(_, left)| *left <= 10_000);
        let queue = interaction::queue_label(self.count);
        let source = self.dialog.extension_id.clone();
        div()
            .flex()
            .flex_col()
            .w_full()
            .rounded(px(radius::R_CARD))
            .bg(t.card())
            .border_1()
            .border_color(t.card_border())
            .shadow(t.e2())
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .px(px(space::S4))
                    .h(px(size::INPUT + space::S1))
                    .bg(if draining {
                        t.waiting_surface()
                    } else {
                        t.header()
                    })
                    .border_b_1()
                    .border_color(t.border())
                    .child(icon(
                        glyph,
                        size::ICON,
                        if self.dangerous {
                            t.warning()
                        } else {
                            t.waiting_fg()
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_role(t, TextRole::Base)
                            .weight_medium()
                            .child(title),
                    )
                    .when_some(queue, |el, q| el.child(pill(t, t.tag(), t.fg_subtle(), q)))
                    .when_some(timeout, |el, (text, _)| {
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::S1))
                                .mono_text(t, TextRole::Sm)
                                .text_color(t.fg_subtle())
                                .child(icon(
                                    gpui_kit::assets::IconName::Clock,
                                    size::ICON_SM,
                                    t.fg_subtle(),
                                ))
                                .child(text),
                        )
                    })
                    .when_some(source, |el, s| {
                        el.child(
                            div()
                                .text_role(t, TextRole::Xs)
                                .text_color(t.fg_subtlest())
                                .child(SharedString::from(s)),
                        )
                    }),
            )
    }

    /// Moves keyboard focus into the card (from the composer).
    pub fn focus_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Whether a resolving key arrives too soon after the card appeared (a keystroke typed
    /// for something else must not approve; 400 ms arming delay).
    pub(crate) fn arming(&self) -> bool {
        self.opened.elapsed() < std::time::Duration::from_millis(400)
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if self.arming()
            && matches!(
                self.kind,
                InteractionKind::Approval | InteractionKind::PlanReview
            )
            && (key == "enter" || key == "space" || digit(event).is_some())
        {
            cx.stop_propagation();
            return;
        }
        let handled = match self.kind {
            InteractionKind::Approval => self.approval_key(event, window, cx),
            InteractionKind::Question => self.question_key(event, window, cx),
            InteractionKind::PlanReview => self.plan_key(event, cx),
            InteractionKind::Dock => self.dock_key(event, cx),
            InteractionKind::Modal | InteractionKind::Unsupported => {
                if event.keystroke.key == "escape" {
                    self.cancel(cx);
                    true
                } else {
                    false
                }
            }
        };
        if handled {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.kind {
            InteractionKind::Approval => self.render_approval(window, cx),
            InteractionKind::Question => self.render_question(window, cx),
            InteractionKind::PlanReview => self.render_plan(window, cx),
            InteractionKind::Dock => self.render_dock(window, cx),
            InteractionKind::Modal | InteractionKind::Unsupported => self.render_modal(window, cx),
        }
    }
}

/// Number key `1`–`9` as an index.
pub(crate) fn digit(event: &KeyDownEvent) -> Option<usize> {
    let k = event.keystroke.key.as_str();
    if crate::components::secondary(&event.keystroke.modifiers) || event.keystroke.modifiers.alt {
        return None;
    }
    k.parse::<usize>()
        .ok()
        .filter(|n| (1..=9).contains(n))
        .map(|n| n - 1)
}

impl Render for InteractionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = Ticker::scoped(cx.entity_id(), || self.body(window, cx));
        div()
            .w_full()
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::on_key))
            .child(body)
    }
}
