//! Design tokens (design.md §2): the Oino Light / Oino Dark palettes (`ZCode` Zai Light / Zai
//! Dark values resolved to RGBA), spacing, radii, sizes, elevation and motion.
//!
//! Colors are `0xRRGGBBAA` and converted at theme install time; nothing here depends on GPUI,
//! so the contrast tests run without a window.

/// One theme's colors (`0xRRGGBBAA`). Field meaning follows design.md §2.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    missing_docs,
    reason = "one field per row of the design.md §2.1 token table"
)]
pub struct Palette {
    // structure
    pub window: u32,
    pub sidebar: u32,
    pub frame: u32,
    pub header: u32,
    pub surface: u32,
    pub surface_hover: u32,
    pub card: u32,
    pub card_border: u32,
    pub card_selected: u32,
    pub popover: u32,
    pub popover_border: u32,
    pub menu: u32,
    pub menu_hover: u32,
    pub input: u32,
    pub input_border: u32,
    pub input_border_hover: u32,
    pub input_border_focus: u32,
    pub border: u32,
    pub border_hover: u32,
    pub hover: u32,
    pub selected: u32,
    pub tab: u32,
    pub tab_active: u32,
    pub tag: u32,
    pub tooltip: u32,
    pub tooltip_fg: u32,
    pub toast: u32,
    pub scrim: u32,
    pub resize_handle: u32,
    // text
    pub fg: u32,
    pub fg_subtle: u32,
    pub fg_subtlest: u32,
    pub fg_inverse: u32,
    // actions
    pub primary: u32,
    pub primary_fg: u32,
    pub secondary: u32,
    pub brand: u32,
    pub accent: u32,
    // semantic
    pub success: u32,
    pub success_surface: u32,
    pub success_surface_fg: u32,
    pub warning: u32,
    pub warning_surface: u32,
    pub warning_surface_fg: u32,
    pub danger: u32,
    pub danger_surface: u32,
    pub danger_surface_fg: u32,
    pub progress_surface: u32,
    pub progress_surface_fg: u32,
    pub waiting_surface: u32,
    pub waiting_fg: u32,
    pub idle_task: u32,
    pub idle_task_surface: u32,
    // content
    pub diff_added: u32,
    pub diff_removed: u32,
    pub diff_added_bg: u32,
    pub diff_removed_bg: u32,
    pub git_modified: u32,
    pub git_added: u32,
    pub git_deleted: u32,
    pub git_renamed: u32,
    pub find: u32,
    pub find_active: u32,
    pub file_node: u32,
    pub file_node_fg: u32,
    pub skill_node: u32,
    pub skill_node_fg: u32,
    pub command_node: u32,
    pub command_node_fg: u32,
    pub agent_node: u32,
    pub agent_node_fg: u32,
    pub session_node: u32,
    pub session_node_fg: u32,
    pub harness: u32,
    pub harness_surface: u32,
    pub thinking: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
    pub categorical: [u32; 12],
}

