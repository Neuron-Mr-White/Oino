//! The installed theme (design.md §2.3): the [`OinoTheme`] global (palette, type sizes,
//! reduce motion) and the gpui-kit `Theme` derived from it so stock components match.
//!
//! Views read colors through [`OinoTheme`] accessors (`t.fg()`, `t.frame()`…), which convert
//! the `0xRRGGBBAA` token on the fly (a few float ops, no allocation).

use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{
    App, BoxShadow, FontWeight, Global, Hsla, Pixels, SharedString, Styled, Window,
    WindowAppearance, point, px, rgba,
};

use crate::design::tokens::{self, DARK, LIGHT, Palette, ShadowLayer};
use crate::design::type_scale::{self, MONO_FONT, TextRole, UI_FONT};
use crate::host::{AppSettings, ThemePref};
use crate::model::ous::Tone;

/// `0xRRGGBBAA` → GPUI color.
#[must_use]
pub fn color(c: u32) -> Hsla {
    rgba(c).into()
}

/// Layout density (design-mobile consumes `Touch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    /// Desktop rows and targets.
    #[default]
    Desktop,
    /// Touch targets (rows 56, tool rows 32, targets 44).
    Touch,
}

/// The installed Oino theme (a GPUI global).
#[derive(Clone, Copy, Debug)]
pub struct OinoTheme {
    /// Dark palette in use.
    pub dark: bool,
    /// Palette.
    pub p: &'static Palette,
    /// UI font size.
    pub ui: f32,
    /// Code font size.
    pub code: f32,
    /// Durations are 0 and spinners are static.
    pub reduce_motion: bool,
    /// Layout density.
    pub density: Density,
}

impl Global for OinoTheme {}

impl Default for OinoTheme {
    fn default() -> Self {
        Self {
            dark: true,
            p: &DARK,
            ui: type_scale::UI_FONT_SIZE,
            code: type_scale::CODE_FONT_SIZE,
            reduce_motion: false,
            density: Density::Desktop,
        }
    }
}

macro_rules! tokens {
    ($($name:ident),* $(,)?) => {
        $(
            #[doc = concat!("`", stringify!($name), "` token.")]
            #[must_use]
            pub fn $name(&self) -> Hsla {
                color(self.p.$name)
            }
        )*
    };
}

impl OinoTheme {
    /// The installed theme (defaults before [`install`]).
    #[must_use]
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    tokens!(
        window,
        frame,
        header,
        surface,
        surface_hover,
        card,
        card_border,
        popover,
        popover_border,
        menu,
        menu_hover,
        input,
        input_border,
        input_border_hover,
        input_border_focus,
        border,
        border_hover,
        hover,
        selected,
        tab,
        tab_active,
        tag,
        tooltip,
        tooltip_fg,
        toast,
        scrim,
        resize_handle,
        fg,
        fg_subtle,
        fg_subtlest,
        fg_inverse,
        primary,
        primary_hover,
        primary_fg,
        secondary,
        brand,
        accent,
        focus,
        working,
        amber,
        gold,
        cream,
        deep,
        glow,
        info,
        info_surface,
        info_surface_fg,
        success,
        success_surface,
        success_surface_fg,
        warning,
        warning_surface,
        warning_surface_fg,
        danger,
        danger_surface,
        danger_surface_fg,
        progress_surface,
        progress_surface_fg,
        waiting_surface,
        waiting_fg,
        idle_task,
        idle_task_surface,
        diff_added,
        diff_removed,
        diff_added_bg,
        diff_removed_bg,
        git_modified,
        git_added,
        git_deleted,
        git_renamed,
        find,
        find_active,
        file_node,
        file_node_fg,
        skill_node,
        skill_node_fg,
        command_node,
        command_node_fg,
        agent_node,
        agent_node_fg,
        session_node,
        session_node_fg,
        harness,
        harness_surface,
        thinking,
        sidebar,
        selection,
        sheet,
        chip,
        bubble,
        line_soft,
        row_divider,
        fg_strong,
        list_marker,
        link,
        selected_border,
        selected_border_strong,
        chip_active,
        chip_active_border,
        attention_surface,
        attention_border,
        attention_fg,
        scrollbar_thumb,
        scrollbar_thumb_hover,
        diff_added_fg,
        diff_removed_fg,
    );

    /// ANSI color `i` (0–15).
    #[must_use]
    pub fn ansi(&self, i: usize) -> Hsla {
        color(self.p.ansi[i % 16])
    }

