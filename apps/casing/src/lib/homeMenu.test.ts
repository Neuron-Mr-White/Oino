import { describe, it, expect } from 'vitest';
import { homeMenuItems } from './homeMenu';
import { knobRelease, DISMISS_FRACTION } from './knobDrag';

describe('home long-press menu', () => {
  it('ongoing: open, rename, close connection (detach)', () => {
    expect(homeMenuItems('ongoing', { canRename: true }).map((i) => i.action)).toEqual([
      'open',
      'rename',
      'close-connection',
    ]);
  });
  it('recent: reconnect, rename, remove from recents', () => {
    const items = homeMenuItems('recent', { canRename: true });
    expect(items.map((i) => i.label)).toEqual(['Reconnect', 'Rename label', 'Remove from Recents']);
  });
  it('never offers a host-side close/kill', () => {
    for (const group of ['ongoing', 'recent'] as const) {
      for (const canRename of [true, false]) {
        const labels = homeMenuItems(group, { canRename }).map((i) => `${i.action} ${i.label}`);
        expect(labels.join('|')).not.toMatch(/kill|close session|workspace_close/i);
      }
    }
  });
});

describe('knob drag release', () => {
  const height = 800;
  it('a short slow drag snaps back', () => {
    expect(knobRelease({ offset: 60, height, velocity: 0.1 })).toBe('snap-back');
  });
  it('past the threshold dismisses', () => {
    expect(knobRelease({ offset: height * DISMISS_FRACTION + 1, height, velocity: 0 })).toBe('dismiss');
  });
  it('a fast downward fling dismisses even when short', () => {
    expect(knobRelease({ offset: 40, height, velocity: 0.9 })).toBe('dismiss');
  });
  it('a tiny flick does not dismiss', () => {
    expect(knobRelease({ offset: 10, height, velocity: 2 })).toBe('snap-back');
  });
  it('dragging back up or a cancelled pointer snaps back', () => {
    expect(knobRelease({ offset: 500, height, velocity: -0.6 })).toBe('snap-back');
    expect(knobRelease({ offset: 500, height, velocity: 1, cancelled: true })).toBe('snap-back');
  });
});