/// Oino Light (Zai Light).
pub const LIGHT: Palette = Palette {
    window: 0xECEC_EEFF,
    sidebar: 0x0000_0000,
    frame: 0xF8F8_F8FF,
    header: 0xF8F8_F8FF,
    surface: 0x0D0D_0D08,
    surface_hover: 0x0D0D_0D0D,
    card: 0xFFFF_FFFF,
    card_border: 0x0D0D_0D1A,
    card_selected: 0xFFFF_FFFF,
    popover: 0xFFFF_FFFF,
    popover_border: 0x0D0D_0D1A,
    menu: 0xFFFF_FFFF,
    menu_hover: 0xF0F0_F0FF,
    input: 0xFFFF_FFFF,
    input_border: 0x0D0D_0D1A,
    input_border_hover: 0x0D0D_0D26,
    input_border_focus: 0x0D0D_0D26,
    border: 0x0D0D_0D1A,
    border_hover: 0x0D0D_0D26,
    hover: 0x0D0D_0D0D,
    selected: 0x0D0D_0D0D,
    tab: 0xF0F0_F0FF,
    tab_active: 0xFFFF_FFFF,
    tag: 0xE6E6_E6FF,
    tooltip: 0xF0F0_F0FF,
    tooltip_fg: 0x0D0D_0DFF,
    toast: 0xFFFF_FFF2,
    scrim: 0x0000_004D,
    resize_handle: 0x0D0D_0D33,
    fg: 0x2626_26FF,
    fg_subtle: 0x2626_2699,
    fg_subtlest: 0x2626_2666,
    fg_inverse: 0xFFFF_FFFF,
    primary: 0x0000_00FF,
    primary_fg: 0xFFFF_FFFF,
    secondary: 0xE6E6_E6FF,
    brand: 0xE030_1AFF,
    accent: 0xEBF4_FFFF,
    success: 0x1E8A_3EFF,
    success_surface: 0xEAF7_EEFF,
    success_surface_fg: 0x166B_32FF,
    warning: 0xE07B_00FF,
    warning_surface: 0xE07B_001F,
    warning_surface_fg: 0x914E_00FF,
    danger: 0xE031_31FF,
    danger_surface: 0xE031_311A,
    danger_surface_fg: 0xB420_20FF,
    progress_surface: 0xEFF6_FFFF,
    progress_surface_fg: 0x1D4E_D8FF,
    waiting_surface: 0xEAF7_EEFF,
    waiting_fg: 0x166B_32FF,
    idle_task: 0x9E77_EDFF,
    idle_task_surface: 0xF5F3_FFFF,
    diff_added: 0x1E8A_3EFF,
    diff_removed: 0xE031_31FF,
    diff_added_bg: 0x1E8A_3E1A,
    diff_removed_bg: 0xE031_311A,
    git_modified: 0xE07B_00FF,
    git_added: 0x1E8A_3EFF,
    git_deleted: 0xE031_31FF,
    git_renamed: 0x0B7F_FFFF,
    find: 0xFFF4_EBFF,
    find_active: 0xFFB2_6BFF,
    file_node: 0x1A70_B81A,
    file_node_fg: 0x1A70_B8FF,
    skill_node: 0x7453_B01A,
    skill_node_fg: 0x7453_B0FF,
    command_node: 0x5662_701A,
    command_node_fg: 0x5662_70FF,
    agent_node: 0x6F7A_2F1A,
    agent_node_fg: 0x6F7A_2FFF,
    session_node: 0x1480_7A1A,
    session_node_fg: 0x1480_7AFF,
    harness: 0x7453_B0FF,
    harness_surface: 0x7453_B014,
    thinking: 0x2626_2699,
    selection: 0x0B7F_FF38,
    ansi: [
        0x5C5C_5CFF,
        0xE031_31FF,
        0x1E8A_3EFF,
        0xE07B_00FF,
        0x0B7F_FFFF,
        0x9E77_EDFF,
        0x0AA7_A7FF,
        0xADAD_ADFF,
        0x8888_88FF,
        0xE031_31FF,
        0x1E8A_3EFF,
        0xE07B_00FF,
        0x0066_DDFF,
        0x9E77_EDFF,
        0x0AA7_A7FF,
        0x0D0D_0DFF,
    ],
    categorical: [
        0x0B7F_FFFF,
        0x1E8A_3EFF,
        0x9E77_EDFF,
        0xE031_31FF,
        0xE07B_00FF,
        0x0AA7_A7FF,
        0x1A70_B8FF,
        0x7453_B0FF,
        0x5662_70FF,
        0x6F7A_2FFF,
        0x1480_7AFF,
        0xB87A_1AFF,
    ],
};

