//! `Kbd` (design.md A2.15): a 20 px key chip, radius 5, `tag` (surface-3) fill with an inset
//! `line` border, 11/500. Labels follow the platform ([`crate::components::badge::platform_keys`]).

use gpui_kit::{Div, ParentElement as _, SharedString, Styled as _, div, px};

use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{TextRole, radius, size, space};

/// A key chip.
pub fn kbd(t: &OinoTheme, keys: impl Into<SharedString>) -> Div {
    let keys: SharedString = keys.into();
    let keys = SharedString::from(crate::components::badge::platform_keys(&keys));
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .min_w(px(size::KBD))
        .h(px(size::KBD))
        .px(px(space::S1))
        .rounded(px(radius::R_KBD))
        .bg(t.tag())
        .border_1()
        .border_color(t.border())
        .text_color(t.fg_subtle())
        .text_role(t, TextRole::Xs)
        .weight_medium()
        .child(keys)
}

/// A footer legend entry (`Kbd` + label 12 dim), as in the palette footer.
pub fn legend(t: &OinoTheme, keys: impl Into<SharedString>, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::S1_5))
        .text_role(t, TextRole::Sm)
        .text_color(t.fg_subtlest())
        .child(kbd(t, keys))
        .child(label.into())
}
