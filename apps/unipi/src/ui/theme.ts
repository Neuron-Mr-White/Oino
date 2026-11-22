/**
 * Theme tokens (ui-protocol §5): role token → CSS variable (defined once in
 * `ui/ui.css` on top of the `@theme` palette in `style.css`), and ratatui
 * `Style` → class list + inline style (§4.2 rendering rules). Colours are
 * tokens only; an unknown token is dropped (core already diagnosed it).
 */
import type { JSX } from 'solid-js';
import type { Color, Modifier, Style } from './types';

/** Normative token table (§5). Values are the CSS variables `ui.css` defines. */
export const TOKENS: Record<Color, string> = {
  reset: 'inherit',
  text: 'var(--ui-text)',
  muted: 'var(--ui-muted)',
  dim: 'var(--ui-dim)',
  accent: 'var(--ui-accent)',
  primary: 'var(--ui-primary)',
  info: 'var(--ui-info)',
  link: 'var(--ui-info)',
  success: 'var(--ui-success)',
  warning: 'var(--ui-warning)',
  error: 'var(--ui-error)',
  harness: 'var(--ui-harness)',
  thinking: 'var(--ui-muted)',
  tool: 'var(--ui-text)',
  user: 'var(--ui-text)',
  assistant: 'var(--ui-text)',
  code: 'var(--ui-code)',
  border: 'var(--ui-border)',
  borderAccent: 'var(--ui-border-accent)',
  borderMuted: 'var(--ui-border-muted)',
  c1: 'var(--ui-c1)',
  c2: 'var(--ui-c2)',
  c3: 'var(--ui-c3)',
  c4: 'var(--ui-c4)',
  c5: 'var(--ui-c5)',
  c6: 'var(--ui-c6)',
  c7: 'var(--ui-c7)',
  c8: 'var(--ui-c8)',
  c9: 'var(--ui-c9)',
  c10: 'var(--ui-c10)',
  c11: 'var(--ui-c11)',
  c12: 'var(--ui-c12)',
  surface1: 'var(--ui-surface1)',
  surface2: 'var(--ui-surface2)',
  surface3: 'var(--ui-surface3)',
  selectedBg: 'var(--ui-surface3)',
  successBg: 'var(--ui-success-bg)',
  errorBg: 'var(--ui-error-bg)',
  warningBg: 'var(--ui-warning-bg)',
  infoBg: 'var(--ui-info-bg)',
  accentBg: 'var(--ui-accent-bg)',
  diffAdded: 'var(--ui-diff-added)',
  diffRemoved: 'var(--ui-diff-removed)',
  diffContext: 'var(--ui-diff-context)',
};

/** Resolved sRGB values (for the contrast check and SVG fills that can't use vars). */
export const TOKEN_HEX: Partial<Record<Color, string>> = {
  text: '#F5EFE6',
  muted: '#A39A90',
  dim: '#6E655D',
  accent: '#F07818',
  primary: '#F03018',
  info: '#28BEF0',
  link: '#28BEF0',
  success: '#5AD25A',
  warning: '#FAC828',
  error: '#F03C32',
  harness: '#F0A818',
  thinking: '#A39A90',
  tool: '#F5EFE6',
  user: '#F5EFE6',
  assistant: '#F5EFE6',
  code: '#F0F0A8',
  border: '#2E2825',
  borderAccent: '#F03018',
  c1: '#69ADD0',
  c2: '#68B78D',
  c3: '#CD9A22',
  c4: '#A798D7',
  c5: '#EA7F81',
  c6: '#5DB3BA',
  c7: '#DE8D5A',
  c8: '#9776E5',
  c9: '#DE54C1',
  c10: '#6185CC',
  c11: '#5EB286',
  c12: '#FE5462',
  surface1: '#141211',
  surface2: '#1C1917',
  surface3: '#26211F',
  selectedBg: '#26211F',
  diffAdded: '#B5E8B5',
  diffRemoved: '#F3B3AD',
  diffContext: '#6E655D',
};

/** Translucent tints (§5): `[r, g, b, alpha]` composited over the surface. */
export const TOKEN_TINT: Partial<Record<Color, [number, number, number, number]>> = {
  successBg: [90, 210, 90, 0.12],
  errorBg: [240, 60, 50, 0.12],
  warningBg: [250, 200, 40, 0.12],
  infoBg: [40, 190, 240, 0.12],
  accentBg: [240, 120, 24, 0.14],
  borderMuted: [46, 40, 37, 0.6],
};