/// Oino Dark (Zai Dark).
pub const DARK: Palette = Palette {
    window: 0x2B2B_2BFF,
    sidebar: 0x0000_0000,
    frame: 0x1616_16FF,
    header: 0x2020_20FF,
    surface: 0xFFFF_FF0D,
    surface_hover: 0xFFFF_FF1A,
    card: 0x2B2B_2BFF,
    card_border: 0xFFFF_FF1A,
    card_selected: 0x2B2B_2BFF,
    popover: 0x2B2B_2BFF,
    popover_border: 0xFFFF_FF1A,
    menu: 0x2B2B_2BFF,
    menu_hover: 0x3636_36FF,
    input: 0x2B2B_2BFF,
    input_border: 0xFFFF_FF1A,
    input_border_hover: 0xFFFF_FF26,
    input_border_focus: 0xFFFF_FF26,
    border: 0xFFFF_FF1A,
    border_hover: 0xFFFF_FF26,
    hover: 0xFFFF_FF0D,
    selected: 0xFFFF_FF1A,
    tab: 0x2020_20FF,
    tab_active: 0x1616_16FF,
    tag: 0x3636_36FF,
    tooltip: 0x2B2B_2BFF,
    tooltip_fg: 0xF8F8_F8FF,
    toast: 0x2B2B_2BF2,
    scrim: 0x0000_0080,
    resize_handle: 0xFFFF_FF33,
    fg: 0xD4D4_D4FF,
    fg_subtle: 0xD4D4_D499,
    fg_subtlest: 0xD4D4_D44D,
    fg_inverse: 0x0000_00FF,
    primary: 0xFFFF_FFFF,
    primary_fg: 0x0000_00FF,
    secondary: 0x3636_36FF,
    brand: 0xFF5A_3CFF,
    accent: 0x001D_3DFF,
    success: 0x46BF_72FF,
    success_surface: 0x46BF_7229,
    success_surface_fg: 0x87D9_A4FF,
    warning: 0xFF8A_30FF,
    warning_surface: 0xFF8A_3029,
    warning_surface_fg: 0xFFA1_59FF,
    danger: 0xFF5C_5CFF,
    danger_surface: 0xFF5C_5C29,
    danger_surface_fg: 0xFF85_85FF,
    progress_surface: 0x4682_F029,
    progress_surface_fg: 0x93C5_FDFF,
    waiting_surface: 0x46BF_7229,
    waiting_fg: 0x87D9_A4FF,
    idle_task: 0x7B5C_E5FF,
    idle_task_surface: 0x160D_38FF,
    diff_added: 0x46BF_72FF,
    diff_removed: 0xFF5C_5CFF,
    diff_added_bg: 0x46BF_721F,
    diff_removed_bg: 0xFF5C_5C1F,
    git_modified: 0xFF8A_30FF,
    git_added: 0x46BF_72FF,
    git_deleted: 0xFF5C_5CFF,
    git_renamed: 0x4099_FFFF,
    find: 0x5425_00FF,
    find_active: 0xFF8A_30FF,
    file_node: 0x70AE_E029,
    file_node_fg: 0x8FC5_EFFF,
    skill_node: 0xA689_DA29,
    skill_node_fg: 0xBDA5_E6FF,
    command_node: 0x9AA6_B424,
    command_node_fg: 0xB5C0_CCFF,
    agent_node: 0xB7BD_7529,
    agent_node_fg: 0xC8CD_90FF,
    session_node: 0x68C4_BC29,
    session_node_fg: 0x93D8_D2FF,
    harness: 0xBDA5_E6FF,
    harness_surface: 0xA689_DA1F,
    thinking: 0xD4D4_D499,
    selection: 0x4099_FF47,
    ansi: [
        0x3636_36FF,
        0xFF5C_5CFF,
        0x46BF_72FF,
        0xFF8A_30FF,
        0x4099_FFFF,
        0x7B5C_E5FF,
        0x42C8_C8FF,
        0xADAD_ADFF,
        0x7474_74FF,
        0xFF99_99FF,
        0x87D9_A4FF,
        0xFFB2_6BFF,
        0x80BE_FFFF,
        0xA888_F2FF,
        0x8EE5_E5FF,
        0xF8F8_F8FF,
    ],
    categorical: [
        0x4099_FFFF,
        0x46BF_72FF,
        0x7B5C_E5FF,
        0xFF5C_5CFF,
        0xFF8A_30FF,
        0x42C8_C8FF,
        0x8FC5_EFFF,
        0xBDA5_E6FF,
        0xB5C0_CCFF,
        0xC8CD_90FF,
        0x93D8_D2FF,
        0xEFC5_8FFF,
    ],
};

