//! M4 Right drawer (design.md §6.4): 88 % width from the right on `frame`, `R_2XL` leading
//! corners, scrim on the rest; the pane's side tabs (Review, Plan, Agent tabs, `sidePane`
//! panels). Swipe right, a scrim tap or Back closes it.

use gpui_kit::{
    AnyElement, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollEvent, StatefulInteractiveElement as _, Styled as _, Window, div, px,
    relative,
};

use crate::design::theme::OinoTheme;
use crate::design::tokens::touch;
use crate::design::{radius, space};
use crate::mobile::shell::MobileShell;
use crate::model::kanban::BOARD_PANEL;
use crate::views::session_pane::{SessionPane, SideTab};

impl MobileShell {
    pub(crate) fn render_drawer(
        &mut self,
        pane: &Entity<SessionPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        // Every sidePane panel (but the board, which has its own page) gets a tab.
        let body = pane.update(cx, |p, cx| {
            if !p.side.tabs.contains(&SideTab::Review) {
                p.side.tabs.insert(0, SideTab::Review);
            }
            for (id, _) in p.side_panels() {
                let tab = SideTab::Panel(id.clone());
                if id != BOARD_PANEL && !p.side.tabs.contains(&tab) {
                    p.side.tabs.push(tab);
                }
            }
            let plan = p
                .interaction
                .as_ref()
                .is_some_and(|v| v.read(cx).plan_markdown().is_some());
            if plan && !p.side.tabs.contains(&SideTab::Plan) {
                p.side.tabs.insert(1.min(p.side.tabs.len()), SideTab::Plan);
            }
            p.side.tabs.retain(|tab| {
                !matches!(tab, SideTab::Diff { .. } | SideTab::Output { .. })
                    && *tab != SideTab::Panel(BOARD_PANEL.to_owned())
            });
            if p.side.active >= p.side.tabs.len() {
                p.side.active = 0;
            }
            p.render_side(window, cx)
        });
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_row()
            .child(
                div()
                    .id("m-drawer-scrim")
                    .flex_1()
                    .h_full()
                    .bg(t.scrim())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.drawer = false;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("m-drawer")
                    .w(relative(touch::DRAWER))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(t.frame())
                    .rounded_tl(px(radius::R_2XL))
                    .rounded_bl(px(radius::R_2XL))
                    .border_l_1()
                    .border_color(t.border())
                    .shadow(t.e3())
                    .overflow_hidden()
                    .occlude()
                    .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                        // A firm swipe to the right closes the drawer.
                        let d = e.delta.pixel_delta(px(touch::ROW));
                        if f32::from(d.x) > space::S6 && d.x.abs() > d.y.abs() * 2.0 {
                            this.drawer = false;
                            cx.notify();
                        }
                    }))
                    .child(div().pt(px(space::S1)).flex_1().min_h_0().child(body)),
            )
            .into_any_element()
    }
}
