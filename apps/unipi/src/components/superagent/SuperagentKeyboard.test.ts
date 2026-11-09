import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync('src/components/superagent/superagent.css', 'utf8');

/** OIN-63: one keyboard source of truth on the phone — the frame shrinks by --kb, the dock adds no IME inset. */
describe('Superagent phone composer dock vs the soft keyboard (OIN-63)', () => {
  it('pads the dock with the safe area only while closed, and 8px while the keyboard is open', () => {
    expect(css).toContain('html[data-shell=phone] .sa-center .tl-dock { padding-bottom: calc(8px + var(--safe-bottom, 0px)); }');
    expect(css).toContain('html[data-shell=phone][data-kb=open] .sa-center .tl-dock { padding-bottom: 8px; }');
  });

  it('never re-applies the keyboard (--kb / --inset-ime / --vv-bottom) in the superagent rules', () => {
    const rules = css.replace(/\/\*[\s\S]*?\*\//g, '').split('\n');
    expect(rules.filter((l) => /--inset-ime|--vv-bottom|var\(--kb/.test(l))).toEqual([]);
  });
});
