// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { dropTargetFrom, edgeSpeed, SCROLL_EDGE, SCROLL_MAX_SPEED } from './cardDrag';

describe('edgeSpeed (autoscroll, px/s)', () => {
  it('is 0 outside the edge zones and grows linearly to the max at the edge', () => {
    expect(edgeSpeed(500, 0, 1000)).toBe(0);
    expect(edgeSpeed(1000 - SCROLL_EDGE, 0, 1000)).toBe(0);
    expect(edgeSpeed(1000 - SCROLL_EDGE / 2, 0, 1000)).toBeCloseTo(SCROLL_MAX_SPEED / 2);
    expect(edgeSpeed(1000, 0, 1000)).toBe(SCROLL_MAX_SPEED);
    expect(edgeSpeed(0, 0, 1000)).toBe(-SCROLL_MAX_SPEED);
  });
  it('is capped past the edge (pointer outside the scroller)', () => {
    expect(edgeSpeed(1200, 0, 1000)).toBe(SCROLL_MAX_SPEED);
    expect(edgeSpeed(-50, 0, 1000)).toBe(-SCROLL_MAX_SPEED);
  });
});

function lane(id: string, cards: Array<{ id: string; top: number; chain?: string }>, drop?: string) {
  const section = document.createElement('section');
  section.dataset.lanePage = id;
  if (drop) section.dataset.drop = drop;
  const list = document.createElement('div');
  list.className = 'lane-cards';
  for (const card of cards) {
    const node = document.createElement('article');
    node.className = 'bcard';
    node.dataset.id = card.id;
    node.dataset.chain = card.chain ?? 'single';
    node.getBoundingClientRect = () => ({ top: card.top, height: 40, bottom: card.top + 40, left: 0, right: 100, width: 100, x: 0, y: card.top, toJSON() {} });
    list.append(node);
  }
  section.append(list);
  document.body.append(section);
  return section;
}

describe('dropTargetFrom', () => {
  it('in the strip beside a column (≤ 40 px) snaps to the nearest column', () => {
    const strip = document.createElement('div');
    strip.className = 'lane-pager';
    const el = lane('cancelled', [{ id: 'A', top: 0 }]);
    el.getBoundingClientRect = () => ({ top: 0, bottom: 500, left: 600, right: 888, width: 288, height: 500, x: 600, y: 0, toJSON() {} });
    strip.append(el);
    document.body.append(strip);
    expect(dropTargetFrom(strip, 10, 'X', 910)).toEqual({ lane: 'cancelled', before: 'A' });
    expect(dropTargetFrom(strip, 10, 'X', 990)).toEqual({ lane: null, before: null });
    strip.remove();
  });
  it('finds the lane and the card the drop lands before (midpoint test), null = end', () => {
    const el = lane('todo', [{ id: 'A', top: 0 }, { id: 'B', top: 50 }, { id: 'C', top: 100 }]);
    expect(dropTargetFrom(el, 10, 'X')).toEqual({ lane: 'todo', before: 'A' });
    expect(dropTargetFrom(el.querySelector('.bcard')!, 30, 'X')).toEqual({ lane: 'todo', before: 'B' });
    expect(dropTargetFrom(el, 500, 'X')).toEqual({ lane: 'todo', before: null });
    el.remove();
  });
  it('skips the dragged card and snaps into chain starts', () => {
    const el = lane('todo', [{ id: 'A', top: 0 }, { id: 'B', top: 50, chain: 'first' }, { id: 'C', top: 100, chain: 'last' }]);
    expect(dropTargetFrom(el, 110, 'X')).toEqual({ lane: 'todo', before: 'B' });
    expect(dropTargetFrom(el, 10, 'A')).toEqual({ lane: 'todo', before: 'B' });
    el.remove();
  });
  it('a refused lane has no insert position; outside any lane is no target', () => {
    const el = lane('done', [{ id: 'A', top: 0 }], 'refused');
    expect(dropTargetFrom(el, 10, 'X')).toEqual({ lane: 'done', before: null });
    expect(dropTargetFrom(document.body, 10, 'X')).toEqual({ lane: null, before: null });
    expect(dropTargetFrom(null, 10, 'X')).toEqual({ lane: null, before: null });
    el.remove();
  });
});
