import { describe, expect, it, vi } from 'vitest';
import {
  BrowserSession,
  displayUrl,
  encodeFrame,
  fitRect,
  imeCommands,
  keyCommand,
  parseFrame,
  parseStreamMsg,
  problemFromClose,
  browserName,
  canShowWindow,
  INSTALL_STEPS,
  problemFromError,
  sameViewport,
  statusView,
  tabLabel,
  toPage,
  viewportFor,
  type BrowserTransport,
  type StreamMsg,
  windowShown,
} from './browser';
import { imeInitial } from './ime';

const status = (extra: Record<string, unknown> = {}) => ({
  enabled: true,
  browser: { name: 'Google Chrome', path: '/usr/bin/google-chrome' },
  running: false,
  headless: true,
  idle_minutes: 10,
  tabs: [],
  ...extra,
});

describe('statusView', () => {
  it('loading, disabled (flag or error), not found, error, ready', () => {
    expect(statusView(undefined)).toEqual({ kind: 'loading' });
    expect(statusView({ ok: true, value: status({ enabled: false }) }).kind).toBe('disabled');
    const off = { code: 'E_BROWSER_DISABLED' as const, message: 'off', retryable: false };
    expect(statusView({ ok: false, error: off })).toEqual({ kind: 'disabled', error: off });
    expect(statusView({ ok: true, value: status({ browser: undefined, install_hint: 'apt' }) })).toEqual({ kind: 'not-found', hint: 'apt' });
    const lost = { code: 'E_TRANSPORT' as const, message: 'gone', retryable: true };
    expect(statusView({ ok: false, error: lost })).toEqual({ kind: 'error', error: lost });
    expect(statusView({ ok: true, value: status() }).kind).toBe('ready');
  });
  it('explains an old host that does not know the op', () => {
    const view = statusView({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant browser_status', retryable: false } });
    expect(view.kind).toBe('error');
    expect(view.kind === 'error' && view.error.message).toMatch(/older agent host/);
  });
});

describe('problems', () => {
  it('maps errors and close reasons', () => {
    expect(problemFromError({ code: 'E_BROWSER_DISABLED', message: 'off', retryable: false }).kind).toBe('disabled');
    expect(problemFromError({ code: 'E_BROWSER_UNAVAILABLE', message: 'x', retryable: false, detail: { reason: 'not_found' }, hint: 'apt' })).toEqual({
      kind: 'not-found',
      message: 'x',
      hint: 'apt',
    });
    expect(problemFromError({ code: 'E_BROWSER_UNAVAILABLE', message: 'x', retryable: true, detail: { reason: 'crashed' } }).kind).toBe('crashed');
    // UNI-287: a launch failure carries the browser's own stderr tail.
    expect(problemFromError({
      code: 'E_BROWSER_UNAVAILABLE',
      message: 'Chromium exited before DevTools came up — it could not write its profile directory',
      retryable: true,
      detail: { reason: 'launch_failed', stderr: ['Failed to create /p/SingletonLock: Permission denied'] },
    })).toEqual({
      kind: 'crashed',
      message: 'Chromium exited before DevTools came up — it could not write its profile directory',
      stderr: ['Failed to create /p/SingletonLock: Permission denied'],
    });
    expect(problemFromError({ code: 'E_BROWSER_UNAVAILABLE', message: 'x', retryable: true, detail: { reason: 'launch_failed', stderr: [] } })).toMatchObject({ kind: 'crashed' });
    expect(problemFromError({ code: 'E_TRANSPORT', message: 'x', retryable: true }).kind).toBe('lost');
    expect(problemFromClose('idle').message).toMatch(/nobody was watching/);
    expect(problemFromClose('closed').kind).toBe('closed');
    // UNI-268: a relaunch (window shown/hidden on the PC) is not an error — the viewer reconnects.
    expect(problemFromClose('relaunch').kind).toBe('relaunch');
  });
});

describe('frames and geometry', () => {
  it('round-trips the 8-byte header and rejects non-JPEG', () => {
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 1, 2]);
    const frame = parseFrame(encodeFrame(70000, 412, 780, jpeg))!;
    expect(frame).toMatchObject({ seq: 70000, cssWidth: 412, cssHeight: 780 });
    expect([...frame.jpeg]).toEqual([...jpeg]);
    expect(parseFrame(encodeFrame(1, 1, 1, new Uint8Array([1, 2, 3, 4])))).toBeUndefined();
    expect(parseFrame(new Uint8Array(4))).toBeUndefined();
  });
  it('fits and maps points (letterboxed while a resize is in flight)', () => {
    expect(fitRect(400, 800, 400, 800)).toEqual({ x: 0, y: 0, width: 400, height: 800 });
    const r = fitRect(400, 800, 800, 800);
    expect(r).toEqual({ x: 0, y: 200, width: 400, height: 400 });
    expect(toPage(200, 400, { width: 400, height: 800 }, { width: 800, height: 800 })).toEqual({ x: 400, y: 400 });
    expect(toPage(200, 100, { width: 400, height: 800 }, { width: 800, height: 800 })).toBeUndefined();
    expect(toPage(10, 10, { width: 412, height: 780 }, { width: 412, height: 780 })).toEqual({ x: 10, y: 10 });
  });
  it('viewport clamps sizes and caps the scale', () => {
    expect(viewportFor(412.4, 780.6, 2.625, true)).toEqual({ width: 412, height: 781, scale: 2, mobile: true });
    expect(viewportFor(10, 99999, 0, false)).toEqual({ width: 50, height: 4096, scale: 1, mobile: false });
    expect(sameViewport(undefined, viewportFor(1, 1, 1, false))).toBe(false);
    expect(sameViewport(viewportFor(400, 400, 1, true), viewportFor(400, 400, 1, true))).toBe(true);
  });
});

