//! Design tokens (design.md §2 as amended by A1.2): the Oino Dark palette (unipi warm OLED,
//! shell primary, wordmark semantics) and the Oino Light palette derived from the same hues,
//! spacing, radii, sizes, elevation and motion.
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
    pub primary_hover: u32,
    pub primary_fg: u32,
    pub secondary: u32,
    pub brand: u32,
    pub accent: u32,
    // unipi brand set (A1.2)
    pub focus: u32,
    pub working: u32,
    pub amber: u32,
    pub gold: u32,
    pub cream: u32,
    pub deep: u32,
    pub glow: u32,
    pub info: u32,
    pub info_surface: u32,
    pub info_surface_fg: u32,
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
    // unipi look (design.md A2.2)
    pub sheet: u32,
    pub chip: u32,
    pub bubble: u32,
    pub line_soft: u32,
    pub row_divider: u32,
    pub fg_strong: u32,
    pub list_marker: u32,
    pub link: u32,
    pub selected_border: u32,
    pub selected_border_strong: u32,
    pub chip_active: u32,
    pub chip_active_border: u32,
    pub attention_surface: u32,
    pub attention_border: u32,
    pub attention_fg: u32,
    pub scrollbar_thumb: u32,
    pub scrollbar_thumb_hover: u32,
    pub diff_added_fg: u32,
    pub diff_removed_fg: u32,
    pub ansi: [u32; 16],
    pub categorical: [u32; 12],
}

