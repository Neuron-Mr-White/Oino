import { describe, expect, it, vi } from 'vitest';
import {
  annexBNals,
  annexBToAvc,
  avcDescription,
  sameBytes,
  GestureMachine,
  ScreenSession,
  clampView,
  encodeFrame,
  fitView,
  hardwareKey,
  imeToCommands,
  keysymFor,
  oneToOneView,
  parseFrame,
  pressCommands,
  problemFromClose,
  problemFromError,
  statusView,
  toScreen,
  viewportFor,
  zoomAt,
  type ScreenTransport,
  type SessionState,
  type StreamMsg,
} from './screen';
import type { DesktopStatusData } from '@unipi/protocol';

const ready: DesktopStatusData = { enabled: true, view_only: false, compiled: ['portal', 'x11'], display: 'wayland', backend: 'portal', consent: 'needed' };

describe('statusView', () => {
  it('maps every status', () => {
    expect(statusView(undefined)).toEqual({ kind: 'loading' });
    expect(statusView({ ok: true, value: ready }).kind).toBe('ready');
    expect(statusView({ ok: true, value: { ...ready, enabled: false } }).kind).toBe('disabled');
    expect(statusView({ ok: false, error: { code: 'E_DESKTOP_DISABLED', message: 'off', retryable: false } }).kind).toBe('disabled');
    const v = statusView({ ok: true, value: { ...ready, backend: undefined, reason: 'no_display', hint: 'log in' } });
    expect(v).toMatchObject({ kind: 'unavailable', reason: 'no_display', hint: 'log in' });
    const old = statusView({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown op', retryable: false } });
    expect(old.kind === 'error' && old.error.message).toContain('older unipi-host');
    expect(statusView({ ok: false, error: { code: 'E_TRANSPORT', message: 'gone', retryable: true } }).kind).toBe('error');
  });
});

describe('problems', () => {
  it('maps host errors and close reasons', () => {
    expect(problemFromError({ code: 'E_DESKTOP_DISABLED', message: 'off' }).kind).toBe('disabled');
    expect(problemFromError({ code: 'E_DESKTOP_UNAVAILABLE', message: 'x', reason: 'denied' }).kind).toBe('denied');
    expect(problemFromError({ code: 'E_DESKTOP_UNAVAILABLE', message: 'x', reason: 'no_display' }).kind).toBe('no-display');
    expect(problemFromError({ code: 'E_DESKTOP_UNAVAILABLE', message: 'x', reason: 'not_compiled' }).kind).toBe('not-compiled');
    expect(problemFromError({ code: 'E_DESKTOP_UNAVAILABLE', message: 'x', reason: 'encoder_failed' }).kind).toBe('encoder');
    expect(problemFromError({ code: 'E_TRANSPORT', message: '' }).message).toContain('lost');
    expect(problemFromClose('replaced').kind).toBe('replaced');
    expect(problemFromClose('capture_ended').kind).toBe('ended');
  });
});

describe('frames', () => {
  it('round-trips the 13-byte header and checks Annex-B', () => {
    const au = new Uint8Array([0, 0, 0, 1, 0x67, 0x42, 0, 0x1f]);
    const f = parseFrame(encodeFrame(70000, true, 123456, au))!;
    expect(f).toMatchObject({ seq: 70000, key: true, tsMs: 123456 });
    expect([...f.data]).toEqual([...au]);
    expect(parseFrame(encodeFrame(1, false, 0, new Uint8Array([9, 9, 9, 9])))).toBeUndefined();
    expect(parseFrame(new Uint8Array(5))).toBeUndefined();
  });
});

describe('view transform', () => {
  const box = { width: 400, height: 800 };
  const screen = { width: 1920, height: 1080 };
  it('fits, zooms around a point, clamps and maps taps', () => {
    const fit = fitView(box, screen);
    expect(fit.scale).toBeCloseTo(400 / 1920);
    expect(fit.x).toBe(0);
    expect(fit.y).toBeCloseTo((800 - 1080 * fit.scale) / 2);
    // Centre of the box = centre of the screen.
    expect(toScreen(200, 400, fit, screen)).toEqual({ x: 0.5, y: 0.5 });
    // Outside the picture clamps to the edge.
    expect(toScreen(200, 0, fit, screen).y).toBe(0);
    const z = zoomAt(fit, 3, 100, 400, box, screen);
    expect(z.scale).toBeCloseTo(fit.scale * 3);
    // The point under the fingers stays put.
    const before = toScreen(100, 400, fit, screen);
    const after = toScreen(100, 400, z, screen);
    expect(after.x).toBeCloseTo(before.x, 2);
    // Can't zoom out past fit.
    expect(zoomAt(fit, 0.1, 200, 400, box, screen).scale).toBeCloseTo(fit.scale);
    // Pans stay inside the picture.
    const panned = clampView({ ...z, x: 5000 }, box, screen);
    expect(panned.x).toBe(0);
    const one = oneToOneView(box, screen, 2);
    expect(one.scale).toBe(0.5);
  });
  it('viewport hint is the box in device pixels', () => {
    expect(viewportFor({ width: 412, height: 780 }, 2.625)).toEqual({ width: 1082, height: 2048 });
    expect(viewportFor({ width: 1, height: 1 }, NaN)).toEqual({ width: 16, height: 16 });
  });
});

describe('gestures', () => {
  const kinds = (a: { kind: string; button?: string; down?: boolean }[]) => a.map((x) => (x.kind === 'button' ? `${x.button}${x.down ? '↓' : '↑'}` : x.kind));
  it('tap = left click at the tap point', () => {
    const g = new GestureMachine();
    expect(kinds(g.down(1, { x: 10, y: 10 }, 0))).toEqual(['long-press-armed']);
    expect(g.move(1, { x: 13, y: 12 })).toEqual([]);
    const up = g.up(1, { x: 13, y: 12 }, 100);
    expect(kinds(up)).toEqual(['move', 'left↓', 'left↑']);
    expect(up[0]).toEqual({ kind: 'move', x: 10, y: 10 });
  });
  it('long-press = right click, no left click after', () => {
    const g = new GestureMachine();
    g.down(1, { x: 50, y: 60 }, 0);
    expect(kinds(g.longPress())).toEqual(['move', 'right↓', 'right↑']);
    expect(g.up(1, { x: 50, y: 60 }, 700)).toEqual([]);
    // A moved finger can't long-press.
    g.down(1, { x: 0, y: 0 }, 0);
    g.move(1, { x: 40, y: 0 });
    expect(g.longPress()).toEqual([]);
  });
  it('drag = left button held while moving', () => {
    const g = new GestureMachine();
    g.down(1, { x: 0, y: 0 }, 0);
    expect(kinds(g.move(1, { x: 30, y: 0 }))).toEqual(['move', 'left↓', 'move']);
    expect(kinds(g.move(1, { x: 60, y: 5 }))).toEqual(['move']);
    expect(kinds(g.up(1, { x: 60, y: 5 }, 300))).toEqual(['move', 'left↑']);
  });
  it('two-finger tap = right click', () => {
    const g = new GestureMachine();
    g.down(1, { x: 100, y: 100 }, 0);
    g.down(2, { x: 160, y: 100 }, 20);
    expect(g.up(2, { x: 160, y: 100 }, 120)).toEqual([]);
    expect(kinds(g.up(1, { x: 100, y: 100 }, 130))).toEqual(['move', 'right↓', 'right↑']);
  });
  it('two-finger drag = scroll (content follows fingers)', () => {
    const g = new GestureMachine();
    g.down(1, { x: 100, y: 300 }, 0);
    g.down(2, { x: 160, y: 300 }, 0);
    g.move(1, { x: 100, y: 280 });
    const a = g.move(2, { x: 160, y: 280 });
    const b = g.move(1, { x: 100, y: 200 });
    const scrolls = [...a, ...b].filter((x) => x.kind === 'scroll');
    expect(scrolls.length).toBeGreaterThan(0);
    expect(scrolls.every((s) => s.kind === 'scroll' && s.dy > 0)).toBe(true);
    g.up(1, { x: 100, y: 200 }, 500);
    expect(g.up(2, { x: 160, y: 200 }, 500)).toEqual([]);
  });
  it('pinch = local zoom, no clicks', () => {
    const g = new GestureMachine();
    g.down(1, { x: 100, y: 300 }, 0);
    g.down(2, { x: 140, y: 300 }, 0);
    const out = [...g.move(2, { x: 200, y: 300 }), ...g.move(1, { x: 60, y: 300 })];
    const zooms = out.filter((x) => x.kind === 'zoom');
    expect(zooms.length).toBeGreaterThan(0);
    expect(zooms.some((z) => z.kind === 'zoom' && z.factor > 1)).toBe(true);
    expect(out.some((x) => x.kind === 'button' || x.kind === 'scroll')).toBe(false);
  });
  it('second finger during a drag releases the button', () => {
    const g = new GestureMachine();
    g.down(1, { x: 0, y: 0 }, 0);
    g.move(1, { x: 50, y: 0 });
    expect(kinds(g.down(2, { x: 100, y: 0 }, 10))).toEqual(['left↑']);
    expect(kinds(g.cancel())).toEqual([]);
  });
});

describe('keys', () => {
  it('maps DOM keys to keysyms', () => {
    expect(keysymFor('a')).toBe(0x61);
    expect(keysymFor('Enter')).toBe(0xff0d);
    expect(keysymFor('é')).toBe(0xe9);
    expect(keysymFor('€')).toBe(0x010020ac);
    expect(keysymFor('Dead')).toBeUndefined();
    expect(hardwareKey({ key: 'Control' }, true)).toEqual({ op: 'key', keysym: 0xffe3, down: true });
    expect(hardwareKey({ key: 'x', isComposing: true }, true)).toBeUndefined();
  });
  it('wraps presses in sticky modifiers', () => {
    expect(pressCommands(0x63, ['ctrl'])).toEqual([
      { op: 'key', keysym: 0xffe3, down: true },
      { op: 'key', keysym: 0x63, down: true },
      { op: 'key', keysym: 0x63, down: false },
      { op: 'key', keysym: 0xffe3, down: false },
    ]);
  });
  it('turns IME diffs into text and BackSpaces', () => {
    expect(imeToCommands('hi', [])).toEqual([{ op: 'text', text: 'hi' }]);
    const cmds = imeToCommands('\x7fab\n', []);
    expect(cmds[0]).toEqual({ op: 'key', keysym: 0xff08, down: true });
    expect(cmds).toContainEqual({ op: 'text', text: 'ab' });
    expect(cmds.at(-1)).toEqual({ op: 'key', keysym: 0xff0d, down: false });
    // With Ctrl held, letters go as key presses (Ctrl+C).
    expect(imeToCommands('c', ['ctrl'])).toEqual(pressCommands(0x63, ['ctrl']));
  });
});

describe('ScreenSession', () => {
  function fake() {
    let onMsg: (m: StreamMsg) => void = () => {};
    let onFrame: (b: Uint8Array) => void = () => {};
    const cmds: unknown[] = [];
    const t: ScreenTransport = {
      status: vi.fn(),
      open: vi.fn(async (_h, _a, m, f) => {
        onMsg = m;
        onFrame = f;
        return { ok: true as const, value: { streamId: 's1' } };
      }),
      cmd: vi.fn(async (_id, c) => {
        cmds.push(c);
      }),
      stop: vi.fn(),
    };
    return { t, cmds, msg: (m: StreamMsg) => onMsg(m), frame: (b: Uint8Array) => onFrame(b) };
  }

  it('walks consent → live → frames (acked) → closed', async () => {
    const f = fake();
    const states: SessionState[] = [];
    const drawn: number[] = [];
    const s = new ScreenSession('h', { onState: (x) => states.push(x), onFrame: (fr) => void drawn.push(fr.seq) }, f.t);
    void s.send({ op: 'quality', quality: 'low' });
    await s.open({ viewport: { width: 400, height: 800 } });
    expect(f.cmds).toContainEqual({ op: 'quality', quality: 'low' });
    f.msg({ kind: 'event', event: { kind: 'consent', state: 'waiting' } });
    f.msg({ kind: 'event', event: { kind: 'started', backend: 'portal', width: 1920, height: 1080, view_only: false } });
    f.frame(encodeFrame(3, true, 0, new Uint8Array([0, 0, 0, 1, 0x65])));
    await new Promise((r) => setTimeout(r, 0));
    expect(drawn).toEqual([3]);
    expect(f.cmds).toContainEqual({ op: 'ack', seq: 3 });
    f.msg({ kind: 'event', event: { kind: 'closed', reason: 'replaced' } });
    expect(states.map((x) => x.kind)).toEqual(['connecting', 'consent', 'live', 'problem']);
    expect(states.at(-1)).toMatchObject({ problem: { kind: 'replaced' } });
    // Ended: nothing more goes out.
    const n = f.cmds.length;
    await s.send({ op: 'keyframe' });
    expect(f.cmds.length).toBe(n);
  });

  it('maps open refusals and stream errors', async () => {
    const t: ScreenTransport = {
      status: vi.fn(),
      open: vi.fn(async () => ({ ok: false as const, error: { code: 'E_DESKTOP_DISABLED' as const, message: 'off', retryable: false } })),
      cmd: vi.fn(),
      stop: vi.fn(),
    };
    const states: SessionState[] = [];
    await new ScreenSession('h', { onState: (x) => states.push(x), onFrame: () => {} }, t).open({ viewport: { width: 400, height: 800 } });
    expect(states.at(-1)).toMatchObject({ kind: 'problem', problem: { kind: 'disabled' } });

    const f = fake();
    const s2: SessionState[] = [];
    await new ScreenSession('h', { onState: (x) => s2.push(x), onFrame: () => {} }, f.t).open({ viewport: { width: 400, height: 800 } });
    f.msg({ kind: 'error', message: 'Cancelled', code: 'E_DESKTOP_UNAVAILABLE', reason: 'denied', hint: 'press Share' });
    expect(s2.at(-1)).toMatchObject({ kind: 'problem', problem: { kind: 'denied', hint: 'press Share' } });
  });
});

describe('H.264 Annex-B → AVC (WebKitGTK decodes only AVC)', () => {
  const sps = [0x67, 0x42, 0xc0, 0x28, 0xda, 0x01];
  const pps = [0x68, 0xce, 0x3c, 0x80];
  const idr = [0x65, 0x88, 0x84, 0x00, 0x10];
  const key = new Uint8Array([0, 0, 0, 1, ...sps, 0, 0, 1, ...pps, 0, 0, 0, 1, ...idr]);
  it('splits NAL units on 3- and 4-byte start codes', () => {
    expect(annexBNals(key).map((n) => Array.from(n))).toEqual([sps, pps, idr]);
  });
  it('builds the avcC description from SPS + PPS', () => {
    const d = avcDescription(key)!;
    expect(Array.from(d)).toEqual([1, 0x42, 0xc0, 0x28, 0xff, 0xe1, 0, sps.length, ...sps, 1, 0, pps.length, ...pps]);
    expect(avcDescription(new Uint8Array([0, 0, 0, 1, ...idr]))).toBeUndefined();
  });
  it('length-prefixes the slices and drops parameter sets / AUD', () => {
    expect(Array.from(annexBToAvc(key))).toEqual([0, 0, 0, idr.length, ...idr]);
    const delta = new Uint8Array([0, 0, 0, 1, 0x09, 0xf0, 0, 0, 0, 1, 0x41, 0x9a, 0x02]);
    expect(Array.from(annexBToAvc(delta))).toEqual([0, 0, 0, 3, 0x41, 0x9a, 0x02]);
  });
  it('sameBytes', () => {
    expect(sameBytes(new Uint8Array([1, 2]), new Uint8Array([1, 2]))).toBe(true);
    expect(sameBytes(new Uint8Array([1, 2]), new Uint8Array([1, 3]))).toBe(false);
    expect(sameBytes(undefined, new Uint8Array([1]))).toBe(false);
  });
});

describe('displayLabel (UNI-252, cross-OS hosts)', () => {
  it('names the session', async () => {
    const { displayLabel } = await import('./screen');
    expect(displayLabel({ display: 'wayland', backend: 'mutter' })).toBe('GNOME');
    expect(displayLabel({ display: 'wayland', backend: 'portal' })).toBe('Wayland');
    expect(displayLabel({ display: 'x11', backend: 'x11' })).toBe('X11');
    expect(displayLabel({ display: 'macos', backend: 'macos' })).toBe('macOS');
    expect(displayLabel({ display: 'windows', backend: 'windows' })).toBe('Windows');
    expect(displayLabel({ display: 'none' })).toBe('Desktop');
  });
  it('accepts macOS / Windows hosts on the wire (proto 1.12)', async () => {
    const { DesktopStatusDataSchema } = await import('@unipi/protocol');
    for (const display of ['macos', 'windows'] as const) {
      expect(DesktopStatusDataSchema.safeParse({ enabled: true, view_only: false, compiled: [display], display, backend: display, consent: 'not_needed' }).success).toBe(true);
    }
  });
});
