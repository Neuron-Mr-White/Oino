/**
 * Minimal SGR → styled-runs converter for ANSI thumbnails (DESIGN
 * §3.1). Handles: reset (0), bold (1), dim (2), 16-colour fg/bg
 * (30–37/90–97, 40–47/100–107), 256-colour (38;5;n / 48;5;n),
 * truecolor (38;2;r;g;b / 48;2;r;g;b). Every other escape sequence
 * (cursor movement, OSC titles, mode switches…) is stripped.
 * Output: lines of {text, style} runs — Solid-renderable, no innerHTML.
 */
import type { JSX } from 'solid-js';

export type AnsiRun = { text: string; style: JSX.CSSProperties };
export type AnsiLine = AnsiRun[];

/** Maps an SGR colour onto the Unicrab xterm theme. */
function themeColor(code: number, background: boolean): string | undefined {
  const palette: Record<number, string> = {
    30: '#181818', 31: '#F03018', 32: '#5AD25A', 33: '#F0C030',
    34: '#28BEF0', 35: '#F07818', 36: '#28BEF0', 37: '#F5EFE6',
    90: '#6E655D', 91: '#F06048', 92: '#8AE48A', 93: '#FADF70',
    94: '#68D8FF', 95: '#FFA858', 96: '#68D8FF', 97: '#FFFFFF',
  };
  const base = background ? code - 10 : code;
  return palette[base];
}

type Sgr = {
  fg?: string;
  bg?: string;
  bold?: boolean;
  dim?: boolean;
};

function applySgr(current: Sgr, codes: number[]): Sgr {
  const next = { ...current };
  let i = 0;
  while (i < codes.length) {
    const code = codes[i];
    if (code === 0) {
      delete next.fg;
      delete next.bg;
      next.bold = false;
      next.dim = false;
    } else if (code === 1) {
      next.bold = true;
    } else if (code === 2) {
      next.dim = true;
    } else if (code === 22) {
      next.bold = false;
      next.dim = false;
    } else if (code === 39) {
      delete next.fg;
    } else if (code === 49) {
      delete next.bg;
    } else if ((code >= 30 && code <= 37) || (code >= 90 && code <= 97)) {
      next.fg = themeColor(code, false);
    } else if ((code >= 40 && code <= 47) || (code >= 100 && code <= 107)) {
      next.bg = themeColor(code, true);
    } else if ((code === 38 || code === 48) && i + 1 < codes.length) {
      if (codes[i + 1] === 5 && i + 2 < codes.length) {
        const colour = xterm256(codes[i + 2]);
        if (code === 38) next.fg = colour;
        else next.bg = colour;
        i += 2;
      } else if (codes[i + 1] === 2 && i + 4 <= codes.length - 1) {
        const colour = `rgb(${codes[i + 2]}, ${codes[i + 3]}, ${codes[i + 4]})`;
        if (code === 38) next.fg = colour;
        else next.bg = colour;
        i += 4;
      }
    }
    i += 1;
  }
  return next;
}

/** The xterm.js 256-colour cube: 0–15 standard, 16–231 cube, 232–255 grey. */
function xterm256(index: number): string {
  if (index < 16) return themeColor(index < 8 ? index + 30 : index - 8 + 90, false) ?? '#000';
  if (index >= 232) {
    const grey = 8 + (index - 232) * 10;
    return `rgb(${grey}, ${grey}, ${grey})`;
  }
  const cube = index - 16;
  const r = Math.floor(cube / 36);
  const g = Math.floor((cube % 36) / 6);
  const b = cube % 6;
  const channel = (v: number) => (v === 0 ? 0 : 55 + v * 40);
  return `rgb(${channel(r)}, ${channel(g)}, ${channel(b)})`;
}

function styleOf(sgr: Sgr): JSX.CSSProperties {
  const style: JSX.CSSProperties = {};
  if (sgr.fg) style.color = sgr.fg;
  if (sgr.bg) style.background = sgr.bg;
  if (sgr.bold) style['font-weight'] = '600';
  if (sgr.dim) style.opacity = '0.6';
  return style;
}

/** Strips every non-SGR escape: CSI cursor/ modes, OSC titles, DCS. */
function stripNonSgr(text: string): string {
  return text
    .replace(/\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g, '') // OSC
    .replace(/\x1bP[\s\S]*?\x1b\\/g, '') // DCS
    .replace(/\x1b\[[0-9;?]*[A-Za-ln-z]/g, (match) =>
      match.endsWith('m') ? match : '',
    )
    .replace(/\x1b[=>78Mc]/g, '');
}

/** Converts ANSI text (with escapes) into styled lines. */
export function ansiToLines(text: string, maxLines?: number): AnsiLine[] {
  const clean = stripNonSgr(text);
  const rawLines = clean.split('\n');
  const lines: AnsiLine[] = [];
  let current: Sgr = {};
  for (const raw of rawLines) {
    const line: AnsiLine = [];
    const pattern = /\x1b\[([0-9;]*)m/g;
    let lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(raw)) !== null) {
      if (match.index > lastIndex) {
        line.push({ text: raw.slice(lastIndex, match.index), style: styleOf(current) });
      }
      const codes = match[1]
        .split(';')
        .filter((code) => code !== '')
        .map(Number);
      current = applySgr(current, codes.length ? codes : [0]);
      lastIndex = match.index + match[0].length;
    }
    if (lastIndex < raw.length) {
      line.push({ text: raw.slice(lastIndex), style: styleOf(current) });
    }
    lines.push(line);
  }
  const trimmed = maxLines !== undefined ? lines.slice(-maxLines) : lines;
  // Drop leading empty lines left over from clipping.
  while (trimmed.length > 1 && trimmed[0].every((run) => run.text.trim() === '')) {
    trimmed.shift();
  }
  return trimmed;
}