    /// Categorical color `i` (0–11).
    #[must_use]
    pub fn categorical(&self, i: usize) -> Hsla {
        color(self.p.categorical[i % 12])
    }

    /// Text color of an OUS tone (design.md §2.2, `_fg` column).
    #[must_use]
    pub fn tone_fg(&self, tone: Tone) -> Hsla {
        color(crate::design::tones::fg(tone, self.p))
    }

    /// Rail / bar / dot color of an OUS tone.
    #[must_use]
    pub fn tone_rail(&self, tone: Tone) -> Hsla {
        color(crate::design::tones::rail(tone, self.p))
    }

    /// Badge background of an OUS tone.
    #[must_use]
    pub fn tone_surface(&self, tone: Tone) -> Hsla {
        color(crate::design::tones::surface(tone, self.p))
    }

    /// `(font size, line height)` of a text role.
    #[must_use]
    pub fn text(&self, role: TextRole) -> (Pixels, Pixels) {
        let (s, l) = type_scale::size(role, self.ui);
        (px(s), px(l))
    }

    /// Code font size and line height.
    #[must_use]
    pub fn code_text(&self) -> (Pixels, Pixels) {
        (px(self.code), px((self.code * 1.5).round()))
    }

    /// UI font family.
    #[must_use]
    pub fn ui_font(&self) -> SharedString {
        SharedString::new_static(UI_FONT)
    }

    /// Monospace font family.
    #[must_use]
    pub fn mono_font(&self) -> SharedString {
        SharedString::new_static(MONO_FONT)
    }

    fn shadow(layer: ShadowLayer) -> BoxShadow {
        let (y, blur, spread, c) = layer;
        BoxShadow {
            color: color(c),
            offset: point(px(0.), px(y)),
            blur_radius: px(blur),
            spread_radius: px(spread),
            inset: false,
        }
    }

    /// Elevation E2 (menus, popovers, dialogs, docked cards, status panel).
    #[must_use]
    pub fn e2(&self) -> Vec<BoxShadow> {
        let layers = if self.dark {
            tokens::E2.1
        } else {
            tokens::E2.0
        };
        layers.into_iter().map(Self::shadow).collect()
    }

    /// Elevation E3 (toasts).
    #[must_use]
    pub fn e3(&self) -> Vec<BoxShadow> {
        vec![Self::shadow(if self.dark {
            tokens::E3.1
        } else {
            tokens::E3.0
        })]
    }

    fn pick(&self, layer: (ShadowLayer, ShadowLayer)) -> Vec<BoxShadow> {
        vec![Self::shadow(if self.dark { layer.0 } else { layer.1 })]
    }

    /// Dialog elevation (A2.4 `0 24 80`).
    #[must_use]
    pub fn e_dialog(&self) -> Vec<BoxShadow> {
        self.pick(tokens::elevation::DIALOG)
    }

    /// Palette elevation (A2.4 `0 30 90`).
    #[must_use]
    pub fn e_palette(&self) -> Vec<BoxShadow> {
        self.pick(tokens::elevation::PALETTE)
    }

    /// Menu / popover elevation (A2.4 `0 12 32`).
    #[must_use]
    pub fn e_menu(&self) -> Vec<BoxShadow> {
        self.pick(tokens::elevation::MENU)
    }

    /// Composer elevation (A2.4 `0 -8 30`).
    #[must_use]
    pub fn e_composer(&self) -> Vec<BoxShadow> {
        self.pick(tokens::elevation::COMPOSER)
    }

    /// Jump-to-latest / FAB elevation (A2.4 `0 6 20`).
    #[must_use]
    pub fn e_fab(&self) -> Vec<BoxShadow> {
        self.pick(tokens::elevation::FAB)
    }

    /// Toast / mascot bubble elevation (A2.4 `0 8 26`).
    #[must_use]
    pub fn e_toast(&self) -> Vec<BoxShadow> {
        self.pick(tokens::elevation::TOAST)
    }

    /// A tint of `c` at `alpha` (mode chips: 13 % bg, 45 % border; A2.2).
    #[must_use]
    pub fn tint(c: Hsla, alpha: f32) -> Hsla {
        c.opacity(alpha)
    }

    /// Touch density is active (Oino Mobile).
    #[must_use]
    pub fn touch(&self) -> bool {
        self.density == Density::Touch
    }

    /// Timeline tool row height (24 desktop, 32 touch).
    #[must_use]
    pub fn tool_row(&self) -> Pixels {
        px(if self.touch() {
            tokens::touch::TOOL_ROW
        } else {
            tokens::size::TOOL_ROW
        })
    }

