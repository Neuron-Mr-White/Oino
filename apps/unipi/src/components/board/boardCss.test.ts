import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/**
 * OIN-92: the Workbench mounts the task detail outside the Board screen, so `screens/board.css` (the only home of
 * `.att-grid .att-thumb`) was never loaded there and composer.css's 28px chip thumbnail rendered the attachment
 * tiles as blank 28px squares. Whoever mounts these components must pull the sheet in itself.
 */
describe('board.css reaches every surface that renders task attachments', () => {
  for (const file of ['./TaskDetail.tsx', '../superagent/ReviewWorkspace.tsx']) {
    it(`${file} imports screens/board.css`, () => {
      const source = readFileSync(new URL(file, import.meta.url), 'utf8');
      expect(source).toMatch(/^import '\.\.\/\.\.\/screens\/board\.css';$/m);
    });
  }
});

/**
 * D4: a tall phone screenshot rendered as an empty tile. In a centring grid the implicit `auto` row grew to the tall
 * image's own height (the image itself is capped to the frame by `max-height:100%`, which a row that is still being
 * sized cannot resolve) and the image sat centred in that oversized row, outside the frame's `overflow:hidden`.
 * Every "box that holds one image to fit" must give its grid ONE track that is the box's size.
 */
describe('image boxes size their only grid track to the box (D4)', () => {
  const css = (file: string) => readFileSync(new URL(file, import.meta.url), 'utf8');
  const rule = (source: string, selector: string) => {
    const at = source.indexOf(`\n${selector} {`);
    expect(at, `${selector} is defined`).toBeGreaterThanOrEqual(0);
    return source.slice(at, source.indexOf('}', at));
  };
  for (const [file, selector] of [
    ['../../screens/board.css', '.att-thumb-frame'],
    ['../superagent/superagent.css', '.image-pane-stage'],
    ['../media/media.css', '.ptab-img'],
  ] as const) {
    it(`${selector} has a single minmax(0, 1fr) track`, () => {
      expect(rule(css(file), selector).replace(/\s+/g, ' ')).toMatch(/grid-template: ?minmax\(0, ?1fr\) \/ ?minmax\(0, ?1fr\)/);
    });
  }
});

/** N10: the chip's ✕ stays 26 px on the phone, its touch target is 44 × 44 (26 + 2 × 9). */
describe('the attachment chip ✕ has a 44 px touch target on the phone (N10)', () => {
  it('draws 26 px and extends its hit area by 9 px on every side', () => {
    const sheet = readFileSync(new URL(['./note-field', 'css'].join('.'), import.meta.url), 'utf8');
    expect(sheet).toMatch(/html\[data-shell=phone\] \.draft-chip-x \{[^}]*width: 26px; height: 26px;/);
    expect(sheet).toMatch(/html\[data-shell=phone\] \.draft-chip-x::after \{[^}]*inset: -9px;/);
  });
});