describe('keys and text', () => {
  it('printables go as text, specials and shortcuts as key presses', () => {
    expect(keyCommand({ key: 'a' })).toEqual({ op: 'text', text: 'a' });
    expect(keyCommand({ key: 'é' })).toEqual({ op: 'text', text: 'é' });
    expect(keyCommand({ key: 'Enter', code: 'Enter' })).toEqual({ op: 'key', kind: 'press', key: 'Enter', code: 'Enter', modifiers: 0 });
    expect(keyCommand({ key: 'a', ctrlKey: true, code: 'KeyA' })).toEqual({ op: 'key', kind: 'press', key: 'a', code: 'KeyA', modifiers: 2 });
    expect(keyCommand({ key: 'ArrowLeft', shiftKey: true })).toMatchObject({ key: 'ArrowLeft', modifiers: 8 });
    expect(keyCommand({ key: 'Shift' })).toBeUndefined();
    expect(keyCommand({ key: 'a', isComposing: true })).toBeUndefined();
    expect(keyCommand({ key: 'Unidentified' })).toBeUndefined();
  });
  it('soft keyboard: diffs into Backspaces + insertText, newline → Enter', () => {
    let r = imeCommands('hel', imeInitial());
    expect(r.cmds).toEqual([{ op: 'text', text: 'hel' }]);
    r = imeCommands('help', r.state);
    expect(r.cmds).toEqual([{ op: 'text', text: 'p' }]);
    // Autocorrect rewrites the word.
    r = imeCommands('hello', imeCommands('helo', imeInitial()).state);
    expect(r.cmds).toEqual([{ op: 'key', kind: 'press', key: 'Backspace', code: 'Backspace', modifiers: 0 }, { op: 'text', text: 'lo' }]);
    r = imeCommands('a\n', imeInitial());
    expect(r.cmds.at(-1)).toMatchObject({ op: 'key', key: 'Enter' });
  });
  it('URL bar and tab labels', () => {
    expect(displayUrl('https://example.com/')).toBe('example.com');
    expect(displayUrl('http://127.0.0.1:5173/a?b')).toBe('127.0.0.1:5173/a?b');
    expect(displayUrl('about:blank')).toBe('');
    expect(tabLabel({ title: 'Docs', url: 'https://a/' })).toBe('Docs');
    expect(tabLabel({ title: '', url: 'https://a.dev/x' })).toBe('a.dev/x');
    expect(tabLabel({ title: 'about:blank', url: 'about:blank' })).toBe('New tab');
  });
  it('browser names: Chrome by name, never the other brands (UNI-268)', () => {
    expect(browserName('Google Chrome')).toBe('Google Chrome');
    expect(browserName('chrome')).toBe('Google Chrome');
    expect(browserName('chromium')).toBe('Chromium');
    expect(browserName('edge')).toBe('Microsoft Edge');
    expect(browserName('brave')).toBe('Chromium-based browser');
    expect(browserName('Brave Browser')).toBe('Chromium-based browser');
    expect(browserName(undefined)).toBe('Browser');
    expect(INSTALL_STEPS.map((s) => s.os)).toEqual(['Linux', 'macOS', 'Windows']);
    expect(JSON.stringify(INSTALL_STEPS).toLowerCase()).not.toContain('brave');
    expect(canShowWindow({ can_show_window: false })).toBe(false);
    expect(canShowWindow({})).toBe(true);
  });
  it('stream messages are validated', () => {
    expect(parseStreamMsg({ kind: 'event', event: { kind: 'tabs', tabs: [], active: 'T' } })).toBeTruthy();
    expect(parseStreamMsg({ kind: 'event', event: { kind: 'nope' } })).toBeUndefined();
    expect(parseStreamMsg({ kind: 'closed' })).toEqual({ kind: 'closed' });
  });
});