/// Spacing grid (4 px base, design.md §2.5).
pub mod space {
    /// 0.
    pub const S0: f32 = 0.0;
    /// 4 — icon–text gaps, frame gaps.
    pub const S1: f32 = 4.0;
    /// 8 — control padding, inline gaps.
    pub const S2: f32 = 8.0;
    /// 12 — dense rows, menu rows.
    pub const S3: f32 = 12.0;
    /// 16 — card/panel padding, conversation gutters.
    pub const S4: f32 = 16.0;
    /// 20 — dialog interiors.
    pub const S5: f32 = 20.0;
    /// 24.
    pub const S6: f32 = 24.0;
    /// 32 — empty-state rhythm.
    pub const S8: f32 = 32.0;
    /// 40.
    pub const S10: f32 = 40.0;
    /// 48.
    pub const S12: f32 = 48.0;
    /// 2 — hairline gap between menu rows (half step, design.md §4.4).
    pub const HALF: f32 = 2.0;
    /// 6 — button gap (design.md §4.1).
    pub const S1_5: f32 = 6.0;
}

/// Radii (Tailwind v4, design.md §2.6).
pub mod radius {
    /// 4.
    pub const R_SM: f32 = 4.0;
    /// 6.
    pub const R_MD: f32 = 6.0;
    /// 8.
    pub const R_LG: f32 = 8.0;
    /// 12.
    pub const R_XL: f32 = 12.0;
    /// 16 (composer, status panel, dialogs, toasts).
    pub const R_2XL: f32 = 16.0;
    /// Pills.
    pub const R_FULL: f32 = 9999.0;
}