    /// Option row height in docked cards and dialogs (36 desktop, 56 touch).
    #[must_use]
    pub fn option_row(&self) -> Pixels {
        px(if self.touch() {
            tokens::touch::ROW
        } else {
            tokens::size::OPTION_ROW
        })
    }

    /// Timeline / conversation gutter (16 desktop, 12 touch).
    #[must_use]
    pub fn gutter(&self) -> Pixels {
        px(if self.touch() {
            tokens::touch::GUTTER
        } else {
            tokens::space::S4
        })
    }

    /// Row height scaled for the density (touch rows are taller).
    #[must_use]
    pub fn row(&self, desktop: f32) -> Pixels {
        match self.density {
            Density::Desktop => px(desktop),
            Density::Touch => px(desktop.max(44.0)),
        }
    }
}

/// Text role and font helpers on any styled element.
pub trait TextExt: Styled + Sized {
    /// Applies a text role (size + line height).
    #[must_use]
    fn text_role(self, t: &OinoTheme, role: TextRole) -> Self {
        let (size, line) = t.text(role);
        self.text_size(size).line_height(line)
    }

    /// Monospace at the code font size.
    #[must_use]
    fn code_text(self, t: &OinoTheme) -> Self {
        let (size, line) = t.code_text();
        self.font_family(t.mono_font())
            .text_size(size)
            .line_height(line)
    }

    /// Monospace at a UI text role (paths, ids, kbd).
    #[must_use]
    fn mono_text(self, t: &OinoTheme, role: TextRole) -> Self {
        self.text_role(t, role).font_family(t.mono_font())
    }

    /// Medium weight (labels, titles).
    #[must_use]
    fn weight_medium(self) -> Self {
        self.font_weight(FontWeight::MEDIUM)
    }

    /// Semibold weight (Display, markdown headings).
    #[must_use]
    fn weight_semibold(self) -> Self {
        self.font_weight(FontWeight::SEMIBOLD)
    }
}

impl<E: Styled> TextExt for E {}

/// Fonts bundled with the app (design.md §2.4).
const FONTS: [&[u8]; 5] = [
    include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf"),
];

/// Registers the bundled Inter and `JetBrains` Mono faces.
pub fn register_fonts(cx: &App) {
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        tracing::warn!(%error, "registering bundled fonts");
    }
}

/// Whether `pref` resolves to the dark palette for `appearance`.
#[must_use]
pub fn resolve_dark(pref: ThemePref, appearance: Option<WindowAppearance>) -> bool {
    match pref {
        ThemePref::Light => false,
        ThemePref::Dark => true,
        ThemePref::System => !matches!(
            appearance,
            Some(WindowAppearance::Light | WindowAppearance::VibrantLight)
        ),
    }
}

/// Installs the Oino theme for `settings` and derives the gpui-kit theme from it.
pub fn install(settings: &AppSettings, window: Option<&mut Window>, cx: &mut App) {
    let appearance = window.as_ref().map(|w| w.appearance());
    let dark = resolve_dark(settings.theme, appearance);
    let density = cx
        .try_global::<OinoTheme>()
        .map_or(Density::Desktop, |t| t.density);
    let theme = OinoTheme {
        dark,
        p: if dark { &DARK } else { &LIGHT },
        ui: type_scale::clamp_ui(f32::from(settings.ui_font_size)),
        code: type_scale::clamp_code(f32::from(settings.code_font_size)),
        reduce_motion: settings.reduce_motion.resolve(),
        density,
    };
    cx.set_global(theme);
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        window,
        cx,
    );
    Theme::update(cx, |kit| derive(&theme, kit));
}

/// Sets the layout density (mobile shells call this with [`Density::Touch`]).
pub fn set_density(density: Density, cx: &mut App) {
    let mut theme = OinoTheme::get(cx);
    theme.density = density;
    cx.set_global(theme);
}