function fakeTransport(openResult: Awaited<ReturnType<BrowserTransport['open']>> = { ok: true, value: { streamId: 's1', open: { browser: { name: 'Google Chrome', path: '/c' }, headless: true, target: 'T1', tabs: [] } } }) {
  let onMsg: (m: StreamMsg) => void = () => {};
  let onFrame: (b: Uint8Array) => void = () => {};
  const t: BrowserTransport & { cmds: unknown[]; stopped: string[] } = {
    cmds: [],
    stopped: [],
    status: vi.fn(),
    close: vi.fn(),
    window: vi.fn(),
    open: vi.fn(async (_h, _a, m, f) => {
      onMsg = m;
      onFrame = f;
      return openResult;
    }),
    cmd: vi.fn(async (_id, cmd) => {
      t.cmds.push(cmd);
    }),
    stop: vi.fn((id) => {
      t.stopped.push(id);
    }),
  };
  return { t, msg: (m: StreamMsg) => onMsg(m), frame: (b: Uint8Array) => onFrame(b) };
}

describe('BrowserSession', () => {
  const viewport = { width: 412, height: 780, scale: 2, mobile: true };
  it('opens, forwards events, draws frames then acks them', async () => {
    const { t, msg, frame } = fakeTransport();
    const states: string[] = [];
    const events: string[] = [];
    const drawn: number[] = [];
    const s = new BrowserSession('h', { onState: (x) => states.push(x.kind), onEvent: (e) => events.push(e.kind), onFrame: (f) => void drawn.push(f.seq) }, t);
    await s.send({ op: 'reload' }); // queued before open
    await s.open({ viewport });
    expect(states).toEqual(['starting', 'live']);
    msg({ kind: 'event', event: { kind: 'tabs', tabs: [], active: 'T1' } });
    frame(encodeFrame(4, 412, 780, new Uint8Array([0xff, 0xd8, 0xff, 0])));
    await vi.waitFor(() => expect(t.cmds).toContainEqual({ op: 'ack', seq: 4 }));
    expect(t.cmds[0]).toEqual({ op: 'reload' });
    expect(events).toEqual(['tabs']);
    expect(drawn).toEqual([4]);
  });
  it('crash keeps the stream; close/error end it once', async () => {
    const { t, msg } = fakeTransport();
    const states: unknown[] = [];
    const s = new BrowserSession('h', { onState: (x) => states.push(x), onEvent: () => {}, onFrame: () => {} }, t);
    await s.open({ viewport });
    msg({ kind: 'event', event: { kind: 'crashed', message: 'The page crashed' } });
    expect(states.at(-1)).toEqual({ kind: 'problem', problem: { kind: 'crashed', message: 'The page crashed' } });
    msg({ kind: 'event', event: { kind: 'closed', reason: 'idle' } });
    expect(states.at(-1)).toMatchObject({ kind: 'problem', problem: { kind: 'closed' } });
    msg({ kind: 'error', message: 'later', code: 'E_TRANSPORT' });
    expect(states).toHaveLength(4);
    expect(s.isEnded).toBe(true);
  });
  it('open failure → a problem; stop() hangs up', async () => {
    const { t } = fakeTransport({ ok: false, error: { code: 'E_BROWSER_DISABLED', message: 'off', retryable: false } });
    const states: unknown[] = [];
    const s = new BrowserSession('h', { onState: (x) => states.push(x), onEvent: () => {}, onFrame: () => {} }, t);
    await s.open({ viewport });
    expect(states.at(-1)).toMatchObject({ kind: 'problem', problem: { kind: 'disabled' } });

    const ok = fakeTransport();
    const live = new BrowserSession('h', { onState: () => {}, onEvent: () => {}, onFrame: () => {} }, ok.t);
    await live.open({ viewport });
    live.stop();
    expect(ok.t.stopped).toEqual(['s1']);
  });
  it('a failing command ends the session as lost', async () => {
    const { t } = fakeTransport();
    t.cmd = vi.fn(async () => {
      throw { code: 'E_TRANSPORT', message: 'The browser view is not connected', retryable: true };
    });
    const states: unknown[] = [];
    const s = new BrowserSession('h', { onState: (x) => states.push(x), onEvent: () => {}, onFrame: () => {} }, t);
    await s.open({ viewport });
    await s.send({ op: 'back' });
    expect(states.at(-1)).toMatchObject({ kind: 'problem', problem: { kind: 'lost' } });
  });
});

describe('windowShown (UNI-299)', () => {
  it('trusts display when the host reports it', () => {
    expect(windowShown({ headless: false, display: 'hidden' })).toBe(false);
    expect(windowShown({ headless: false, display: 'window' })).toBe(true);
    expect(windowShown({ headless: true, display: 'headless' })).toBe(false);
  });
  it('falls back to headless for older hosts', () => {
    expect(windowShown({ headless: false })).toBe(true);
    expect(windowShown({ headless: true })).toBe(false);
  });
});
