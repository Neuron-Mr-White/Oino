import { afterEach, describe, expect, it } from 'vitest';
import {
  BUBBLE_MAX,
  bubbleText,
  chatLink,
  DEFAULT_POS,
  floatingOn,
  freshBubbles,
  GENERIC_NTFY_BODY,
  hostForItem,
  loadPos,
  mascotWindowOn,
  overlayOn,
  place,
  savePos,
  setFloatingOn,
  setOverlayOn,
  snap,
  type Bounds,
} from './mascot';
import type { NeedsYouItem } from './needsYou';
import type { HostInfo } from './schemas';

const item = (over: Partial<NeedsYouItem> = {}): NeedsYouItem => ({
  id: 'h:1:agent_end:1',
  hostId: 'h',
  hostName: 'Workstation',
  pid: 1,
  session: 'auth',
  title: 'auth rewrite',
  kind: 'agent_end',
  group: 'finished',
  since: 1,
  ...over,
});

afterEach(() => localStorage.clear());

describe('bubbleText', () => {
  it('shows the summarized reply ntfy carried', () => {
    expect(bubbleText(item({ what: 'auth: Refactored the token refresh and all 42 tests pass.' }))).toBe('auth: Refactored the token refresh and all 42 tests pass.');
  });
  it('drops the "<session>: " prefix of the pi recap (the bubble heads with the session)', () => {
    expect(bubbleText(item({ what: 'auth rewrite: All 42 tests pass.' }))).toBe('All 42 tests pass.');
    expect(bubbleText(item({ what: 'Auth Rewrite: ok' }))).toBe('ok');
  });
  it('falls back to "finished / needs you" without a summary, or with the generic minimal-detail body', () => {
    expect(bubbleText(item({ what: undefined }))).toBe('auth rewrite finished');
    expect(bubbleText(item({ what: GENERIC_NTFY_BODY, group: 'waiting' }))).toBe('auth rewrite needs you');
    expect(bubbleText(item({ what: '   ' }))).toBe('auth rewrite finished');
  });
  it('collapses whitespace and caps long replies with an ellipsis (by code point)', () => {
    expect(bubbleText(item({ what: 'a\n\n  b' }))).toBe('a b');
    const long = '🦀'.repeat(300);
    const out = bubbleText(item({ what: long }));
    expect(Array.from(out)).toHaveLength(BUBBLE_MAX);
    expect(out.endsWith('…')).toBe(true);
  });
});

describe('freshBubbles', () => {
  const a = item({ id: 'a', since: 2 });
  const b = item({ id: 'b', since: 1, group: 'waiting' });
  const other = item({ id: 'c', group: 'other' });
  it('first look only seeds (no replay of history)', () => {
    expect(freshBubbles(undefined, [a, b], () => false)).toEqual([]);
  });
  it('new unread waiting/finished items, oldest first; read and "other" skipped', () => {
    expect(freshBubbles(new Set(), [a, b, other], () => false).map((i) => i.id)).toEqual(['b', 'a']);
    expect(freshBubbles(new Set(['a']), [a, b], () => false).map((i) => i.id)).toEqual(['b']);
    expect(freshBubbles(new Set(), [a, b], (id) => id === 'b').map((i) => i.id)).toEqual(['a']);
  });
});

describe('hostForItem', () => {
  const hosts: HostInfo[] = [
    { id: 'id-1', name: 'Workstation' } as HostInfo,
    { id: 'id-2', name: 'laptop' } as HostInfo,
  ];
  it('matches a paired id, or (ntfy items) the PC hostname case-insensitively', () => {
    expect(hostForItem(hosts, { hostId: 'id-2' })?.name).toBe('laptop');
    expect(hostForItem(hosts, { hostId: 'LAPTOP' })?.id).toBe('id-2');
  });
  it('no hosts → undefined', () => {
    expect(hostForItem([], { hostId: 'x' })).toBeUndefined();
  });
});

describe('snap / place (AssistiveTouch-style position)', () => {
  const b: Bounds = { width: 412, height: 915, top: 40, bottom: 80, margin: 6 };
  it('snaps to the nearer side and keeps y as a fraction of the free height', () => {
    expect(snap(10, 40, 56, b)).toEqual({ side: 'left', y: 0 });
    expect(snap(300, 915 - 80 - 56, 56, b)).toEqual({ side: 'right', y: 1 });
    const mid = snap(200, 40 + (915 - 120 - 56) / 2, 56, b);
    expect(mid.side).toBe('right'); // centre 228 > 206
    expect(mid.y).toBeCloseTo(0.5);
  });
  it('clamps y past the top/bottom limits', () => {
    expect(snap(0, -500, 56, b).y).toBe(0);
    expect(snap(0, 5000, 56, b).y).toBe(1);
  });
  it('place puts it at the margin on its side, and survives a rotation (same fraction)', () => {
    expect(place({ side: 'left', y: 0 }, 56, b)).toEqual({ left: 6, top: 40 });
    expect(place({ side: 'right', y: 1 }, 56, b)).toEqual({ left: 412 - 56 - 6, top: 915 - 80 - 56 });
    const land: Bounds = { ...b, width: 915, height: 412 };
    const p = place({ side: 'right', y: 0.5 }, 56, land);
    expect(p.left).toBe(915 - 56 - 6);
    expect(p.top).toBeGreaterThanOrEqual(40);
    expect(p.top + 56).toBeLessThanOrEqual(412 - 80);
  });
});

describe('remembered position + prefs', () => {
  it('round-trips the position; garbage → default', () => {
    expect(loadPos()).toEqual(DEFAULT_POS);
    savePos({ side: 'left', y: 0.25 });
    expect(loadPos()).toEqual({ side: 'left', y: 0.25 });
    localStorage.setItem('unipi.mascot.pos.v1', '{"side":"up","y":"x"}');
    expect(loadPos()).toEqual(DEFAULT_POS);
    localStorage.setItem('unipi.mascot.pos.v1', 'not json');
    expect(loadPos()).toEqual(DEFAULT_POS);
    localStorage.setItem('unipi.mascot.pos.v1', '{"side":"left","y":7}');
    expect(loadPos()).toEqual({ side: 'left', y: 1 });
  });
  it('floating crab defaults on; overlay and desktop window default off', () => {
    expect(floatingOn()).toBe(true);
    expect(overlayOn()).toBe(false);
    expect(mascotWindowOn()).toBe(false);
    setFloatingOn(false);
    setOverlayOn(true);
    expect(floatingOn()).toBe(false);
    expect(overlayOn()).toBe(true);
  });
});

describe('chatLink', () => {
  it('builds the v2 session deep link when the session is known, else the v1 chat form', () => {
    expect(chatLink({ hostId: 'my pc', pid: 42, session: 's-9' })).toBe(`unipi://s/${encodeURIComponent('my pc')}/s-9`);
    expect(chatLink({ hostId: 'h', pid: null, session: undefined })).toBe('unipi://chat?host=h');
    expect(chatLink({ hostId: 'my pc', pid: 42, session: undefined })).toBe('unipi://chat?host=my+pc&pid=42');
  });
});