/// Oino Light: derived from the unipi hues (design.md A1.2) on warm off-white surfaces; same
/// shell primary and wordmark semantics, darkened where text needs 4.5:1.
pub const LIGHT: Palette = Palette {
    window: 0xFAF7_F2FF,
    sidebar: 0xF2ED_E6B8,
    frame: 0xFBF8_F4FF,
    header: 0xFBF8_F4FF,
    surface: 0x2A1F_1A0A,
    surface_hover: 0x2A1F_1A14,
    card: 0xFFFD_FAFF,
    card_border: 0xE4DC_D3FF,
    card_selected: 0xFFFD_FAFF,
    popover: 0xFFFD_FAFF,
    popover_border: 0xE4DC_D3FF,
    menu: 0xFFFD_FAFF,
    menu_hover: 0xF2ED_E6FF,
    input: 0xFFFD_FAFF,
    input_border: 0xE4DC_D3FF,
    input_border_hover: 0xD3C8_BDFF,
    input_border_focus: 0xF030_18FF,
    border: 0xE4DC_D3FF,
    border_hover: 0xD3C8_BDFF,
    hover: 0xF2ED_E6FF,
    selected: 0xF030_1821,
    tab: 0xF2ED_E6FF,
    tab_active: 0xFBF8_F4FF,
    tag: 0xEAE3_DAFF,
    tooltip: 0x1C19_17FF,
    tooltip_fg: 0xF5EF_E6FF,
    toast: 0xFFFD_FAF2,
    scrim: 0x1C19_174D,
    resize_handle: 0x2A1F_1A33,
    fg: 0x1C19_17FF,
    fg_subtle: 0x5E55_4DFF,
    fg_subtlest: 0x8A80_78FF,
    fg_inverse: 0xFFFF_FFFF,
    primary: 0xF030_18FF,
    primary_hover: 0xF018_18FF,
    primary_fg: 0x1818_18FF,
    secondary: 0xEAE3_DAFF,
    brand: 0xF030_18FF,
    accent: 0xF030_181A,
    focus: 0xF030_18FF,
    working: 0xD060_10FF,
    amber: 0xF0A8_18FF,
    gold: 0xC89A_10FF,
    cream: 0xF0F0_A8FF,
    deep: 0xC000_18FF,
    glow: 0xF030_1814,
    info: 0x1A8A_C0FF,
    info_surface: 0x28BE_F01F,
    info_surface_fg: 0x0E6A_94FF,
    success: 0x2E8B_2EFF,
    success_surface: 0x5AD2_5A26,
    success_surface_fg: 0x1F6B_1FFF,
    warning: 0xA878_00FF,
    warning_surface: 0xFAC8_2833,
    warning_surface_fg: 0x7A56_00FF,
    danger: 0xD42A_20FF,
    danger_surface: 0xF03C_321A,
    danger_surface_fg: 0xA81E_16FF,
    progress_surface: 0xF078_181F,
    progress_surface_fg: 0xA345_00FF,
    waiting_surface: 0xF030_181F,
    waiting_fg: 0xB824_10FF,
    idle_task: 0x5E55_4DFF,
    idle_task_surface: 0x2A1F_1A0D,
    diff_added: 0x2E8B_2EFF,
    diff_removed: 0xD42A_20FF,
    diff_added_bg: 0x5AD2_5A1F,
    diff_removed_bg: 0xF03C_321A,
    git_modified: 0xA878_00FF,
    git_added: 0x2E8B_2EFF,
    git_deleted: 0xD42A_20FF,
    git_renamed: 0x1A8A_C0FF,
    find: 0xFDEF_C2FF,
    find_active: 0xF0A8_18FF,
    file_node: 0x28BE_F01F,
    file_node_fg: 0x0E6A_94FF,
    skill_node: 0x7453_B01A,
    skill_node_fg: 0x7453_B0FF,
    command_node: 0x5E55_4D1A,
    command_node_fg: 0x5E55_4DFF,
    agent_node: 0xF078_181F,
    agent_node_fg: 0xA345_00FF,
    session_node: 0x1480_7A1A,
    session_node_fg: 0x1480_7AFF,
    harness: 0x7453_B0FF,
    harness_surface: 0x7453_B014,
    thinking: 0x5E55_4DFF,
    selection: 0xF0C0_3059,
    sheet: 0xFAF7_F2FF,
    chip: 0xF2ED_E6FF,
    bubble: 0xE9E2_D9FF,
    line_soft: 0xDDD4_C9B3,
    row_divider: 0xDDD4_C999,
    fg_strong: 0x1410_0EFF,
    list_marker: 0xD060_10FF,
    link: 0x0E6A_94FF,
    selected_border: 0xF030_1838,
    selected_border_strong: 0xF030_1840,
    chip_active: 0xF030_1824,
    chip_active_border: 0xF030_1861,
    attention_surface: 0xF0A8_181F,
    attention_border: 0xC080_10B3,
    attention_fg: 0x8A5A_00FF,
    scrollbar_thumb: 0x8A80_7873,
    scrollbar_thumb_hover: 0x8A80_78BF,
    diff_added_fg: 0x1F6B_1FFF,
    diff_removed_fg: 0xA81E_16FF,
    ansi: [
        0x5E55_4DFF,
        0xD42A_20FF,
        0x2E8B_2EFF,
        0xA878_00FF,
        0x1A8A_C0FF,
        0x9B4F_C0FF,
        0x138A_8AFF,
        0xA39A_90FF,
        0x8A80_78FF,
        0xF03C_32FF,
        0x3FA8_3FFF,
        0xC894_00FF,
        0x1A8A_C0FF,
        0xB36A_D6FF,
        0x1AA7_A7FF,
        0x1C19_17FF,
    ],
    categorical: [
        0x1A8A_C0FF,
        0x2E8B_2EFF,
        0xD060_10FF,
        0xB88A_10FF,
        0x9B4F_C0FF,
        0x138A_8AFF,
        0xD42A_20FF,
        0xC07A_00FF,
        0x1A70_B8FF,
        0x7453_B0FF,
        0x1480_7AFF,
        0xB87A_1AFF,
    ],
};

