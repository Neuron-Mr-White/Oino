/**
 * Phone layout of Settings › MCP (OIN-101 WI-3: cards, full-screen sheet, ≥ 44 px targets). jsdom has no layout, so the
 * rules are asserted in the stylesheet; the real measurement is `scripts/qa/mcp-wi3-shots.mjs` (it lists every control
 * under 44 px at 412 wide — docs/dev/v3/ux/mcp/WI-3.md has the result).
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync(`${process.cwd()}/src/components/mcp/mcp.css`, 'utf8');
const PHONE = 'html:not([data-shell=desktop])';

/** The declarations of every rule whose selector list mentions `selector` under the phone prefix. */
function phoneRules(selector: string): string {
  return css
    .split('}')
    .filter((rule) => rule.includes(PHONE) && rule.split('{')[0]!.split(',').some((sel) => sel.trim() === `${PHONE} ${selector}`))
    .map((rule) => rule.split('{')[1] ?? '')
    .join(';');
}

describe('MCP page on the phone', () => {
  it.each(['.mcps-btn', '.mcps-foot-btn', '.mcps-name', '.mcps-scope-select', '.mcps-masked', '.mcps-add-row', '.mcps-chip', '.mcps-sheet .seg-option', '.mcps-sheet .settings-input', '.mcps-kv > .mcps-secret'])('%s is at least 44 px tall', (selector) => {
    expect(phoneRules(selector), selector).toMatch(/min-height:\s*44px/);
  });

  it('the remove-row ✕ is 44 × 44', () => {
    const r = phoneRules('.mcps-kv > .mcps-kv-x');
    expect(r).toMatch(/width:\s*44px/);
    expect(r).toMatch(/height:\s*44px/);
  });

  it('server rows become cards (own surface, one column, gap between)', () => {
    expect(phoneRules('.mcps-row')).toMatch(/grid-template-columns:\s*minmax\(0, 1fr\)/);
    expect(phoneRules('.mcps-row')).toMatch(/border-radius:\s*14px/);
    expect(phoneRules('.mcps-list')).toMatch(/gap:\s*10px/);
  });

  it('the scope picker is a full-width select', () => {
    expect(phoneRules('.mcps-scope-select')).toMatch(/width:\s*100%/);
  });
});
