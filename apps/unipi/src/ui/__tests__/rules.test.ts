/**
 * Source rules of `src/ui/` (ui-protocol §13.3, D-U12): no HTML strings into
 * the DOM outside `nodes/Markdown.tsx`; no JS timers driving the Spinner;
 * one component file per node kind.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { NODE_KINDS } from '../types';

const ROOT = resolve(__dirname, '..');

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    // `__tests__` is test code; `dialogs/` is the app's dialog card (moved in
    // from `components/chat/` by WP-A9), not a node renderer — its markdown
    // text is rendered by `lib/chat/markdown`.
    if (statSync(p).isDirectory()) return f === '__tests__' || f === 'dialogs' ? [] : files(p);
    return /\.(ts|tsx)$/.test(f) ? [p] : [];
  });
}

describe('src/ui rules', () => {
  it('no innerHTML / outerHTML / insertAdjacentHTML writes outside nodes/Markdown.tsx', () => {
    const offenders = files(ROOT)
      .filter((p) => !p.endsWith(join('nodes', 'Markdown.tsx')))
      .filter((p) => /\.(innerHTML|outerHTML)\s*=|insertAdjacentHTML|dangerouslySetInnerHTML|innerHTML=\{/.test(readFileSync(p, 'utf8')))
      .map((p) => relative(ROOT, p));
    expect(offenders).toEqual([]);
  });

  it('one file per node kind under nodes/', () => {
    const present = new Set(readdirSync(join(ROOT, 'nodes')).map((f) => f.replace(/\.tsx?$/, '')));
    expect(NODE_KINDS.filter((k) => !present.has(k))).toEqual([]);
  });

  it('the Spinner uses no JS timer', () => {
    expect(readFileSync(join(ROOT, 'nodes', 'Spinner.tsx'), 'utf8')).not.toMatch(/setInterval|setTimeout|requestAnimationFrame/);
  });
});
