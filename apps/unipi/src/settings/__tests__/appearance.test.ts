// @vitest-environment jsdom
/**
 * The appearance settings (WP-A24, Z-114; owner-fix-8): the UI font size
 * scales the app's whole type scale live.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { applyAppearance, fontScale, readAppearanceVars, UI_FONT_SCALE_VAR, UI_FONT_SIZE_VAR } from '../appearance';

const appearance = (over: Partial<{ uiFontSize: number; codeFontSize: number; codeWrap: boolean }> = {}) => ({
  uiFontSize: 15,
  codeFontSize: 13,
  codeWrap: true,
  ...over,
});

afterEach(() => {
  document.documentElement.style.removeProperty(UI_FONT_SCALE_VAR);
  document.documentElement.style.removeProperty(UI_FONT_SIZE_VAR);
});

describe('the appearance scale (owner-fix-8)', () => {
  it('writes the type scale next to the font size', () => {
    const style = document.documentElement.style;
    applyAppearance(style, appearance({ uiFontSize: 18 }));
    expect(style.getPropertyValue(UI_FONT_SCALE_VAR)).toBe('1.2');
    expect(style.getPropertyValue(UI_FONT_SIZE_VAR)).toBe('18px');
    expect(readAppearanceVars(style).ui).toBe('18px');
  });

  it('scales with the size: 12 px is 0.8, 15 px is 1, 20 px is 1.333', () => {
    expect(fontScale(12)).toBe(0.8);
    expect(fontScale(15)).toBe(1);
    expect(fontScale(20)).toBe(1.333);
  });

  it('keeps the code font size on its own setting (it is not scaled by the UI size)', () => {
    const style = document.documentElement.style;
    applyAppearance(style, appearance({ uiFontSize: 20, codeFontSize: 14 }));
    expect(style.getPropertyValue('--code-font-size')).toBe('14px');
  });
});
