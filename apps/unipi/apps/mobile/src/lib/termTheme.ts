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
  selectionBackground: '#26211F',
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
