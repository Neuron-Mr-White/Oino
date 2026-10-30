import { describe, expect, it } from 'vitest';
import {
  chordCommands,
  drawRect,
  edgePan,
  followCursor,
  HeldKeys,
  initialQuality,
  isEscapeHatch,
  loadPrefs,
  qualityOptions,
  savePrefs,
  TrackpadMachine,
  viewportForView,
  zoomPercent,
  DOUBLE_TAP_MS,
} from './desktopUx';
import { fitView, GestureMachine, oneToOneView } from './screen';

describe('quality presets (UNI-271)', () => {
  it('new hosts open at Best quality, old hosts at auto; a remembered preset wins when known', () => {
    expect(qualityOptions({ features: ['presets'] }).map((o) => o.value)).toEqual(['best', 'balanced', 'speed']);
    expect(qualityOptions({}).map((o) => o.value)).toEqual(['auto', 'low', 'medium', 'high']);
    expect(initialQuality({ features: ['presets'] })).toBe('best');
    expect(initialQuality({ features: ['presets'] }, 'speed')).toBe('speed');
    expect(initialQuality({ features: ['presets'] }, 'high'), 'old preset → default').toBe('best');
    expect(initialQuality(undefined, 'best'), 'old host never gets a preset it rejects').toBe('auto');
    expect(initialQuality(undefined, 'medium')).toBe('medium');
  });
});

describe('view (UNI-271)', () => {
  const screen = { width: 3440, height: 1440 };
  it('viewport hint follows the zoom (device px)', () => {
    const box = { width: 412, height: 800 };
    const fit = fitView(box, screen);
    expect(viewportForView(box, 2.625, fit, screen)).toEqual({ width: 1082, height: 2100 });
    const one = oneToOneView(box, screen, 2.625);
    expect(viewportForView(box, 2.625, one, screen)).toEqual({ width: 3440, height: 2100 });
    expect(zoomPercent(one, 2.625)).toBe(100);
    expect(zoomPercent(fit, 1)).toBe(12);
  });
  it('draws 1:1 pixel-exact (integer rect, no smoothing) and smooths when scaled', () => {
    const one = oneToOneView({ width: 1440, height: 860 }, screen, 1);
    const r = drawRect(one, screen, 1);
    expect(r).toMatchObject({ w: 3440, h: 1440, smooth: false });
    expect(Number.isInteger(r.x) && Number.isInteger(r.y)).toBe(true);
    expect(drawRect(oneToOneView({ width: 412, height: 800 }, screen, 2.625), screen, 2.625).smooth).toBe(false);
    expect(drawRect(fitView({ width: 1440, height: 860 }, screen), screen, 1).smooth).toBe(true);
  });
  it('mouse mode keeps the cursor on screen when zoomed; desktop edge pan', () => {
    const box = { width: 400, height: 300 };
    const v = { scale: 1, x: 0, y: 0 };
    const moved = followCursor(v, box, screen, { x: 0.5, y: 0.5 });
    // Cursor at PC (1720, 720) must be inside the box now.
    expect(moved.x + 1720).toBeGreaterThan(0);
    expect(moved.x + 1720).toBeLessThanOrEqual(400);
    expect(moved.y + 720).toBeLessThanOrEqual(300);
    expect(edgePan({ x: 200, y: 150 }, box)).toEqual({ dx: 0, dy: 0 });
    expect(edgePan({ x: 2, y: 150 }, box).dx).toBeGreaterThan(0);
    expect(edgePan({ x: 399, y: 150 }, box).dx).toBeLessThan(0);
  });
});

describe('keys (UNI-271)', () => {
  it('chords press in order and release in reverse', () => {
    expect(chordCommands([1, 2])).toEqual([
      { op: 'key', keysym: 1, down: true },
      { op: 'key', keysym: 2, down: true },
      { op: 'key', keysym: 2, down: false },
      { op: 'key', keysym: 1, down: false },
    ]);
  });
  it('escape hatch is Ctrl+Shift+F12 only', () => {
    expect(isEscapeHatch({ key: 'F12', ctrlKey: true, shiftKey: true })).toBe(true);
    expect(isEscapeHatch({ key: 'F12', ctrlKey: true })).toBe(false);
    expect(isEscapeHatch({ key: 'Escape' })).toBe(false);
  });
  it('held keys are released (once)', () => {
    const h = new HeldKeys();
    h.track({ op: 'key', keysym: 0xffe3, down: true });
    h.track({ op: 'key', keysym: 0x61, down: true });
    h.track({ op: 'key', keysym: 0x61, down: false });
    h.track({ op: 'pointer', x: 0, y: 0 });
    expect(h.releaseAll()).toEqual([{ op: 'key', keysym: 0xffe3, down: false }]);
    expect(h.releaseAll()).toEqual([]);
  });
});

