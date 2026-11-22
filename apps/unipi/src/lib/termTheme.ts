/**
 * Unicrab xterm theme (DESIGN.md §1): the terminal surface is the
 * warm-dark background token; ANSI colours come from the brand and
 * agent-state tokens, with brighter 8–15 variants.
 */
export const termTheme = {
  background: '#0A0908',
  foreground: '#F5EFE6',
  cursor: '#F03018',
  cursorAccent: '#181818',
  selectionBackground: 'rgba(240, 120, 24, 0.28)',
  selectionInactiveBackground: 'rgba(163, 154, 144, 0.18)',
  black: '#181818',
  red: '#F03018',
  green: '#5AD25A',
  yellow: '#F0C030',
  blue: '#28BEF0',
  magenta: '#F07818',
  cyan: '#28BEF0',
  white: '#F5EFE6',
  brightBlack: '#6E655D',
  brightRed: '#F06048',
  brightGreen: '#8AE48A',
  brightYellow: '#FADF70',
  brightBlue: '#68D8FF',
  brightMagenta: '#FFA858',
  brightCyan: '#68D8FF',
  brightWhite: '#FFFFFF',
} as const;

export type TermTheme = { -readonly [K in keyof typeof termTheme]: string };

/**
 * The xterm theme from the app's design tokens (`style.css` `:root`), so the
 * terminal follows the palette: the surface is `--color-surface-1` (the active
 * pane tab's colour, so tab and terminal read as one sheet), text, cursor and
 * ANSI colours come from the brand / window-manager tokens. Any token that
 * can't be read keeps the `termTheme` value.
 */
export function termThemeFromTokens(read: (token: string) => string | undefined): TermTheme {
  const pick = (token: string, fallback: string) => {
    const v = read(token)?.trim();
    return v ? v : fallback;
  };
  return {
    ...termTheme,
    background: pick('--color-surface-1', '#141211'),
    foreground: pick('--color-text', termTheme.foreground),
    cursor: pick('--color-shell', termTheme.cursor),
    cursorAccent: pick('--color-face', termTheme.cursorAccent),
    black: pick('--color-face', termTheme.black),
    red: pick('--color-shell', termTheme.red),
    green: pick('--color-wm-green', termTheme.green),
    yellow: pick('--color-gold', termTheme.yellow),
    blue: pick('--color-wm-cyan', termTheme.blue),
    magenta: pick('--color-claw', termTheme.magenta),
    cyan: pick('--color-wm-cyan', termTheme.cyan),
    white: pick('--color-text', termTheme.white),
    brightBlack: pick('--color-text-dim', termTheme.brightBlack),
  };
}

/** `termThemeFromTokens` over the document's computed `:root` styles. */
export function appTermTheme(): TermTheme {
  if (typeof document === 'undefined' || typeof getComputedStyle === 'undefined') return termThemeFromTokens(() => undefined);
  const style = getComputedStyle(document.documentElement);
  return termThemeFromTokens((token) => style.getPropertyValue(token));
}
