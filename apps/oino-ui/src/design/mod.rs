//! The Oino design system (design.md §2): one token table for every color, size, radius and
//! duration used by views and components.
//!
//! | Module | Contents |
//! |---|---|
//! | [`tokens`] | palettes (Oino Light / Dark), spacing, radii, sizes, elevation, motion |
//! | [`type_scale`] | text roles as offsets from `ui_font_size` |
//! | [`tones`] | OUS tones → tokens, OUS icon names → Lucide |
//! | [`brand`] | Unicrab sprite data and attention states |
//! | [`perf`] | frame-time ring buffer (+ overlay with `views`) |
//! | `theme` (views) | [`OinoTheme`] global, gpui-kit theme derivation, fonts |
//! | `icons` (views) | icon asset source and helpers |
//! | `syntax` (views) | highlight theme |
//! | `ticker` (views) | shared 12 fps spinner ticker |

pub mod brand;
pub mod perf;
pub mod tokens;
pub mod tones;
pub mod type_scale;

#[cfg(feature = "views")]
pub mod icons;
#[cfg(feature = "views")]
pub mod motion;
#[cfg(feature = "views")]
pub mod syntax;
#[cfg(feature = "views")]
pub mod theme;
#[cfg(feature = "views")]
pub mod ticker;

#[cfg(test)]
mod tests;

#[cfg(feature = "views")]
pub use theme::{Density, OinoTheme, TextExt};
pub use tokens::{radius, size, space};
pub use type_scale::TextRole;
