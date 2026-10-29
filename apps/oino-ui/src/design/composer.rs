//! Round-5 composer geometry (round5.md §2 #1/#3): picker and context-ring metrics from
//! the `unipi` reference and `ZCode` that do not exist in [`crate::design::tokens`] yet. They
//! live in their own module so the round's packages do not fight over `tokens.rs`.

/// Context ring diameter (`ZCode` `context.tsx`: a 14 px gauge).
pub const RING: f32 = 14.0;
/// Context ring stroke width.
pub const RING_STROKE: f32 = 2.0;
/// Source swatch size of the context hover panel.
pub const SWATCH: f32 = 8.0;
/// Multisegment bar height of the context hover panel (`ZCode` `h-2`).
pub const BAR: f32 = 8.0;
/// Context hover panel width (`ZCode` `!w-64`).
pub const PANEL: f32 = 256.0;
/// Hover panel / search field radius (`unipi` `.model-search` radius 12).
pub const PANEL_R: f32 = 12.0;
/// The "Now" pill height (`unipi` `.mp-current-label` 18 px).
pub const NOW_PILL: f32 = 18.0;
/// The picker search field height (`unipi` `.model-search` 42 px).
pub const SEARCH_H: f32 = 42.0;
/// The picker list max height (`unipi` `min(44vh, 380px)` desktop value).
pub const LIST_MAX: f32 = 380.0;
/// A fusion column list max height (`unipi` `min(30vh, 220px)`).
pub const FUSION_LIST_MAX: f32 = 220.0;
