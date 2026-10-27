//! Shared GPUI components (design.md §4). Components read only tokens from
//! [`crate::design`]; desktop and mobile compose them into screens. Session-bound parts
//! (timeline, composer, dock, header, side pane) are `impl SessionPane` blocks.

#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    clippy::fn_params_excessive_bools,
    reason = "declarative GPUI element trees (session-bound render helpers stay methods for uniform call sites; callbacks are `Rc` handles cloned into child closures): one long builder chain per screen part reads better than fragments, and layout code uses short names for theme (`t`) and geometry (`w`, `h`, `r`)"
)]

pub mod badge;
#[cfg(feature = "browser")]
pub mod browser;
pub mod button;
pub mod card;
pub mod chip;
pub mod composer;
pub mod controls;
pub mod conversation;
pub mod dialog;
pub mod dock;
pub mod empty;
pub mod header;
pub mod interaction;
pub mod kbd;
pub mod keys;
pub mod log_view;
pub mod menu;
pub mod meta_row;
pub mod model_menu;
pub mod ous;
pub mod page_header;
pub mod palette;
pub mod query_rail;
pub mod section_label;
pub mod settings;
pub mod side_pane;
pub mod sidebar;
pub mod status_panel;
pub mod tabs;
#[cfg(feature = "terminal")]
pub mod terminal;
pub mod timeline;
pub mod toast;
pub mod tooltip;
pub mod unicrab;
pub mod window_chrome;

use gpui_kit::Pixels;

/// OUS `responsive` columns for a width (≈ 8 px per column).
#[must_use]
pub fn ous_cols(width: Pixels) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positive pixel width"
    )]
    let cols = (f32::from(width) / 8.0).max(0.0) as u32;
    cols
}

/// The platform's primary modifier (⌘ on macOS, Ctrl elsewhere) is held.
#[must_use]
pub fn secondary(m: &gpui_kit::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.platform
    } else {
        m.control
    }
}
