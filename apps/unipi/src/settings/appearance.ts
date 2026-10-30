/**
 * Appearance application (WP-A24, Z-114): the UI font-size preference drives
 * `--ui-font-size` on the root, and the code settings `--code-font-size` /
 * `--code-wrap`. ZCode's rules apply (settings-appearance.md):
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

/** Writes the appearance vars on one element's inline style (idempotent). */
export function applyAppearance(
  style: CSSStyleDeclaration,
  appearance: { uiFontSize: number; codeFontSize: number; codeWrap: boolean },
): void {
  style.setProperty(UI_FONT_SIZE_VAR, proseScale(clamp(appearance.uiFontSize, UI_FONT_MIN, UI_FONT_MAX)));
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
