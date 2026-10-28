//! `Button` and `IconButton` (design.md §4.1–§4.2 as amended by A2.15): every button is a
//! pill. Primary `shell`/`face` (one per view), secondary surface-3, ghost transparent
//! `fg_subtle` (hover surface-2 + `fg`), danger soft (`danger_surface` + `danger`), and
//! destructive confirms in `deep` with an icon.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, px,
};

use crate::components::tooltip::tip;
use crate::design::icons::{icon, spinner};
use crate::design::theme::{OinoTheme, TextExt};
use crate::design::{TextRole, radius, size, space};

/// Visual variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    /// `primary` fill.
    Primary,
    /// `secondary` fill.
    Secondary,
    /// Transparent with a border.
    Outline,
    /// Transparent, hover fill.
    #[default]
    Ghost,
    /// Destructive confirm: `deep` fill, white text (always icon + word).
    Danger,
    /// Soft danger: `danger_surface` fill, `danger` text (A2.15 danger button).
    DangerSoft,
    /// Ghost with `danger_surface_fg` text.
    GhostDanger,
    /// Outline with `danger_surface_fg` text.
    OutlineDanger,
    /// Text link.
    Link,
}

/// Height preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BtnSize {
    /// 20.
    Xs,
    /// 24.
    Sm,
    /// 28.
    #[default]
    Default,
    /// 32.
    Lg,
    /// 44 (touch targets, design.md §6).
    Touch,
    /// 34: `BUTTON_SM` toolbars and inline actions (A2.15).
    Md,
    /// 44: `BUTTON` dialog footers, empty states, page CTAs (A2.15).
    Xl,
}

impl BtnSize {
    const fn height(self) -> f32 {
        match self {
            Self::Xs => size::CONTROL_XS,
            Self::Sm => size::CONTROL_SM,
            Self::Default => size::CONTROL,
            Self::Lg => size::CONTROL_LG,
            Self::Touch => crate::design::tokens::touch::TARGET,
            Self::Md => size::BUTTON_SM,
            Self::Xl => size::BUTTON,
        }
    }

    const fn pad(self) -> f32 {
        match self {
            Self::Xs => space::S1_5,
            Self::Sm => space::S2,
            Self::Default => 10.0,
            Self::Lg => space::S3,
            Self::Touch => space::S4,
            Self::Md => space::S3_5,
            Self::Xl => space::S5,
        }
    }
}

/// Click handler.
pub type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A button.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent visual flags of one element"
)]
#[derive(IntoElement)]
pub struct Btn {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconName>,
    trailing: Option<IconName>,
    kbd: Option<SharedString>,
    kbd_first: bool,
    variant: Variant,
    size: BtnSize,
    square: bool,
    disabled: bool,
    loading: bool,
    selected: bool,
    radius: f32,
    tooltip: Option<(SharedString, Option<SharedString>)>,
    color: Option<Hsla>,
    full: bool,
    on_click: Option<OnClick>,
}