export function tokenVar(color: string | undefined | null): string | undefined {
  if (!color) return undefined;
  return (TOKENS as Record<string, string>)[color];
}

const MOD_CLASS: Partial<Record<Modifier, string>> = {
  BOLD: 'ui-m-bold',
  DIM: 'ui-m-dim',
  ITALIC: 'ui-m-italic',
  UNDERLINED: 'ui-m-under',
  CROSSED_OUT: 'ui-m-strike',
  HIDDEN: 'ui-m-hidden',
  // SLOW_BLINK / RAPID_BLINK: ignored (motion rule: only the Unicrab loops).
};

/** The undo class of a removed modifier (`subModifier`, ratatui `Style::patch`). */
const SUB_CLASS: Partial<Record<Modifier, string>> = {
  BOLD: 'ui-s-bold',
  DIM: 'ui-s-dim',
  ITALIC: 'ui-s-italic',
  UNDERLINED: 'ui-s-under',
  CROSSED_OUT: 'ui-s-under',
  HIDDEN: 'ui-s-hidden',
};

export type StyleOut = { class: string; style: JSX.CSSProperties };

export type StyleOptions = {
  /** Spans render a `bg` as an inline chip (§4.2). */
  chip?: boolean;
};

/**
 * ratatui `Style` → CSS. Composition is the DOM's: child patches parent via
 * inheritance (`color`), modifiers via classes, `subModifier` via undo classes.
 */
export function styleCss(style: Style | undefined | null, opts: StyleOptions = {}): StyleOut {
  if (!style) return { class: '', style: {} };
  const classes: string[] = [];
  const css: JSX.CSSProperties = {};
  const mods = style.addModifier ?? [];
  const reversed = mods.includes('REVERSED');
  const fg = tokenVar(style.fg);
  const bg = tokenVar(style.bg);
  if (reversed) {
    css.color = bg ?? 'var(--ui-surface1)';
    css['background-color'] = fg ?? 'var(--ui-text)';
    if (opts.chip) classes.push('ui-chip');
  } else {
    if (fg) css.color = fg;
    if (bg && style.bg !== 'reset') {
      css['background-color'] = bg;
      if (opts.chip) classes.push('ui-chip');
    }
  }
  const under = tokenVar(style.underlineColor);
  if (under) css['text-decoration-color'] = under;
  for (const m of mods) {
    const c = MOD_CLASS[m];
    if (c) classes.push(c);
  }
  for (const m of style.subModifier ?? []) {
    const c = SUB_CLASS[m];
    if (c) classes.push(c);
  }
  return { class: classes.join(' '), style: css };
}

/** Merges ratatui-style: `b` patches `a` (fg/bg/underline override; modifiers add/remove). */
export function patchStyle(a: Style | undefined, b: Style | undefined): Style | undefined {
  if (!a) return b;
  if (!b) return a;
  const add = new Set([...(a.addModifier ?? [])]);
  for (const m of b.subModifier ?? []) add.delete(m);
  for (const m of b.addModifier ?? []) add.add(m);
  return {
    fg: b.fg ?? a.fg,
    bg: b.bg ?? a.bg,
    underlineColor: b.underlineColor ?? a.underlineColor,
    addModifier: add.size ? [...add] : undefined,
    subModifier: b.subModifier,
  };
}

// ── Contrast (WCAG 2.x relative luminance) ──────────────────────────────

function hexRgb(hex: string): [number, number, number] {
  const h = hex.replace('#', '');
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

function luminance([r, g, b]: [number, number, number]): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** The colour a token paints over `under` (tints are composited). */
export function resolveRgb(token: Color, under: [number, number, number]): [number, number, number] | undefined {
  const tint = TOKEN_TINT[token];
  if (tint) {
    const [r, g, b, a] = tint;
    return [r * a + under[0] * (1 - a), g * a + under[1] * (1 - a), b * a + under[2] * (1 - a)];
  }
  const hex = TOKEN_HEX[token];
  return hex ? hexRgb(hex) : undefined;
}

/** `--color-bg`. */
export const APP_BG = '#0A0908';

/** Contrast ratio of `fg` on `bg` painted over `--color-bg` (§5 rule). */
export function contrast(fg: Color, bg: Color | undefined): number {
  const base = hexRgb(APP_BG);
  const back = bg ? (resolveRgb(bg, base) ?? base) : base;
  const front = resolveRgb(fg, back) ?? hexRgb(TOKEN_HEX.text!);
  const [l1, l2] = [luminance(front), luminance(back)].sort((x, y) => y - x) as [number, number];
  return (l1 + 0.05) / (l2 + 0.05);
}