/// Fixed sizes (design.md §2.7).
pub mod size {
    /// Small icon.
    pub const ICON_SM: f32 = 12.0;
    /// Compact icon (tool rows, tabs).
    pub const ICON_MD: f32 = 14.0;
    /// Default icon.
    pub const ICON: f32 = 16.0;
    /// Large icon.
    pub const ICON_LG: f32 = 20.0;
    /// Extra small control.
    pub const CONTROL_XS: f32 = 20.0;
    /// Small control.
    pub const CONTROL_SM: f32 = 24.0;
    /// Default button.
    pub const CONTROL: f32 = 28.0;
    /// Large control, menu rows, sidebar rows.
    pub const CONTROL_LG: f32 = 32.0;
    /// Form inputs.
    pub const INPUT: f32 = 36.0;
    /// Sidebar default / minimum width.
    pub const SIDEBAR: f32 = 264.0;
    /// Side pane default width.
    pub const SIDE_PANE: f32 = 440.0;
    /// Side pane minimum width.
    pub const SIDE_PANE_MIN: f32 = 320.0;
    /// Bottom frame default height.
    pub const BOTTOM: f32 = 240.0;
    /// Conversation reading column.
    pub const READING: f32 = 896.0;
    /// User bubble max width.
    pub const BUBBLE: f32 = 576.0;
    /// Header height.
    pub const HEADER: f32 = 44.0;
    /// Tab bar height.
    pub const TAB_BAR: f32 = 36.0;
    /// Meta row height.
    pub const META_ROW: f32 = 24.0;
    /// Tool row height.
    pub const TOOL_ROW: f32 = 24.0;
    /// Sidebar footer height.
    pub const SIDEBAR_FOOTER: f32 = 40.0;
    /// Minimum conversation width before the side pane collapses.
    pub const CONVERSATION_MIN: f32 = 480.0;
    /// Small dialog width.
    pub const DIALOG_SM: f32 = 420.0;
    /// Medium dialog width.
    pub const DIALOG_MD: f32 = 560.0;
    /// Large dialog width.
    pub const DIALOG_LG: f32 = 760.0;
    /// Palette width.
    pub const PALETTE: f32 = 640.0;
    /// Model menu width.
    pub const MODEL_MENU: f32 = 280.0;
    /// Menu min width.
    pub const MENU_MIN: f32 = 192.0;
    /// Toast width.
    pub const TOAST: f32 = 360.0;
    /// Status panel width.
    pub const STATUS_PANEL: f32 = 300.0;
    /// Query rail width.
    pub const QUERY_RAIL: f32 = 16.0;
    /// Settings nav width.
    pub const SETTINGS_NAV: f32 = 220.0;
    /// Settings content max width.
    pub const SETTINGS_CONTENT: f32 = 720.0;
    /// Kanban lane width.
    pub const LANE: f32 = 280.0;
    /// Composer max width on the welcome screen.
    pub const WELCOME_COMPOSER: f32 = 720.0;
    /// Window outer inset (Linux / macOS).
    pub const WINDOW_INSET: f32 = 4.0;
    /// Window outer inset on Windows.
    pub const WINDOW_INSET_WINDOWS: f32 = 6.0;
    /// Gap between frames (resize hit area).
    pub const FRAME_GAP: f32 = 4.0;
    /// 1 px hairline.
    pub const HAIRLINE: f32 = 1.0;
    /// Rail of provenance cards.
    pub const RAIL: f32 = 3.0;
    /// Status dot.
    pub const DOT: f32 = 6.0;
    /// Badge height.
    pub const BADGE: f32 = 18.0;
    /// Status capsule height.
    pub const CAPSULE: f32 = 22.0;
    /// Composer token chip height.
    pub const TOKEN: f32 = 22.0;
    /// Option rows in docked cards.
    pub const OPTION_ROW: f32 = 36.0;
    /// Settings row minimum height.
    pub const SETTING_ROW: f32 = 48.0;
    /// Context ring diameter.
    pub const RING: f32 = 18.0;
    /// Inline-diff / bash detail max height.
    pub const DETAIL_MAX: f32 = 320.0;
    /// Approval detail max height.
    pub const APPROVAL_DETAIL_MAX: f32 = 240.0;
    /// Suggestion card width on the welcome screen.
    pub const SUGGESTION: f32 = 232.0;
    /// Mini status capsule height.
    pub const MINI_STATUS: f32 = 28.0;
    /// Find bar width.
    pub const FIND_BAR: f32 = 360.0;
    /// Completion menu width.
    pub const COMPLETION: f32 = 360.0;
    /// Thinking bar cell (4 × 10).
    pub const THINK_BAR: (f32, f32) = (4.0, 10.0);
    /// Switch (28 × 16).
    pub const SWITCH: (f32, f32) = (28.0, 16.0);
    /// Checkbox / radio.
    pub const CHECK: f32 = 16.0;
    /// Unicrab sizes: glyph, empty state, splash.
    pub const CRAB: (f32, f32, f32) = (16.0, 64.0, 128.0);
    /// Empty-state icon tile.
    pub const EMPTY_TILE: f32 = 40.0;
    /// Palette top offset.
    pub const PALETTE_TOP: f32 = 80.0;
    /// Palette max height.
    pub const PALETTE_MAX_H: f32 = 560.0;
    /// Overlay dialog max height.
    pub const DIALOG_MAX_H: f32 = 680.0;
    /// Query rail bar (12 × 2).
    pub const RAIL_BAR: (f32, f32) = (12.0, 2.0);
    /// Traffic-light safe area (macOS).
    pub const TRAFFIC_LIGHTS: f32 = 78.0;
    /// List overdraw for the virtualised timeline.
    pub const OVERDRAW: f32 = 1200.0;
    /// Thumbnail in the user bubble.
    pub const THUMB: f32 = 48.0;
}

