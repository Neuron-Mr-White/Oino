/**
 * Appearance application (WP-A24, Z-114): the UI font-size preference drives
 * `--ui-font-scale` (and `--ui-font-size`) on the root, and the code settings
 * `--code-font-size` / `--code-wrap`. ZCode's rules apply
 * (settings-appearance.md):
 *
 * - the root `html` font size is **never** touched — icons and layout
 *   dimensions do not scale, only text that consumes the vars;
 * - the terminal keeps its own size setting (Terminal section), chat keeps
 *   its markdown scale derived from the UI size.
 *
 * Pure functions + one installer the SettingsProvider mounts app-wide.
 */

/** The CSS var the whole app's prose scale reads (`--ui-font-size`). */
export const UI_FONT_SIZE_VAR = '--ui-font-size';
/**
 * [owner-fix-8] The scale every text size multiplies with: the stylesheets
 * read `calc(<px> * var(--ui-font-scale))`, so the setting moves the whole type
 * scale at once (the icons and the layout stay the same).
 */
export const UI_FONT_SCALE_VAR = '--ui-font-scale';
/** The CSS var code blocks, the editor and diffs read. */
export const CODE_FONT_SIZE_VAR = '--code-font-size';
/** Whether long code lines wrap (`''` = no wrap). */
export const CODE_WRAP_VAR = '--code-wrap';

export const UI_FONT_MIN = 12;
export const UI_FONT_MAX = 20;
export const CODE_FONT_MIN = 12;
export const CODE_FONT_MAX = 20;

/** The prose scale derived from a UI font size (headings keep their ratios). */
export function proseScale(px: number): string {
  return `${px}px`;
}

/**
 * [owner-fix-8] The type scale for a UI font size: the app's base text is
 * 15 px, so 15 px → 1, 12 px → 0.8 and 20 px → 1.33.
 */
export function fontScale(px: number): number {
  return Math.round((clamp(px, UI_FONT_MIN, UI_FONT_MAX) / 15) * 1000) / 1000;
}

/** Writes the appearance vars on one element's inline style (idempotent). */
export function applyAppearance(
  style: CSSStyleDeclaration,
  appearance: { uiFontSize: number; codeFontSize: number; codeWrap: boolean },
): void {
  const ui = clamp(appearance.uiFontSize, UI_FONT_MIN, UI_FONT_MAX);
  style.setProperty(UI_FONT_SCALE_VAR, String(fontScale(ui)));
  style.setProperty(UI_FONT_SIZE_VAR, proseScale(ui));
  style.setProperty(CODE_FONT_SIZE_VAR, `${clamp(appearance.codeFontSize, CODE_FONT_MIN, CODE_FONT_MAX)}px`);
  style.setProperty(CODE_WRAP_VAR, appearance.codeWrap ? 'pre-wrap' : 'pre');
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

/** Reads back what `applyAppearance` wrote (tests). */
export function readAppearanceVars(style: CSSStyleDeclaration): { ui: string; code: string; wrap: string } {
  return {
    ui: style.getPropertyValue(UI_FONT_SIZE_VAR),
    code: style.getPropertyValue(CODE_FONT_SIZE_VAR),
    wrap: style.getPropertyValue(CODE_WRAP_VAR),
  };
}