/// Oino Dark (default): the unipi warm OLED palette (unipi-app DESIGN §1.1–§1.3, design.md
/// A1.2). `bg #0A0908` frames on `surface-1` chrome, shell primary, wordmark semantics.
pub const DARK: Palette = Palette {
    window: 0x0A09_08FF,
    sidebar: 0x1412_11B8,
    frame: 0x0A09_08FF,
    header: 0x0A09_08FF,
    surface: 0xF5EF_E60D,
    surface_hover: 0xF5EF_E61A,
    card: 0x1412_11FF,
    card_border: 0x2E28_25FF,
    card_selected: 0x1C19_17FF,
    popover: 0x1C19_17FF,
    popover_border: 0x2E28_25FF,
    menu: 0x1C19_17FF,
    menu_hover: 0x2621_1FFF,
    input: 0x1C19_17FF,
    input_border: 0x2E28_25FF,
    input_border_hover: 0x3D35_31FF,
    input_border_focus: 0xF030_18FF,
    border: 0x2E28_25FF,
    border_hover: 0x3D35_31FF,
    hover: 0x1C19_17FF,
    selected: 0xF030_1821,
    tab: 0x1412_11FF,
    tab_active: 0x0A09_08FF,
    tag: 0x2621_1FFF,
    tooltip: 0x2621_1FFF,
    tooltip_fg: 0xF5EF_E6FF,
    toast: 0x1C19_17F2,
    scrim: 0x0000_0073,
    resize_handle: 0xF5EF_E633,
    fg: 0xF5EF_E6FF,
    fg_subtle: 0xA39A_90FF,
    fg_subtlest: 0x6E65_5DFF,
    fg_inverse: 0x1818_18FF,
    primary: 0xF030_18FF,
    primary_hover: 0xF018_18FF,
    primary_fg: 0x1818_18FF,
    secondary: 0x2621_1FFF,
    brand: 0xF030_18FF,
    accent: 0xF030_1826,
    focus: 0xF030_18FF,
    working: 0xF078_18FF,
    amber: 0xF0A8_18FF,
    gold: 0xF0C0_30FF,
    cream: 0xF0F0_A8FF,
    deep: 0xC000_18FF,
    glow: 0xF030_180D,
    info: 0x28BE_F0FF,
    info_surface: 0x28BE_F029,
    info_surface_fg: 0x7DD6_F5FF,
    success: 0x5AD2_5AFF,
    success_surface: 0x5AD2_5A29,
    success_surface_fg: 0x8FE0_8FFF,
    warning: 0xFAC8_28FF,
    warning_surface: 0xFAC8_2829,
    warning_surface_fg: 0xFBD8_66FF,
    danger: 0xF03C_32FF,
    danger_surface: 0xF03C_3229,
    danger_surface_fg: 0xF6A3_9DFF,
    progress_surface: 0xF078_1829,
    progress_surface_fg: 0xF5A0_65FF,
    waiting_surface: 0xF030_1833,
    waiting_fg: 0xFF80_68FF,
    idle_task: 0xA39A_90FF,
    idle_task_surface: 0xA39A_901F,
    diff_added: 0x5AD2_5AFF,
    diff_removed: 0xF03C_32FF,
    diff_added_bg: 0x5AD2_5A1A,
    diff_removed_bg: 0xF03C_321C,
    git_modified: 0xFAC8_28FF,
    git_added: 0x5AD2_5AFF,
    git_deleted: 0xF03C_32FF,
    git_renamed: 0x28BE_F0FF,
    find: 0x5A3A_08FF,
    find_active: 0xF0A8_18FF,
    file_node: 0x28BE_F029,
    file_node_fg: 0x7DD6_F5FF,
    skill_node: 0xA689_DA29,
    skill_node_fg: 0xBDA5_E6FF,
    command_node: 0xA39A_9024,
    command_node_fg: 0xC9C1_B8FF,
    agent_node: 0xF078_1829,
    agent_node_fg: 0xF5A0_65FF,
    session_node: 0x68C4_BC29,
    session_node_fg: 0x93D8_D2FF,
    harness: 0xBDA5_E6FF,
    harness_surface: 0xA689_DA1F,
    thinking: 0xA39A_90FF,
    selection: 0xF0C0_3040,
    sheet: 0x1412_11FF,
    chip: 0x1C19_17FF,
    bubble: 0x2621_1FFF,
    line_soft: 0x2E28_25B3,
    row_divider: 0x2E28_2599,
    fg_strong: 0xFFF8_ECFF,
    list_marker: 0xF078_18FF,
    link: 0x28BE_F0FF,
    selected_border: 0xF030_1838,
    selected_border_strong: 0xF030_1840,
    chip_active: 0xF030_1824,
    chip_active_border: 0xF030_1861,
    attention_surface: 0xF0A8_1814,
    attention_border: 0xF0A8_1899,
    attention_fg: 0xF0A8_18FF,
    scrollbar_thumb: 0x6E65_5D73,
    scrollbar_thumb_hover: 0x6E65_5DBF,
    diff_added_fg: 0xB5E8_B5FF,
    diff_removed_fg: 0xF3B3_ADFF,
    ansi: [
        0x2621_1FFF,
        0xF03C_32FF,
        0x5AD2_5AFF,
        0xFAC8_28FF,
        0x28BE_F0FF,
        0xC678_DDFF,
        0x56D4_DDFF,
        0xA39A_90FF,
        0x6E65_5DFF,
        0xFF6E_64FF,
        0x8BE0_8BFF,
        0xFFD9_5CFF,
        0x6CD3_F5FF,
        0xD79B_E8FF,
        0x8EE5_E5FF,
        0xF5EF_E6FF,
    ],
    categorical: [
        0x28BE_F0FF,
        0x5AD2_5AFF,
        0xF078_18FF,
        0xF0C0_30FF,
        0xC678_DDFF,
        0x56D4_DDFF,
        0xF03C_32FF,
        0xF0A8_18FF,
        0x8FC5_EFFF,
        0xBDA5_E6FF,
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
    /// 6 — button gap (design.md §4.1), unipi half step.
    pub const S1_5: f32 = 6.0;
    /// 10 — unipi half step (chat row gap, nav padding; A2.4).
    pub const S2_5: f32 = 10.0;
    /// 14 — unipi half step (row padding, dialog title padding; A2.4).
    pub const S3_5: f32 = 14.0;
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
    /// Cards, docked cards, status panel (unipi §1.4 / A1.2).
    pub const R_CARD: f32 = 16.0;
    /// Dialogs, menus, palette, popovers (unipi `--radius-float`).
    pub const R_DIALOG: f32 = 16.0;
    /// Inputs and text fields.
    pub const R_INPUT: f32 = 12.0;
    /// Sheets (top corners).
    pub const R_SHEET: f32 = 24.0;
    /// Buttons, chips and toolbar capsules are pill-shaped.
    pub const R_BUTTON: f32 = R_FULL;
    /// 10 — nav items, palette / menu rows, tool rows, search field, small tiles (A2.4).
    pub const R_ROW: f32 = 10.0;
    /// 14 — option rows and harness cards.
    pub const R_OPTION: f32 = 14.0;
    /// 14 — harness cards.
    pub const R_HARNESS: f32 = 14.0;
    /// 16 — floating surfaces (dialogs, menus, palette, toasts, tiles 52–56).
    pub const R_FLOAT: f32 = 16.0;
    /// 12 — tool bodies and code blocks.
    pub const R_TOOL: f32 = 12.0;
    /// 20 — user bubble.
    pub const R_BUBBLE: f32 = 20.0;
    /// 24 — interaction card in the composer slot.
    pub const R_DIALOG_CARD: f32 = 24.0;
    /// 26 — composer shell.
    pub const R_COMPOSER: f32 = 26.0;
    /// 5 — kbd chip.
    pub const R_KBD: f32 = 5.0;
    /// 6 — inline code.
    pub const R_CODE: f32 = 6.0;
    /// 7 — option number tile.
    pub const R_NUMBER: f32 = 7.0;
    /// 8 — settings icon tile, board avatar tile.
    pub const R_TILE_SM: f32 = 8.0;
}

/// Focus indicator (unipi §1.6: 2 px shell outline, offset 2).
pub mod focus {
    /// Outline width.
    pub const WIDTH: f32 = 2.0;
    /// Outline offset.
    pub const OFFSET: f32 = 2.0;
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
    /// Nav sidebar width (fixed, A2.4; ⌘B hides it).
    pub const SIDEBAR: f32 = 224.0;
    /// Side pane default width.
    pub const SIDE_PANE: f32 = 440.0;
    /// Side pane minimum width.
    pub const SIDE_PANE_MIN: f32 = 320.0;
    /// Bottom frame default height.
    pub const BOTTOM: f32 = 240.0;
    /// Conversation reading column (A2.4 chat column max).
    pub const READING: f32 = 940.0;
    /// User bubble max width.
    pub const BUBBLE: f32 = 576.0;
    /// Header height.
    pub const HEADER: f32 = 44.0;
    /// Tab bar height.
    pub const TAB_BAR: f32 = 36.0;
    /// Meta row height.
    pub const META_ROW: f32 = 24.0;
    /// Tool row / fold line minimum height (A2.4).
    pub const TOOL_ROW: f32 = 32.0;
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
    /// Palette width (A2.17).
    pub const PALETTE: f32 = 620.0;
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
    /// Settings nav width (A2.16).
    pub const SETTINGS_NAV: f32 = 200.0;
    /// Settings content max width (A2.16).
    pub const SETTINGS_CONTENT: f32 = 760.0;
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
    /// Option rows in interaction cards (A2.11).
    pub const OPTION_ROW: f32 = 46.0;
    /// Settings row minimum height (A2.16).
    pub const SETTING_ROW: f32 = 56.0;
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
    /// Automations: the state column (icon + word).
    pub const CRON_STATE: f32 = 96.0;
    /// Automations: the next/last column.
    pub const CRON_TIMES: f32 = 120.0;
    /// Automations: the run-history side column.
    pub const CRON_DETAIL: f32 = 420.0;
    /// Automations: label/status column widths in rows.
    pub const CRON_LABEL: f32 = 64.0;
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
    /// Overlay dialog max height (A2.12: min(80 %, 760)).
    pub const DIALOG_MAX_H: f32 = 760.0;
    /// Dialog max height as a fraction of the window.
    pub const DIALOG_MAX_FRACTION: f32 = 0.8;
    /// Query rail bar (12 × 2).
    pub const RAIL_BAR: (f32, f32) = (12.0, 2.0);
    /// Traffic-light safe area (macOS).
    pub const TRAFFIC_LIGHTS: f32 = 78.0;
    /// List overdraw for the virtualised timeline.
    pub const OVERDRAW: f32 = 1200.0;
    /// Thumbnail in the user bubble.
    pub const THUMB: f32 = 48.0;

    // ── unipi look (design.md A2.4) ────────────────────────────────────────
    /// Desktop page gutter.
    pub const PAGE_GUTTER: f32 = 20.0;
    /// Page header lead / tools row.
    pub const PAGE_HEAD_BAR: f32 = 44.0;
    /// Page header padding `(top, bottom)`.
    pub const PAGE_HEAD_PAD: (f32, f32) = (10.0, 16.0);
    /// Detail header height (chat, task, review, browser, file).
    pub const DETAIL_HEAD: f32 = 56.0;
    /// Round ghost icon button in headers.
    pub const ICON_BUTTON_LG: f32 = 40.0;
    /// Round ghost icon button in rows and bars.
    pub const ICON_BUTTON: f32 = 32.0;
    /// Sidebar brand row.
    pub const BRAND_ROW: f32 = 40.0;
    /// Brand crab.
    pub const CRAB_BRAND: f32 = 26.0;
    /// Sidebar search field.
    pub const SEARCH: f32 = 36.0;
    /// Nav item height.
    pub const NAV_ITEM: f32 = 38.0;
    /// Session list column default / min / max width.
    pub const LIST: f32 = 400.0;
    /// Session list column minimum width.
    pub const LIST_MIN: f32 = 280.0;
    /// Session list column maximum width.
    pub const LIST_MAX: f32 = 560.0;
    /// Splitter hit width (the drawn line is 2 px).
    pub const SPLITTER: f32 = 8.0;
    /// Grouped-card list row.
    pub const ROW: f32 = 64.0;
    /// Grouped-card list row when the column is narrow.
    pub const ROW_COMPACT: f32 = 56.0;
    /// Icon tile in list rows.
    pub const TILE_ROW: f32 = 36.0;
    /// Tool chip / pill and segmented track.
    pub const CHIP: f32 = 38.0;
    /// Segmented option.
    pub const SEGMENT_OPTION: f32 = 32.0;
    /// Button (dialog footers, empty states, page CTAs).
    pub const BUTTON: f32 = 44.0;
    /// Small button (toolbars, inline).
    pub const BUTTON_SM: f32 = 34.0;
    /// Kbd chip minimum size.
    pub const KBD: f32 = 20.0;
    /// Chat column padding.
    pub const READING_PAD: f32 = 28.0;
    /// Prose max width (≈ 76ch).
    pub const PROSE_MAX: f32 = 680.0;
    /// User bubble max width as a fraction of the row.
    pub const BUBBLE_FRACTION: f32 = 0.8;
    /// Composer max width.
    pub const COMPOSER_MAX: f32 = 884.0;
    /// Send circle.
    pub const SEND: f32 = 38.0;
    /// Attach circle.
    pub const ATTACH: f32 = 34.0;
    /// Mode chip height.
    pub const MODE_CHIP: f32 = 30.0;
    /// Context ring stroke.
    pub const RING_STROKE: f32 = 2.5;
    /// Fold body indent.
    pub const FOLD_INDENT: f32 = 12.0;
    /// Tool output max height.
    pub const TOOL_OUT_MAX: f32 = 240.0;
    /// Queue strip max height.
    pub const QUEUE_MAX: f32 = 152.0;
    /// Jump-to-latest circle.
    pub const JUMP: f32 = 38.0;
    /// Palette top offset as a fraction of the window height.
    pub const PALETTE_TOP_FRACTION: f32 = 0.14;
    /// Palette max height as a fraction of the window height.
    pub const PALETTE_MAX_FRACTION: f32 = 0.64;
    /// Palette search row.
    pub const PALETTE_INPUT: f32 = 52.0;
    /// Palette row.
    pub const PALETTE_ROW: f32 = 40.0;
    /// Anchored menu row.
    pub const MENU_ROW: f32 = 40.0;
    /// Settings icon tile.
    pub const SETTING_TILE: f32 = 28.0;
    /// Inset of row dividers in grouped cards (after the icon tile).
    pub const ROW_DIVIDER_INSET: f32 = 58.0;
    /// Switch (44 × 26).
    pub const SWITCH_LG: (f32, f32) = (44.0, 26.0);
    /// Text field height (settings).
    pub const FIELD: f32 = 40.0;
    /// Empty-state top offset.
    pub const EMPTY_TOP: f32 = 64.0;
    /// Empty-state crab.
    pub const CRAB_EMPTY: f32 = 88.0;
    /// Compact empty-state crab.
    pub const CRAB_COMPACT: f32 = 56.0;
    /// Empty-state body max width.
    pub const EMPTY_BODY: f32 = 340.0;
    /// Pane placeholder tile.
    pub const PLACEHOLDER_TILE: f32 = 52.0;
    /// Header crab.
    pub const CRAB_HEADER: f32 = 28.0;
    /// Header crab badge.
    pub const CRAB_BADGE: f32 = 16.0;
    /// Needs-you strip minimum height.
    pub const STRIP: f32 = 56.0;
    /// Onboarding column max width.
    pub const ONBOARDING: f32 = 560.0;
    /// Board FAB.
    pub const FAB: f32 = 52.0;
    /// Toast max width (A2.12).
    pub const TOAST_MAX: f32 = 420.0;
    /// Number tile in numbered rows.
    pub const NUMBER_TILE: f32 = 22.0;
    /// Minimum width of a dialog footer action.
    pub const DIALOG_ACTION_MIN: f32 = 96.0;
    /// The desktop glow quad (A2.2: 520 px top-left).
    pub const GLOW: f32 = 520.0;
    /// Session strip pill height in the detail header.
    pub const STRIP_PILL: f32 = 32.0;
}

/// Touch layout (design.md §6, Oino Mobile; read through [`crate::design::Density::Touch`]).
pub mod touch {
    /// App bar / page detail header height.
    pub const APP_BAR: f32 = 52.0;
    /// Minimum touch target.
    pub const TARGET: f32 = 44.0;
    /// List rows (sessions, sheets, settings, search).
    pub const ROW: f32 = 56.0;
    /// Timeline tool rows.
    pub const TOOL_ROW: f32 = 32.0;
    /// Floating "New session" pill height.
    pub const FAB: f32 = 48.0;
    /// Right drawer width (fraction of the window).
    pub const DRAWER: f32 = 0.88;
    /// Bottom sheet max height (fraction of the visible height).
    pub const SHEET_MAX: f32 = 0.9;
    /// Sheet drag handle (36 × 4).
    pub const HANDLE: (f32, f32) = (36.0, 4.0);
    /// Page / timeline gutters (A2.4).
    pub const GUTTER: f32 = 16.0;
    /// User bubble max width (fraction of the row).
    pub const BUBBLE: f32 = 0.85;
    /// Editable text size (composer, fields).
    pub const EDIT_FONT: f32 = 16.0;
    /// Width from which phones in landscape and tablets use the desktop shell.
    pub const TABLET: f32 = 720.0;
    /// Width of one swipe action behind a row.
    pub const SWIPE_ACTION: f32 = 76.0;
    /// Horizontal travel before a press becomes a swipe.
    pub const SWIPE_SLOP: f32 = 12.0;
    /// Pull distance that triggers refresh.
    pub const PULL_REFRESH: f32 = 72.0;
    /// Long-press delay in milliseconds.
    pub const LONG_PRESS_MS: u64 = 450;
    /// Board lane pill height.
    pub const PILL: f32 = 32.0;
    /// Thinking level segment height in the model sheet.
    pub const SEGMENT: f32 = 40.0;
    /// App bar crab glyph.
    pub const CRAB: f32 = 28.0;
    /// Status dot in rows.
    pub const DOT: f32 = 8.0;
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
    /// Dialog fade + scale (0.97 → 1).
    pub const DIALOG_MS: u64 = 160;
    /// Palette enter (scale 0.98 → 1).
    pub const PALETTE_MS: u64 = 140;
    /// Detail view slide-in (16 px + fade).
    pub const PANE_MS: u64 = 180;
    /// Fold chevron rotation.
    pub const CHEVRON_MS: u64 = 160;
    /// Status dot pulse period.
    pub const PULSE_MS: u64 = 1200;
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

/// unipi elevation (A2.4), `(dark, light)` single layers.
pub mod elevation {
    use super::ShadowLayer;
    /// Composer shell `0 -8 30`.
    pub const COMPOSER: (ShadowLayer, ShadowLayer) = (
        (-8.0, 30.0, 0.0, 0x0000_0066),
        (-8.0, 30.0, 0.0, 0x0000_0014),
    );
    /// Dialogs `0 24 80`.
    pub const DIALOG: (ShadowLayer, ShadowLayer) = (
        (24.0, 80.0, 0.0, 0x0000_0099),
        (24.0, 80.0, 0.0, 0x0000_0029),
    );
    /// Palette `0 30 90`.
    pub const PALETTE: (ShadowLayer, ShadowLayer) = (
        (30.0, 90.0, 0.0, 0x0000_00A6),
        (30.0, 90.0, 0.0, 0x0000_002E),
    );
    /// Menus and popovers `0 12 32`.
    pub const MENU: (ShadowLayer, ShadowLayer) = (
        (12.0, 32.0, 0.0, 0x0000_0080),
        (12.0, 32.0, 0.0, 0x0000_001F),
    );
    /// Jump-to-latest and FAB `0 6 20`.
    pub const FAB: (ShadowLayer, ShadowLayer) =
        ((6.0, 20.0, 0.0, 0x0000_0080), (6.0, 20.0, 0.0, 0x0000_001F));
    /// Toasts and the mascot bubble `0 8 26`.
    pub const TOAST: (ShadowLayer, ShadowLayer) =
        ((8.0, 26.0, 0.0, 0x0000_0080), (8.0, 26.0, 0.0, 0x0000_001F));
}

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
