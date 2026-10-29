//! The new-chat page (round5 #6, ZCode `ConversationDraftEmptyState`): a greeting with the
//! Oino logo and a project picker — recent projects (`recent_cwds`), the "No project"
//! bucket and "Choose folder…" over the native prompt. Every choice calls
//! [`AppShell::open`] with `OpenRequest::New`, which reuses an existing empty session in
//! that folder instead of stacking drafts (#4); the sidebar keeps the per-project `+`
//! affordances (ZCode `PurposeSections`). Esc returns to the sessions.

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use crate::automation::Automatable as _;
use crate::components::button::{Btn, BtnSize, Variant};
use crate::components::kbd::legend;
use crate::components::section_label::label as section_label;
use crate::design::icons::icon;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};
use crate::model::sessions::project_label;
use crate::views::shell::AppShell;

/// One row of the project picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectChoice {
    /// Row label (the folder's tail).
    pub(crate) label: String,
    /// The full cwd (the row's detail and the `OpenRequest::New` payload).
    pub(crate) cwd: String,
}

/// The picker rows for `recents` (deduplicated, most recent first, capped).
#[must_use]
pub fn project_choices(recents: &[String], max: usize) -> Vec<ProjectChoice> {
    let mut out: Vec<ProjectChoice> = Vec::new();
    for cwd in recents {
        if cwd.is_empty() || out.iter().any(|c| c.cwd == *cwd) {
            continue;
        }
        out.push(ProjectChoice {
            label: project_label(cwd),
            cwd: cwd.clone(),
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

/// How many recent projects the page shows.
const MAX_PROJECTS: usize = 8;

impl AppShell {
    /// Routes to the new-chat page (#6); the session spawns (deduplicated) when a project
    /// is picked.
    pub(crate) fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_view = None;
        self.route = crate::views::shell::Route::NewChat;
        window.focus(&self.new_chat_focus, cx);
        cx.notify();
    }

    /// The new-chat page: greeting, logo, project picker.
    #[allow(clippy::too_many_lines, reason = "the page layout")]
    pub(crate) fn render_new_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let choices = project_choices(&self.settings.recent_cwds, MAX_PROJECTS);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, choice) in choices.iter().enumerate() {
            let cwd = choice.cwd.clone();
            rows.push(
                div()
                    .id(gpui_kit::ElementId::NamedInteger(
                        "new-chat-project".into(),
                        i as u64,
                    ))
                    .flex()
                    .items_center()
                    .gap(px(space::S3))
                    .min_h(px(size::ROW_COMPACT))
                    .px(px(space::S3_5))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.new_session_in(&cwd, window, cx);
                    }))
                    .child(icon(IconName::Folder, size::ICON, t.fg_subtle()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_role(&t, TextRole::Base)
                                    .weight_medium()
                                    .child(SharedString::from(choice.label.clone())),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_role(&t, TextRole::Sm)
                                    .text_color(t.fg_subtlest())
                                    .child(SharedString::from(choice.cwd.clone())),
                            ),
                    )
                    .automation(
                        crate::automation::Meta::labelled(
                            crate::automation::Role::Row,
                            SharedString::from(choice.label.clone()),
                        )
                        .value(choice.cwd.clone()),
                    )
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .id("new-chat-none")
                .flex()
                .items_center()
                .gap(px(space::S3))
                .min_h(px(size::ROW_COMPACT))
                .px(px(space::S3_5))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover()))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    let cwd = this.default_cwd();
                    this.open(
                        crate::host::OpenRequest::New { cwd },
                        window,
                        cx,
                    );
                }))
                .child(icon(IconName::MessageSquareDashed, size::ICON, t.fg_subtle()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_role(&t, TextRole::Base)
                                .weight_medium()
                                .child("No project"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_role(&t, TextRole::Sm)
                                .text_color(t.fg_subtlest())
                                .child("Start in the default folder"),
                        ),
                )
                .automation(
                    crate::automation::Meta::labelled(crate::automation::Role::Row, "No project"),
                )
                .into_any_element(),
        );
        rows.push(
            div()
                .id("new-chat-folder")
                .flex()
                .items_center()
                .gap(px(space::S3))
                .min_h(px(size::ROW_COMPACT))
                .px(px(space::S3_5))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover()))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_folder(window, cx);
                }))
                .child(icon(IconName::FolderOpen, size::ICON, t.fg_subtle()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_role(&t, TextRole::Base)
                                .weight_medium()
                                .child("Choose folder…"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_role(&t, TextRole::Sm)
                                .text_color(t.fg_subtlest())
                                .child("Pick where this chat works"),
                        ),
                )
                .automation(crate::automation::Meta::labelled(
                    crate::automation::Role::Row,
                    "Choose folder",
                ))
                .into_any_element(),
        );
        div()
            .id("new-chat")
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .p(px(size::PAGE_GUTTER))
            .track_focus(&self.new_chat_focus)
            .capture_key_down(cx.listener(Self::on_new_chat_key))
            .on_mouse_down(gpui_kit::MouseButton::Left, {
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.new_chat_focus, cx);
                })
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .w_full()
                    .max_w(px(size::WELCOME_COMPOSER))
                    .gap(px(space::S2_5))
                    // The real Oino mark (round5 #26), consumed from `design/brand.rs`.
                    .child(
                        div()
                            .id("brand-logo")
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::design::brand::logo(size::CRAB_EMPTY))
                            .automation(crate::automation::Meta::labelled(
                                crate::automation::Role::Other,
                                "Oino",
                            )),
                    )
                    .child(
                        div()
                            .pt(px(space::S2))
                            .text_role(&t, TextRole::Display)
                            .weight_semibold()
                            .text_color(t.fg())
                            .child("What should we work on?"),
                    )
                    .child(
                        div()
                            .pb(px(space::S2))
                            .text_role(&t, TextRole::Base)
                            .text_color(t.fg_subtle())
                            .child("Pick a project for the new chat — it starts there."),
                    )
                    .child(section_label(&t, "Projects").w_full())
                    .child(
                        crate::components::card::grouped(&t, rows, size::ROW_DIVIDER_INSET)
                            .w_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .justify_end()
                            .pt(px(space::S2))
                            .child(
                                Btn::new("new-chat-back")
                                    .label("Back to sessions")
                                    .icon(IconName::ArrowLeft)
                                    .variant(Variant::Secondary)
                                    .size(BtnSize::Xl)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.run_app_action("sessions", window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .pt(px(space::S2))
                            .child(legend(&t, "esc", "back")),
                    ),
            )
            .into_any_element()
    }

    /// Keys of the new-chat page: Esc returns to the sessions.
    fn on_new_chat_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key.as_str() != "escape" {
            return;
        }
        self.run_app_action("sessions", window, cx);
        cx.stop_propagation();
    }
}

#[cfg(test)]
mod tests {
    use super::project_choices;

    fn recents() -> Vec<String> {
        ["/w/oino", "/w/oino", "/w/z", "", "/w/oino/", "/w/a"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn choices_dedupe_skip_blank_and_cap() {
        let all = project_choices(&recents(), 8);
        // `/w/oino` (twice) and `/w/oino/` (same tail, different path — kept as given).
        assert_eq!(
            all.iter().map(|c| c.cwd.as_str()).collect::<Vec<_>>(),
            ["/w/oino", "/w/z", "/w/oino/", "/w/a"]
        );
        assert_eq!(all[0].label, "oino");
        assert_eq!(all[1].label, "z");
        let capped = project_choices(&recents(), 2);
        assert_eq!(capped.len(), 2);
        assert!(project_choices(&[], 8).is_empty());
    }
}
