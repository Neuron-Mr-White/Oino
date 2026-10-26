//! M5 Full-screen pages (design.md §6.5) with a detail header (52: back, title, actions):
//! Diff (unified), Output (log tail, follows the end) and overlay / fullscreen extension
//! panels (info-screen, updater, MCP, skills, hints, graph) with their keys as a toolbar.

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, ClickEvent, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};

use crate::components::log_view::log_lines;
use crate::components::ous::diff_lines;
use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, space};
use crate::mobile::shell::MobileShell;
use crate::model::tools;
use crate::views::session_pane::SessionPane;

/// Lines kept in the output page.
const OUTPUT_LINES: usize = 5000;

impl MobileShell {
    fn page_frame(
        t: &OinoTheme,
        header: AnyElement,
        toolbar: Option<AnyElement>,
        body: AnyElement,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .children(toolbar.map(|tb| {
                div()
                    .id("m-page-toolbar")
                    .flex_none()
                    .w_full()
                    .overflow_x_scroll()
                    .border_b_1()
                    .border_color(t.border())
                    .child(tb)
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(t.border())
                    .child(body),
            )
            .into_any_element()
    }

    pub(crate) fn render_diff_page(
        &mut self,
        title: &str,
        patch: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let d = tools::diff_preview(patch, 0);
        let copy = patch.to_owned();
        let header = Self::app_bar(
            &t,
            Some(Self::back_btn(cx)),
            Self::title(
                &t,
                title.to_owned(),
                Some(format!("+{} −{}", d.added, d.removed)),
            ),
            vec![Self::icon_btn(
                "m-diff-copy",
                IconName::Copy,
                "Copy patch",
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    this.toast(
                        crate::components::toast::ToastKind::Success,
                        None,
                        "Patch copied",
                        cx,
                    );
                }),
            )],
        );
        let body = div()
            .id("m-diff")
            .size_full()
            .overflow_y_scroll()
            .p(px(space::S3))
            .child(diff_lines(&t, patch, usize::MAX))
            .into_any_element();
        Self::page_frame(&t, header, None, body)
    }

    pub(crate) fn render_output_page(
        &mut self,
        title: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let lines = tools::tail(text, OUTPUT_LINES);
        let n = lines.len();
        let copy = text.to_owned();
        let header = Self::app_bar(
            &t,
            Some(Self::back_btn(cx)),
            Self::title(&t, "Output", Some(format!("$ {title}"))),
            vec![Self::icon_btn(
                "m-out-copy",
                IconName::Copy,
                "Copy output",
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                    this.toast(
                        crate::components::toast::ToastKind::Success,
                        None,
                        "Output copied",
                        cx,
                    );
                }),
            )],
        );
        // Tail follow: the page opens at the end (`on_show_side`).
        let scroll = self.page_scroll.clone();
        let body = div()
            .id("m-output")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .p(px(space::S3))
            .child(
                div()
                    .pb(px(space::S2))
                    .text_role(&t, TextRole::Sm)
                    .text_color(t.fg_subtle())
                    .child(SharedString::from(format!("{n} lines"))),
            )
            .child(log_lines(&t, &lines, "m-output-lines"))
            .into_any_element();
        Self::page_frame(&t, header, None, body)
    }

    /// An overlay / fullscreen panel of `pane` as a page; Back closes the panel.
    pub(crate) fn render_panel_page(
        &mut self,
        pane: &Entity<SessionPane>,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = OinoTheme::get(cx);
        let title = pane
            .read(cx)
            .model
            .ui
            .panel(id)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        let (toolbar, body) = pane.update(cx, |p, cx| {
            (
                p.panel_toolbar(id, &t, cx),
                p.render_panel_body(id, false, window, cx),
            )
        });
        let close = id.to_owned();
        let header = Self::app_bar(
            &t,
            Some(Self::back_btn(cx)),
            Self::title(&t, title, None),
            vec![Self::icon_btn(
                "m-panel-close",
                IconName::X,
                "Close",
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let id = close.clone();
                    this.with_pane(cx, |p, cx| p.close_panel(&id, cx));
                }),
            )],
        );
        let body = div()
            .id("m-panel-page")
            .size_full()
            .overflow_y_scroll()
            .p(px(space::S4))
            .child(body)
            .into_any_element();
        Self::page_frame(&t, header, toolbar, body)
    }
}
