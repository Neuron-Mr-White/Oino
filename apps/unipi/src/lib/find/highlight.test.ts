import { beforeEach, describe, expect, it } from 'vitest';
import { applyFindHighlights, activateFindMark, clearFindHighlights } from './highlight';

function mount(html: string): HTMLElement {
  document.body.innerHTML = `<div id="root">${html}</div>`;
  return document.getElementById('root')!;
}

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('applyFindHighlights', () => {
  it('wraps every case-insensitive hit and can undo', () => {
    const root = mount(`<div data-turn="t1"><p>The <b>Tally</b> went up. TALLY again.</p></div>`);
    expect(applyFindHighlights(root, 'tally')).toBe(2);
    const marks = [...root.querySelectorAll('mark.find-hit')];
    expect(marks.map((m) => m.textContent)).toEqual(['Tally', 'TALLY']);
    expect(marks[0]!.closest('b')).not.toBeNull(); // wrapped in place, inside <b>
    clearFindHighlights(root);
    expect(root.querySelectorAll('mark').length).toBe(0);
    expect(root.querySelector('p')!.textContent).toBe('The Tally went up. TALLY again.');
  });

  it('a hit that crosses an element boundary is skipped (whole text nodes only)', () => {
    const root = mount('<p>The <b>Tal</b>ly went up.</p>');
    expect(applyFindHighlights(root, 'tally')).toBe(0);
    expect(root.querySelectorAll('mark').length).toBe(0);
    expect(root.querySelector('p')!.textContent).toBe('The Tally went up.'); // untouched
  });

  it('marks two hits in one text node separately', () => {
    const root = mount('<p>ab ab ab</p>');
    expect(applyFindHighlights(root, 'ab')).toBe(3);
    expect(root.querySelectorAll('mark.find-hit').length).toBe(3);
  });

  it('skips existing marks (re-apply is idempotent after clear)', () => {
    const root = mount('<p>find me and find me</p>');
    applyFindHighlights(root, 'find');
    applyFindHighlights(root, 'find'); // would double-wrap without the guard
    expect(root.querySelectorAll('mark.find-hit').length).toBe(2);
  });

  it('activates the nth mark of a turn and centres it', () => {
    const root = mount(`<div data-turn="t1"><p>one two two</p></div><div data-turn="t2"><p>two</p></div>`);
    applyFindHighlights(root, 'two');
    expect(activateFindMark(root, 't1', 1)).toBe(true);
    const active = root.querySelector('mark.find-hit[data-active]');
    expect(active).not.toBeNull();
    expect((active as HTMLElement).closest('[data-turn]')!.getAttribute('data-turn')).toBe('t1');
    expect(root.querySelectorAll('mark.find-hit[data-active]').length).toBe(1);
    expect(activateFindMark(root, 't2', 0)).toBe(true);
    expect(root.querySelector('mark.find-hit[data-active]')!.closest('[data-turn]')!.getAttribute('data-turn')).toBe('t2');
  });

  it('false when the turn is not mounted (the caller re-runs later)', () => {
    const root = mount('<div data-turn="t1"><p>hit</p></div>');
    applyFindHighlights(root, 'hit');
    expect(activateFindMark(root, 't9', 0)).toBe(false);
  });
});
