//! D9 Board (design.md §5.8): the `unipi.kanboard/board` panel promoted to the main area.

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _, Window, div, px,
};

use crate::components::button::{Btn, Variant};
use crate::components::empty::empty_state;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, size, space};
use crate::model::kanban::BOARD_PANEL;
use crate::views::shell::{Route, Workspace};

impl Workspace {
    /// The board frame.
    pub(crate) fn render_board(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let pane = self.active_pane().cloned();
        let title = pane
            .as_ref()
            .and_then(|p| {
                p.read(cx)
                    .model
                    .ui
                    .panel(BOARD_PANEL)
                    .map(|p| p.title.clone())
            })
            .unwrap_or_else(|| "Board".to_owned());
        let body = match &pane {
            Some(p) if p.read(cx).model.ui.panel(BOARD_PANEL).is_some() => {
                let toolbar = p.update(cx, |p, cx| p.panel_toolbar(BOARD_PANEL, &t, cx));
                let board = p.update(cx, |p, cx| {
                    p.render_panel_body(BOARD_PANEL, true, window, cx)
                });
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .children(toolbar)
                    .child(div().flex_1().min_h_0().p(px(space::S4)).child(board))
                    .into_any_element()
            }
            _ => empty_state(
                &t,
                IconName::Kanban,
                "Opening the board…",
                Some("Runs /unipi:kanboard in the active session.".into()),
                Vec::new(),
            )
            .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.frame())
            .border_1()
            .border_color(t.border())
            .rounded(px(crate::design::radius::R_XL))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::S2))
                    .h(px(size::HEADER))
                    .px(px(space::S3))
                    .border_b_1()
                    .border_color(t.border())
                    .child(
                        div()
                            .flex_1()
                            .text_role(&t, TextRole::Base)
                            .weight_medium()
                            .child(title),
                    )
                    .child(
                        Btn::new("board-back")
                            .label("Back to sessions")
                            .variant(Variant::Ghost)
                            .icon(IconName::ArrowLeft)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.route = Route::Sessions;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}
