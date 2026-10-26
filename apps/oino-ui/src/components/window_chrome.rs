//! Client-side window chrome on Linux and Windows (design.md §3.1, checklist #54): no system
//! title bar; the sidebar top row and the pane headers are the drag region, the window
//! controls sit at the right end of the right-most header, and the window edges resize.
//! macOS keeps its transparent title bar with traffic lights in the sidebar's safe area.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, CursorStyle, Decorations, InteractiveElement as _, IntoElement as _, MouseButton,
    ParentElement as _, ResizeEdge, Stateful, StatefulInteractiveElement as _, Styled as _, Window,
    WindowControlArea, div, px,
};

use crate::design::size;
use crate::design::theme::OinoTheme;

/// Whether this window draws its own chrome (Linux client decorations granted, or Windows).
#[must_use]
pub fn client_decorated(window: &Window) -> bool {
    if cfg!(target_os = "windows") {
        return true;
    }
    cfg!(target_os = "linux") && matches!(window.window_decorations(), Decorations::Client { .. })
}

/// Whether no edge of the window is tiled / maximized (rounded corners, resize edges).
#[must_use]
pub fn floating(window: &Window) -> bool {
    match window.window_decorations() {
        Decorations::Client { tiling } => {
            !tiling.top && !tiling.left && !tiling.right && !tiling.bottom && !window.is_maximized()
        }
        Decorations::Server => !window.is_maximized(),
    }
}

/// Minimize / maximize-restore / close, sized to the header height.
#[must_use]
pub fn window_controls(t: &OinoTheme, maximized: bool) -> AnyElement {
    let control = |id: &'static str, glyph: IconName, close: bool, area: WindowControlArea| {
        let hover = if close { t.deep() } else { t.hover() };
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .size(px(size::CONTROL))
            .rounded(px(crate::design::radius::R_FULL))
            .text_color(t.fg_subtle())
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .when(cfg!(target_os = "windows"), |el| {
                el.window_control_area(area)
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match area {
                    WindowControlArea::Min => window.minimize_window(),
                    WindowControlArea::Max => window.zoom_window(),
                    _ => window.remove_window(),
                }
            })
            .child(crate::design::icons::icon(
                glyph,
                size::ICON_MD,
                t.fg_subtle(),
            ))
    };
    div()
        .flex()
        .items_center()
        .gap(px(crate::design::space::S1))
        .pl(px(crate::design::space::S2))
        .child(control(
            "win-min",
            IconName::WindowMinimize,
            false,
            WindowControlArea::Min,
        ))
        .child(control(
            "win-max",
            if maximized {
                IconName::WindowRestore
            } else {
                IconName::WindowMaximize
            },
            false,
            WindowControlArea::Max,
        ))
        .child(control(
            "win-close",
            IconName::WindowClose,
            true,
            WindowControlArea::Close,
        ))
        .into_any_element()
}

/// Makes `el` a window drag region: a drag (not a click) moves the window, a double click
/// zooms, right click opens the window menu (Linux). `armed` keeps the press state.
pub fn drag_region(
    el: Stateful<gpui_kit::Div>,
    armed: std::rc::Rc<std::cell::Cell<bool>>,
) -> Stateful<gpui_kit::Div> {
    let (a1, a2, a3) = (armed.clone(), armed.clone(), armed);
    el.when(cfg!(target_os = "windows"), |el| {
        el.window_control_area(WindowControlArea::Drag)
    })
    .on_mouse_down(MouseButton::Left, move |_, _, _| a1.set(true))
    .on_mouse_up(MouseButton::Left, move |_, _, _| a2.set(false))
    .on_mouse_move(move |e, window, _| {
        if a3.get() && e.pressed_button == Some(MouseButton::Left) {
            a3.set(false);
            window.start_window_move();
        }
    })
    .on_mouse_down(MouseButton::Right, |e, window, _| {
        if cfg!(target_os = "linux") {
            window.show_window_menu(e.position);
        }
    })
    .on_click(|e, window, _| {
        if e.click_count() == 2 {
            window.zoom_window();
        }
    })
}

/// Invisible resize handles along the window edges (client decorations only).
#[must_use]
pub fn resize_edges(width: f32) -> Vec<AnyElement> {
    let edge = |id: &'static str, e: ResizeEdge, cursor: CursorStyle| {
        div()
            .id(id)
            .absolute()
            .cursor(cursor)
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.start_window_resize(e);
            })
    };
    let w = px(width);
    let corner = px(width * 3.0);
    vec![
        edge("rz-t", ResizeEdge::Top, CursorStyle::ResizeUpDown)
            .top_0()
            .left(corner)
            .right(corner)
            .h(w)
            .into_any_element(),
        edge("rz-b", ResizeEdge::Bottom, CursorStyle::ResizeUpDown)
            .bottom_0()
            .left(corner)
            .right(corner)
            .h(w)
            .into_any_element(),
        edge("rz-l", ResizeEdge::Left, CursorStyle::ResizeLeftRight)
            .left_0()
            .top(corner)
            .bottom(corner)
            .w(w)
            .into_any_element(),
        edge("rz-r", ResizeEdge::Right, CursorStyle::ResizeLeftRight)
            .right_0()
            .top(corner)
            .bottom(corner)
            .w(w)
            .into_any_element(),
        edge(
            "rz-tl",
            ResizeEdge::TopLeft,
            CursorStyle::ResizeUpLeftDownRight,
        )
        .top_0()
        .left_0()
        .size(corner)
        .into_any_element(),
        edge(
            "rz-tr",
            ResizeEdge::TopRight,
            CursorStyle::ResizeUpRightDownLeft,
        )
        .top_0()
        .right_0()
        .size(corner)
        .into_any_element(),
        edge(
            "rz-bl",
            ResizeEdge::BottomLeft,
            CursorStyle::ResizeUpRightDownLeft,
        )
        .bottom_0()
        .left_0()
        .size(corner)
        .into_any_element(),
        edge(
            "rz-br",
            ResizeEdge::BottomRight,
            CursorStyle::ResizeUpLeftDownRight,
        )
        .bottom_0()
        .right_0()
        .size(corner)
        .into_any_element(),
    ]
}