/// Maps the palette onto gpui-kit theme colors (design.md §2.3 table).
#[allow(clippy::too_many_lines, reason = "one assignment per gpui-kit color")]
fn derive(t: &OinoTheme, kit: &mut Theme) {
    let p = t.p;
    let c = color;
    let primary = c(p.primary);
    kit.font_family = t.ui_font();
    kit.mono_font_family = t.mono_font();
    kit.font_size = px(t.ui);
    kit.mono_font_size = px(t.code);
    kit.radius = px(tokens::radius::R_INPUT);
    kit.radius_lg = px(tokens::radius::R_CARD);
    kit.shadow = true;
    kit.focus_ring = false;
    let k = &mut kit.colors;
    k.background = c(p.frame);
    k.foreground = c(p.fg);
    k.muted = c(p.surface);
    k.muted_foreground = c(p.fg_subtle);
    k.secondary = c(p.secondary);
    k.secondary_foreground = c(p.fg);
    k.secondary_hover = c(p.border_hover);
    k.secondary_active = c(p.border_hover);
    k.border = c(p.border);
    // Fields have no border at rest; focus draws the shell ring (A2.5 / L11).
    k.input = c(p.input_border);
    k.ring = c(p.input_border_focus);
    k.primary = primary;
    k.primary_foreground = c(p.primary_fg);
    k.primary_hover = c(p.primary_hover);
    k.primary_active = c(p.primary_hover);
    k.button_primary = primary;
    k.button_primary_foreground = c(p.primary_fg);
    k.button_primary_hover = c(p.primary_hover);
    k.button_primary_active = c(p.primary_hover);
    k.accent = c(p.hover);
    k.accent_foreground = c(p.fg);
    k.popover = c(p.popover);
    k.popover_foreground = c(p.fg);
    k.list = c(p.frame);
    k.list_hover = c(p.hover);
    k.list_active = c(p.selected);
    k.list_active_border = c(p.selected_border);
    k.list_even = c(p.frame);
    k.list_head = c(p.frame);
    k.sidebar = c(p.sidebar);
    k.sidebar_foreground = c(p.fg);
    k.sidebar_accent = c(p.selected);
    k.sidebar_accent_foreground = c(p.fg);
    k.sidebar_border = c(p.border);
    k.sidebar_primary = primary;
    k.sidebar_primary_foreground = c(p.primary_fg);
    k.tab_bar = c(p.frame);
    k.tab = c(p.tab);
    k.tab_active = c(p.tab_active);
    k.tab_foreground = c(p.fg_subtle);
    k.tab_active_foreground = c(p.fg);
    k.tab_bar_segmented = c(p.surface);
    k.success = c(p.success);
    k.success_foreground = c(p.fg_inverse);
    k.warning = c(p.warning);
    k.warning_foreground = c(p.fg_inverse);
    k.danger = c(p.deep);
    k.danger_foreground = c(0xFFFF_FFFF);
    k.info = c(p.info);
    k.info_foreground = c(p.fg_inverse);
    k.title_bar = c(p.window);
    k.title_bar_border = c(p.border);
    k.status_bar = c(p.window);
    k.status_bar_border = c(p.border);
    k.scrollbar = gpui_kit::transparent_black();
    k.scrollbar_thumb = c(p.scrollbar_thumb);
    k.scrollbar_thumb_hover = c(p.scrollbar_thumb_hover);
    k.selection = c(p.selection);
    k.caret = c(if t.dark { p.cream } else { p.fg });
    k.overlay = c(p.scrim);
    k.link = c(p.link);
    k.link_hover = c(p.info_surface_fg);
    k.link_active = c(p.info_surface_fg);
    k.switch = c(p.secondary);
    k.switch_thumb = c(p.fg_subtle);
    k.table = c(p.frame);
    k.table_head = c(p.frame);
    k.table_head_foreground = c(p.fg_subtle);
    k.table_hover = c(p.hover);
    k.table_active = c(p.selected);
    k.table_row_border = c(p.border);
    k.table_even = c(p.frame);
    k.progress_bar = c(p.primary);
    k.skeleton = c(p.surface);
    k.drag_border = c(p.resize_handle);
    k.window_border = c(p.border);
    k.chart_1 = c(p.categorical[0]);
    k.chart_2 = c(p.categorical[1]);
    k.chart_3 = c(p.categorical[2]);
    k.chart_4 = c(p.categorical[3]);
    k.chart_5 = c(p.categorical[4]);
    k.chart_grid = c(p.border);
    k.red = c(p.ansi[1]);
    k.red_light = c(p.ansi[9]);
    k.green = c(p.ansi[2]);
    k.green_light = c(p.ansi[10]);
    k.yellow = c(p.ansi[3]);
    k.yellow_light = c(p.ansi[11]);
    k.blue = c(p.ansi[4]);
    k.blue_light = c(p.ansi[12]);
    k.magenta = c(p.ansi[5]);
    k.magenta_light = c(p.ansi[13]);
    k.cyan = c(p.ansi[6]);
    k.cyan_light = c(p.ansi[14]);
    if let Some(highlight) = crate::design::syntax::highlight_theme(t) {
        kit.highlight_theme = highlight;
    }
}
