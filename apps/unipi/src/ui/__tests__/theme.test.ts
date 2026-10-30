/**
 * Theme tokens (ui-protocol §5): the token table is complete, styles map to
 * CSS, and the contrast rule holds — every same-hue `*Bg` + `fg` pair, and
 * every (fg, bg) pair used in the fixture corpus, is ≥ 4.5:1 over `--color-bg`.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { TOKENS, contrast, patchStyle, styleCss } from '../theme';
import type { Color } from '../types';

const SCHEMA = JSON.parse(readFileSync(resolve(__dirname, '../../../../../crates/oino-ui/schema/node.schema.json'), 'utf8')) as {
  $defs: { Color: { enum: string[] } };
};
const NODES = resolve(__dirname, '../../../../../crates/oino-ui/fixtures/nodes');

function stylePairs(v: unknown, out: Array<[Color, Color]>): void {
  if (Array.isArray(v)) return v.forEach((x) => stylePairs(x, out));
  if (!v || typeof v !== 'object') return;
  const o = v as Record<string, unknown>;
  if (typeof o.fg === 'string' && typeof o.bg === 'string') out.push([o.fg as Color, o.bg as Color]);
  Object.values(o).forEach((x) => stylePairs(x, out));
}

describe('tokens', () => {
  it('maps every schema colour token', () => {
    expect(Object.keys(TOKENS).sort()).toEqual([...SCHEMA.$defs.Color.enum].sort());
  });

  it('accent is claw, primary is shell (RULINGS ui-protocol §16)', () => {
    expect(TOKENS.accent).toBe('var(--ui-accent)');
    expect(TOKENS.primary).toBe('var(--ui-primary)');
    const css = readFileSync(resolve(__dirname, '../ui.css'), 'utf8');
    expect(css).toMatch(/--ui-accent: var\(--color-claw\)/);
    expect(css).toMatch(/--ui-primary: var\(--color-shell\)/);
  });

  it('every --ui-* variable the token table names is defined in ui.css', () => {
    const css = readFileSync(resolve(__dirname, '../ui.css'), 'utf8');
    for (const v of Object.values(TOKENS)) {
      const name = /var\((--ui-[\w-]+)\)/.exec(v)?.[1];
      if (name) expect(css, name).toContain(`${name}:`);
    }
  });
});

describe('contrast ≥ 4.5:1 on --color-bg (§5 rule)', () => {
  it.each([
    ['success', 'successBg'],
    ['error', 'errorBg'],
    ['warning', 'warningBg'],
    ['info', 'infoBg'],
    ['accent', 'accentBg'],
    ['text', 'selectedBg'],
    ['text', 'surface1'],
    ['text', 'surface2'],
    ['text', 'surface3'],
    ['muted', 'surface1'],
    ['muted', 'surface3'],
  ] as Array<[Color, Color]>)('%s on %s', (fg, bg) => {
    expect(contrast(fg, bg)).toBeGreaterThanOrEqual(4.5);
  });

  it('every (fg, bg) pair in the fixture corpus', () => {
    const pairs: Array<[Color, Color]> = [];
    for (const f of readdirSync(NODES)) stylePairs(JSON.parse(readFileSync(resolve(NODES, f), 'utf8')), pairs);
    expect(pairs.length).toBeGreaterThan(0);
    for (const [fg, bg] of pairs) expect(contrast(fg, bg), `${fg} on ${bg}`).toBeGreaterThanOrEqual(4.5);
  });
});

describe('styleCss (§4.2)', () => {
  it('maps fg/bg tokens, modifiers and the span chip', () => {
    expect(styleCss({ fg: 'error', bg: 'errorBg', addModifier: ['BOLD', 'ITALIC', 'SLOW_BLINK'] }, { chip: true })).toEqual({
      class: 'ui-chip ui-m-bold ui-m-italic',
      style: { color: 'var(--ui-error)', 'background-color': 'var(--ui-error-bg)' },
    });
  });
  it('REVERSED swaps fg/bg (fg default → surface1)', () => {
    expect(styleCss({ fg: 'accent', addModifier: ['REVERSED'] }).style).toEqual({ color: 'var(--ui-surface1)', 'background-color': 'var(--ui-accent)' });
  });
  it('subModifier undoes inherited modifiers; unknown tokens drop', () => {
    expect(styleCss({ subModifier: ['BOLD'], fg: 'nope' as Color })).toEqual({ class: 'ui-s-bold', style: {} });
  });
  it('patchStyle follows ratatui Style::patch', () => {
    expect(patchStyle({ fg: 'text', addModifier: ['BOLD', 'DIM'] }, { fg: 'error', subModifier: ['DIM'], addModifier: ['ITALIC'] })).toEqual({
      fg: 'error',
      bg: undefined,
      underlineColor: undefined,
      addModifier: ['BOLD', 'ITALIC'],
      subModifier: ['DIM'],
    });
  });
});