impl Btn {
    /// A button with `id`.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            trailing: None,
            kbd: None,
            kbd_first: false,
            variant: Variant::Ghost,
            size: BtnSize::Default,
            square: false,
            disabled: false,
            loading: false,
            selected: false,
            radius: radius::R_BUTTON,
            tooltip: None,
            color: None,
            full: false,
            on_click: None,
        }
    }

    /// An icon-only ghost button (tooltip mandatory, design.md §4.2).
    pub fn icon_only(
        id: impl Into<ElementId>,
        name: IconName,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        let mut b = Self::new(id).icon(name);
        b.square = true;
        b.tooltip = Some((tooltip.into(), None));
        b
    }

    /// Label.
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Leading icon.
    #[must_use]
    pub fn icon(mut self, name: IconName) -> Self {
        self.icon = Some(name);
        self
    }

    /// Trailing icon (chevron).
    #[must_use]
    pub fn trailing(mut self, name: IconName) -> Self {
        self.trailing = Some(name);
        self
    }

    /// Key hint shown after the label (`1` for approval options).
    #[must_use]
    pub fn kbd(mut self, keys: impl Into<SharedString>) -> Self {
        self.kbd = Some(keys.into());
        self
    }

    /// Key hint shown before the label (number keys of docked cards: "1 Allow once").
    #[must_use]
    pub fn kbd_lead(mut self, keys: impl Into<SharedString>) -> Self {
        self.kbd = Some(keys.into());
        self.kbd_first = true;
        self
    }

    /// Variant.
    #[must_use]
    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    /// Size.
    #[must_use]
    pub fn size(mut self, size: BtnSize) -> Self {
        self.size = size;
        self
    }

    /// Disabled.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Loading (spinner replaces the icon).
    #[must_use]
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Selected (filled with `selected`).
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Radius (nesting rule: `R_MD` inside `R_LG` parents).
    #[must_use]
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }

    /// Tooltip with an optional shortcut.
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<SharedString>, keys: Option<&str>) -> Self {
        self.tooltip = Some((text.into(), keys.map(|k| SharedString::from(k.to_owned()))));
        self
    }

    /// Text color override (tone buttons).
    #[must_use]
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Full width.
    #[must_use]
    pub fn full(mut self) -> Self {
        self.full = true;
        self
    }

    /// Click handler.
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Btn {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = OinoTheme::get(cx);
        let h = self.size.height();
        // Automation (`gui-driver.md`): what the registry reports for this button.
        let automation = {
            let label = self
                .label
                .as_ref()
                .map_or_else(
                    || self.tooltip.as_ref().map(|(t, _)| t.to_string()),
                    |l| Some(l.to_string()),
                )
                .unwrap_or_default();
            let role = if self.square {
                crate::automation::Role::IconButton
            } else {
                crate::automation::Role::Button
            };
            crate::automation::Meta::labelled(role, label)
                .disabled(self.disabled)
                .selected(self.selected)
        };
        // No keyboard hints on touch (design.md §6.3: number keys are not shown).
        let kbd = if t.touch() { None } else { self.kbd };
        let (bg, fg, border, hover) = match self.variant {
            Variant::Primary => (
                Some(t.primary()),
                t.primary_fg(),
                None,
                Some(t.primary_hover()),
            ),
            Variant::Secondary => (Some(t.secondary()), t.fg(), None, Some(t.border())),
            Variant::Outline => (None, t.fg(), Some(t.border()), Some(t.hover())),
            Variant::OutlineDanger => (
                None,
                t.danger_surface_fg(),
                Some(t.border()),
                Some(t.danger_surface()),
            ),
            Variant::Ghost => (None, t.fg_subtle(), None, Some(t.hover())),
            Variant::DangerSoft => (
                Some(t.danger_surface()),
                t.danger(),
                None,
                Some(t.danger_surface().opacity(1.0)),
            ),
            Variant::GhostDanger => (None, t.danger_surface_fg(), None, Some(t.danger_surface())),
            // Destructive = unipi `deep` with a white label (always icon + word).
            Variant::Danger => (
                Some(t.deep()),
                gpui_kit::white(),
                None,
                Some(t.deep().opacity(0.9)),
            ),
            Variant::Link => (None, t.fg(), None, None),
        };
        let fg = if self.disabled {
            t.fg_subtlest()
        } else {
            self.color.unwrap_or(fg)
        };
        let bg = if self.disabled && matches!(self.variant, Variant::Primary) {
            Some(t.primary().opacity(0.35))
        } else if self.selected {
            Some(t.selected())
        } else {
            bg
        };
        let icon_size = if h <= size::CONTROL_SM {
            size::ICON_MD
        } else {
            size::ICON
        };
        let lead: Option<AnyElement> = if self.loading {
            Some(
                spinner(crate::design::ticker::Ticker::frame(cx), icon_size, fg).into_any_element(),
            )
        } else {
            self.icon.map(|i| icon(i, icon_size, fg).into_any_element())
        };
        let on_click = self.on_click.clone();
        let disabled = self.disabled;
        let mut el = div()
            .id(self.id)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .gap(px(space::S1_5))
            .h(px(h))
            .rounded(px(self.radius))
            .text_color(fg)
            .text_role(
                &t,
                if h <= size::CONTROL_XS {
                    TextRole::Sm
                } else if h >= size::BUTTON {
                    TextRole::Prose
                } else {
                    TextRole::Base
                },
            )
            .when(
                h >= size::BUTTON || self.variant == Variant::Primary,
                TextExt::weight_semibold,
            )
            .when(
                h < size::BUTTON && self.variant != Variant::Primary,
                TextExt::weight_medium,
            )
            .when(self.square, |el| el.w(px(h)))
            .when(!self.square, |el| el.px(px(self.size.pad())))
            .when(self.full, gpui_kit::Styled::w_full)
            .when_some(bg, gpui_kit::Styled::bg)
            .when_some(border, |el, b| el.border_1().border_color(b))
            .children(lead)
            .when(self.kbd_first, |el| {
                el.children(kbd.clone().map(|k| super::badge::kbd(&t, k)))
            })
            .when_some(self.label, |el, l| {
                el.child(
                    div()
                        .truncate()
                        .when(self.variant == Variant::Link, gpui_kit::Styled::underline)
                        .child(l),
                )
            })
            .when(!self.kbd_first, |el| {
                el.children(kbd.map(|k| super::badge::kbd(&t, k)))
            })
            .when_some(self.trailing, |el, i| {
                el.child(icon(i, size::ICON_SM, t.fg_subtle()))
            });
        if !disabled {
            // Keyboard reachable: Tab stops on it, Enter/Space click it, and keyboard focus
            // draws the 2 px shell ring (A1.2).
            el = el
                .tab_index(0)
                .focus_visible(|s| {
                    s.border(px(crate::design::tokens::focus::WIDTH))
                        .border_color(t.focus())
                })
                .cursor_pointer()
                .when_some(hover, |el, h| {
                    let ghost = matches!(self.variant, Variant::Ghost) && self.color.is_none();
                    let strong = t.fg();
                    el.hover(move |s| {
                        let s = s.bg(h);
                        if ghost { s.text_color(strong) } else { s }
                    })
                });
            if let Some(f) = on_click {
                el = el.on_click(move |e, w, cx| f(e, w, cx));
            }
        }
        if let Some((text, keys)) = self.tooltip {
            el = el.tooltip(tip(text, keys));
        }
        crate::automation::probe(el, automation)
    }
}
