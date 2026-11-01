import { describe, expect, it } from 'vitest';
import { termTheme, termThemeFromTokens } from './termTheme';

describe('terminal theme from the design tokens', () => {
  it('reads the palette tokens and falls back per colour', () => {
    const tokens: Record<string, string> = { '--color-surface-1': ' #141211 ', '--color-shell': '#F03018', '--color-wm-green': '#5AD25A' };
    const theme = termThemeFromTokens((t) => tokens[t]);
    expect(theme.background).toBe('#141211');
    expect(theme.cursor).toBe('#F03018');
    expect(theme.green).toBe('#5AD25A');
    expect(theme.foreground).toBe(termTheme.foreground);
    expect(theme.brightRed).toBe(termTheme.brightRed);
  });
});