describe('trackpad (mouse mode, UNI-271)', () => {
  const geo = () => ({ scale: 0.25, screen: { width: 2000, height: 1000 } });
  it('one finger moves the cursor relatively; tap clicks at the cursor', () => {
    const t = new TrackpadMachine(geo);
    expect(t.down(1, { x: 100, y: 100 }, 0)).toEqual([{ kind: 'long-press-armed' }]);
    expect(t.move(1, { x: 105, y: 100 })).toEqual([]); // inside the slop
    const a = t.move(1, { x: 140, y: 100 });
    expect(a[0]!.kind).toBe('cursor');
    expect(t.cursor.x).toBeGreaterThan(0.5);
    expect(t.cursor.y).toBe(0.5);
    expect(t.up(1, { x: 140, y: 100 }, 200)).toEqual([]);
    const before = { ...t.cursor };
    t.down(1, { x: 50, y: 50 }, 1000);
    expect(t.up(1, { x: 50, y: 50 }, 1080)).toEqual([
      { kind: 'button', button: 'left', down: true },
      { kind: 'button', button: 'left', down: false },
    ]);
    expect(t.cursor).toEqual(before);
  });
  it('tap then touch-and-drag holds the left button (select / move windows)', () => {
    const t = new TrackpadMachine(geo);
    t.down(1, { x: 50, y: 50 }, 0);
    t.up(1, { x: 50, y: 50 }, 60);
    expect(t.down(1, { x: 52, y: 52 }, 60 + DOUBLE_TAP_MS - 50)).toEqual([]);
    const a = t.move(1, { x: 120, y: 52 });
    expect(a[0]).toEqual({ kind: 'button', button: 'left', down: true });
    expect(a[1]!.kind).toBe('cursor');
    expect(t.up(1, { x: 120, y: 52 }, 900)).toEqual([{ kind: 'button', button: 'left', down: false }]);
  });
  it('long-press and two-finger tap = right click; two-finger drag scrolls', () => {
    const t = new TrackpadMachine(geo);
    t.down(1, { x: 50, y: 50 }, 0);
    expect(t.longPress()).toEqual([
      { kind: 'button', button: 'right', down: true },
      { kind: 'button', button: 'right', down: false },
    ]);
    expect(t.up(1, { x: 50, y: 50 }, 700)).toEqual([]);
    t.down(1, { x: 50, y: 50 }, 1000);
    t.down(2, { x: 150, y: 50 }, 1010);
    t.up(1, { x: 50, y: 50 }, 1100);
    expect(t.up(2, { x: 150, y: 50 }, 1110)).toEqual([
      { kind: 'button', button: 'right', down: true },
      { kind: 'button', button: 'right', down: false },
    ]);
    t.down(1, { x: 50, y: 200 }, 2000);
    t.down(2, { x: 150, y: 200 }, 2000);
    const s = [...t.move(1, { x: 50, y: 160 }), ...t.move(2, { x: 150, y: 160 }), ...t.move(1, { x: 50, y: 100 }), ...t.move(2, { x: 150, y: 100 })];
    expect(s.some((x) => x.kind === 'scroll' && x.dy > 0)).toBe(true);
  });
});

describe('touch mode when zoomed (UNI-271)', () => {
  it('one-finger drag pans the zoomed picture; tap-tap-drag still drags on the PC', () => {
    let zoomed = true;
    const g = new GestureMachine(() => zoomed);
    g.down(1, { x: 100, y: 100 }, 0);
    expect(g.move(1, { x: 140, y: 100 })).toEqual([{ kind: 'pan', dx: 40, dy: 0 }]);
    expect(g.move(1, { x: 150, y: 110 })).toEqual([{ kind: 'pan', dx: 10, dy: 10 }]);
    expect(g.up(1, { x: 150, y: 110 }, 300)).toEqual([]);
    // Tap, then touch again and drag → left drag.
    g.down(1, { x: 100, y: 100 }, 1000);
    expect(g.up(1, { x: 100, y: 100 }, 1050).some((a) => a.kind === 'button')).toBe(true);
    g.down(1, { x: 102, y: 101 }, 1200);
    const drag = g.move(1, { x: 160, y: 101 });
    expect(drag).toContainEqual({ kind: 'button', button: 'left', down: true });
    // Not zoomed: a drag is a left drag as before.
    zoomed = false;
    const h = new GestureMachine(() => zoomed);
    h.down(1, { x: 100, y: 100 }, 5000);
    expect(h.move(1, { x: 140, y: 100 })).toContainEqual({ kind: 'button', button: 'left', down: true });
  });
});

describe('prefs', () => {
  it('round-trips and survives garbage', () => {
    const store = new Map<string, string>();
    const s = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    expect(loadPrefs(s)).toEqual({ viewStyle: 'adaptive', pinned: false });
    savePrefs({ quality: 'best', viewStyle: 'original', mouseMode: 'mouse', pinned: true }, s);
    expect(loadPrefs(s)).toEqual({ quality: 'best', viewStyle: 'original', mouseMode: 'mouse', pinned: true });
    store.set('unipi.desktopView.v1', '{"quality":"ultra","viewStyle":7}');
    expect(loadPrefs(s)).toEqual({ quality: undefined, viewStyle: 'adaptive', mouseMode: undefined, pinned: false });
    store.set('unipi.desktopView.v1', 'not json');
    expect(loadPrefs(s).viewStyle).toBe('adaptive');
  });
});
