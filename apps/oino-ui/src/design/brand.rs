//! Brand glyphs drawn from data (design.md §2.9, §4.28): the pixel Unicrab (cell grids copied
//! as numbers from `extensions/unipi/info-screen/assets/crab.json`) and its attention states.
//!
//! [`logo`] is the real mark — the Oino logo PNG that packaging ships — for the surfaces
//! unipi showed its logo on (round-5 #26): the conversation empty state and the sidebar
//! brand row. The `CRAB_*` pixel grids are the mascot window's glyphs only.

use crate::design::tokens::crab;

/// The Oino logo (`apps/oino-ui/assets/brand/oino-logo.png` — the 256 px PNG the desktop
/// packaging ships as `assets/oino-256.png`), embedded once.
const LOGO_PNG: &[u8] = include_bytes!("../../assets/brand/oino-logo.png");

/// The real Oino logo at `size` px (round-5 #26: empty state, sidebar brand row).
#[must_use]
pub fn logo(size: f32) -> gpui_kit::AnyElement {
    use gpui_kit::{Image, ImageFormat, IntoElement as _, Styled as _, img, px};
    img(std::sync::Arc::new(Image::from_bytes(
        ImageFormat::Png,
        LOGO_PNG.to_vec(),
    )))
    .size(px(size))
    .into_any_element()
}

/// The large crab (22 × 19 cells). Hex digits are palette indices, `.` is empty.
pub const CRAB_22: [&str; 19] = [
    ".....11........11.....",
    ".....11........11.....",
    "....1a51......15a1....",
    "...1a51........15a1...",
    "...161..111111..161...",
    "....51..11b511..15....",
    "....51.1a6666a1.15....",
    "....11155666655111....",
    "...1155852222585591...",
    "..195552222222255581..",
    "..145372c2222c273541..",
    ".1115172c2222c2715111.",
    ".19911172222227111991.",
    "1966b11445555441196691",
    "3666b656114411156b6664",
    "15611611......11611651",
    ".15816.1........61861.",
    "..4511..........1154..",
    "..111............111..",
];

/// The compact crab (14 × 12 cells).
pub const CRAB_14: [&str; 12] = [
    "...1......1...",
    "..1a1....1a1..",
    "..61...1..16..",
    "..11.1b51.15..",
    "...19666681...",
    "..1552222551..",
    ".1952c22c2581.",
    ".111222222111.",
    "16b11455411b61",
    "16166.11.66161",
    ".6161....1616.",
    ".11........11.",
];

/// Color of a crab cell (`None` = empty).
#[must_use]
pub const fn cell_color(cell: u8) -> Option<u32> {
    Some(match cell {
        b'1' | b'2' => crab::FACE,
        b'3' => 0xB820_10FF,
        b'4' => 0xD028_14FF,
        b'5' => crab::SHELL,
        b'6' => 0xFF50_30FF,
        b'7' => crab::CLAW,
        b'8' => 0xF090_18FF,
        b'9' => crab::AMBER,
        b'a' => crab::GOLD,
        b'b' => crab::CREAM,
        b'c' => 0xFFFF_FFFF,
        _ => return None,
    })
}

/// Rows of the claw tips (they "clack" while working).
pub const CLAW_ROWS_22: std::ops::Range<usize> = 0..4;

/// Unicrab attention states, highest priority first (unipi-app §1.5.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CrabState {
    /// A session waits for the user (approval, question, plan review).
    NeedsYou,
    /// A run finished while unfocused.
    Done,
    /// A run is active.
    Working,
    /// Backend unavailable.
    Offline,
    /// Idle for more than five minutes.
    Sleep,
    /// Nothing happening.
    #[default]
    Idle,
}

impl CrabState {
    /// The most urgent of `states` (`Idle` when empty).
    pub fn aggregate(states: impl IntoIterator<Item = Self>) -> Self {
        states.into_iter().min().unwrap_or_default()
    }

    /// Short label (tooltips, accessibility).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::Done => "Done",
            Self::Working => "Working",
            Self::Offline => "Offline",
            Self::Sleep => "Sleeping",
            Self::Idle => "Idle",
        }
    }

    /// Vertical offset (cells) of animation `frame` (8 fps ticker).
    #[must_use]
    pub const fn hop(self, frame: u32) -> u32 {
        match self {
            Self::NeedsYou => [0, 1, 2, 1][(frame % 4) as usize],
            Self::Done => [0, 1, 0, 0][(frame % 4) as usize],
            Self::Idle => [0, 0, 0, 0, 0, 0, 1, 1][(frame % 8) as usize],
            Self::Working | Self::Offline | Self::Sleep => 0,
        }
    }

    /// Whether the claw tips are shifted in `frame` (working clack / needs-you wave).
    #[must_use]
    pub const fn claws_open(self, frame: u32) -> bool {
        matches!(self, Self::Working | Self::NeedsYou) && frame % 2 == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_are_rectangular_and_colored() {
        assert!(CRAB_22.iter().all(|r| r.len() == 22));
        assert!(CRAB_14.iter().all(|r| r.len() == 14));
        assert_eq!(cell_color(b'.'), None);
        assert_eq!(cell_color(b'5'), Some(crab::SHELL));
    }

    #[test]
    fn aggregate_picks_the_most_urgent() {
        use CrabState::*;
        assert_eq!(CrabState::aggregate([Idle, Working, Done]), Done);
        assert_eq!(CrabState::aggregate([Working, NeedsYou]), NeedsYou);
        assert_eq!(CrabState::aggregate([]), Idle);
        assert_eq!(NeedsYou.hop(2), 2);
        assert!(Working.claws_open(1));
        assert!(!Idle.claws_open(1));
    }
}