/// Window-width breakpoints (design.md §3.2).
pub mod breakpoint {
    /// Status panel expanded.
    pub const WIDE: f32 = 1280.0;
    /// Status panel mini capsule.
    pub const MEDIUM: f32 = 864.0;
    /// Side pane auto-collapse threshold.
    pub const NARROW: f32 = 720.0;
}

/// Motion durations in milliseconds (design.md §2.10).
pub mod motion {
    /// Hover / press feedback.
    pub const FAST_MS: u64 = 120;
    /// Menu / popover / dialog enter.
    pub const NORMAL_MS: u64 = 180;
    /// Sheet enter, status panel expand.
    pub const SLOW_MS: u64 = 280;
    /// Shared spinner ticker period (12 fps).
    pub const TICK_MS: u64 = 83;
    /// Running shimmer sweep.
    pub const SHIMMER_MS: u64 = 1600;
    /// Caret blink.
    pub const CARET_MS: u64 = 530;
}

/// One box shadow layer `(offset_y, blur, spread, color)`.
pub type ShadowLayer = (f32, f32, f32, u32);

/// Elevation E2 (menus, popovers, dialogs, docked cards) for light and dark.
pub const E2: ([ShadowLayer; 2], [ShadowLayer; 2]) = (
    [
        (4.0, 12.0, -2.0, 0x0000_001F),
        (2.0, 4.0, -2.0, 0x0000_0014),
    ],
    [
        (4.0, 12.0, -2.0, 0x0000_0066),
        (2.0, 4.0, -2.0, 0x0000_004D),
    ],
);

/// Elevation E3 (toasts, drag previews) for light and dark.
pub const E3: (ShadowLayer, ShadowLayer) = (
    (12.0, 32.0, -4.0, 0x0000_0029),
    (12.0, 32.0, -4.0, 0x0000_0080),
);

/// Unicrab sprite colors (unipi-app DESIGN §1.1), `0xRRGGBBAA`.
pub mod crab {
    /// Shell.
    pub const SHELL: u32 = 0xF030_18FF;
    /// Claw.
    pub const CLAW: u32 = 0xF078_18FF;
    /// Amber.
    pub const AMBER: u32 = 0xF0A8_18FF;
    /// Gold.
    pub const GOLD: u32 = 0xF0C0_30FF;
    /// Cream.
    pub const CREAM: u32 = 0xF0F0_A8FF;
    /// Face / outline.
    pub const FACE: u32 = 0x1818_18FF;
}

/// Splits `0xRRGGBBAA` into channels.
#[must_use]
pub const fn channels(c: u32) -> [u8; 4] {
    c.to_be_bytes()
}

/// Composites `top` over `bottom` (both `0xRRGGBBAA`; the result is opaque when `bottom` is).
#[must_use]
pub fn over(top: u32, bottom: u32) -> u32 {
    let [tr, tg, tb, ta] = channels(top);
    let [br, bg, bb, ba] = channels(bottom);
    let a = f32::from(ta) / 255.0;
    let mix = |t: u8, b: u8| -> u8 {
        let v = f32::from(t) * a + f32::from(b) * (1.0 - a);
        // Rounded and clamped to the channel range.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value clamped to 0..=255"
        )]
        let out = v.round().clamp(0.0, 255.0) as u8;
        out
    };
    let out_a = ta.max(ba);
    u32::from_be_bytes([mix(tr, br), mix(tg, bg), mix(tb, bb), out_a])
}

fn linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an opaque color.
#[must_use]
pub fn luminance(c: u32) -> f64 {
    let [r, g, b, _] = channels(c);
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG contrast ratio of `fg` (composited over `bg`) against `bg` (opaque).
#[must_use]
pub fn contrast(fg: u32, bg: u32) -> f64 {
    let fg = over(fg, bg);
    let (a, b) = (luminance(fg), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}
