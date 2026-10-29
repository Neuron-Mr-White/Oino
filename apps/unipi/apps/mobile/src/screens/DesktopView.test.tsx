// @vitest-environment jsdom
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Preview from './Preview';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { call } from '../lib/ipc';
import { encodeFrame, setScreenTransport, type ScreenTransport, type StreamMsg } from '../lib/screen';
import { setPlatformOverride } from '../lib/platform';
import type { Result } from '../lib/ipc';
import type { DesktopStatusData } from '@unipi/protocol';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [{ id: 'host-1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

class RO {
  observe() {}
  disconnect() {}
}
// A fake WebCodecs decoder: every chunk becomes a "frame".
const decoded: { type: string; data?: Uint8Array }[] = [];
const configs: Array<{ codec: string; description?: Uint8Array }> = [];
class FakeDecoder {
  state = 'unconfigured';
  constructor(private init: { output: (f: unknown) => void }) {}
  static async isConfigSupported() {
    return { supported: true };
  }
  configure(c: { codec: string; description?: Uint8Array }) {
    configs.push(c);
    this.state = 'configured';
  }
  decode(chunk: { type: string; data?: Uint8Array }) {
    decoded.push(chunk);
    this.init.output({ displayWidth: 1920, displayHeight: 1080, close() {} });
  }
  close() {
    this.state = 'closed';
  }
}
class FakeChunk {
  type: string;
  data: Uint8Array;
  constructor(init: { type: string; data: Uint8Array }) {
    this.type = init.type;
    this.data = init.data;
  }
}
Object.assign(globalThis, { ResizeObserver: RO });
HTMLCanvasElement.prototype.setPointerCapture = () => {};
HTMLCanvasElement.prototype.getContext = (() => ({ fillRect() {}, drawImage() {}, save() {}, restore() {}, beginPath() {}, arc() {}, stroke() {}, fillStyle: '' })) as never;

const status = (extra: Partial<DesktopStatusData> = {}): Result<DesktopStatusData> => ({
  ok: true,
  value: { enabled: true, view_only: false, compiled: ['portal', 'x11'], display: 'wayland', backend: 'portal', consent: 'granted', ...extra },
});

type Fake = ScreenTransport & { cmds: unknown[]; opened: unknown[]; msg: (m: StreamMsg) => void; frame: (b: Uint8Array) => void };
function fake(statusResult: Result<DesktopStatusData>, openResult?: Awaited<ReturnType<ScreenTransport['open']>>): Fake {
  const f: Fake = {
    cmds: [],
    opened: [],
    msg: () => {},
    frame: () => {},
    status: vi.fn(async () => statusResult),
    open: vi.fn(async (_h, args, onMsg, onFrame) => {
      f.opened.push(args);
      f.msg = onMsg;
      f.frame = onFrame;
      return openResult ?? { ok: true as const, value: { streamId: 's1' } };
    }),
    cmd: vi.fn(async (_id, cmd) => {
      f.cmds.push(cmd);
    }),
    stop: vi.fn(),
  };
  setScreenTransport(f);
  return f;
}

function mount() {
  location.hash = '#/preview?mode=desktop';
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <SettingsProvider><HostsProvider><Preview /></HostsProvider></SettingsProvider>} />
    </HashRouter>
  ));
}

// A key access unit as the PC sends it (Annex-B): SPS, PPS, IDR slice.
const AU = new Uint8Array([0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x28, 0xda, 0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80, 0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00]);

beforeEach(() => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  Object.assign(globalThis, { VideoDecoder: FakeDecoder, EncodedVideoChunk: FakeChunk });
  decoded.length = 0;
  configs.length = 0;
  vi.mocked(call).mockImplementation(async (cmd: string) =>
    (cmd === 'list_hosts' ? { ok: true, value: hosts } : cmd === 'forward_list' ? { ok: true, value: [] } : { ok: true, value: null }) as never,
  );
});
afterEach(() => {
  cleanup();
  setScreenTransport(undefined);
  setPlatformOverride(undefined);
  vi.clearAllMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  delete (globalThis as { VideoDecoder?: unknown }).VideoDecoder;
  location.hash = '#/';
});

async function startLive(f: Fake, extra: { view_only?: boolean } = {}) {
  fireEvent.click(await screen.findByTestId('desktop-start'));
  await screen.findByTestId('desktop-viewer');
  await waitFor(() => expect(f.opened.length).toBe(1));
  f.msg({ kind: 'event', event: { kind: 'started', backend: 'portal', width: 1920, height: 1080, view_only: !!extra.view_only } });
  f.msg({ kind: 'event', event: { kind: 'config', codec: 'avc1.42e028', width: 1920, height: 1080 } });
  f.frame(encodeFrame(1, true, 0, AU));
  await waitFor(() => expect(f.cmds).toContainEqual({ op: 'ack', seq: 1 }));
}

describe('Preview → Desktop mode (UNI-216)', () => {
  it('off on another PC: ask with a watch-only option (UNI-248), the command folded below', async () => {
    fake(status({ enabled: false }));
    mount();
    const notice = await screen.findByTestId('desktop-disabled');
    expect(notice.textContent).toContain('Desktop streaming is off on this PC');
    expect(notice.textContent).toContain('Watch only');
    expect(screen.getByTestId('share-ask')).toBeTruthy();
    fireEvent.click(screen.getByText('Or run this on the PC'));
    expect(notice.textContent).toContain('unipi-host desktop enable');
    expect(notice.textContent).toContain('unipi-host desktop disable');
    expect(screen.getByRole('tab', { name: /Desktop/ }).getAttribute('aria-selected')).toBe('true');
  });

  it('no display / not compiled explain themselves', async () => {
    fake(status({ backend: undefined, reason: 'no_display', hint: 'Log in to a desktop session on the PC' }));
    mount();
    const notice = await screen.findByTestId('desktop-unavailable-no_display');
    expect(notice.textContent).toContain('No desktop session');
    expect(notice.textContent).toContain('Log in');
    cleanup();
    fake(status({ backend: undefined, reason: 'not_compiled', compiled: [] }));
    mount();
    expect((await screen.findByTestId('desktop-unavailable-not_compiled')).textContent).toContain('built without');
  });

  it('ready on a Mac host: permissions note instead of the share dialog, labelled macOS', async () => {
    fake(status({ display: 'macos', backend: 'macos', compiled: ['macos'], consent: 'needed', hint: 'System Settings → Privacy & Security → Screen & System Audio Recording' }));
    mount();
    const note = await screen.findByTestId('desktop-consent-mac');
    expect(note.textContent).toContain('Screen & System Audio Recording');
    expect(note.textContent).toContain('Accessibility');
    expect(note.textContent).not.toContain('Share your screen');
    expect(screen.getByTestId('desktop-ready').textContent).toContain('macOS');
    cleanup();
    fake(status({ display: 'windows', backend: 'windows', compiled: ['windows'], consent: 'not_needed' }));
    mount();
    expect((await screen.findByTestId('desktop-ready')).textContent).toContain('Windows');
    expect(screen.queryByTestId('desktop-consent-note')).toBeNull();
  });

  it('ready: consent note when the PC will ask, then live with decoded frames and input', async () => {
    const f = fake(status({ consent: 'needed' }));
    mount();
    expect((await screen.findByTestId('desktop-consent-note')).textContent).toContain('Share');
    fireEvent.click(screen.getByTestId('desktop-start'));
    await screen.findByTestId('desktop-viewer');
    await waitFor(() => expect(f.opened.length).toBe(1));
    expect(f.opened[0]).toMatchObject({ viewOnly: false, quality: 'auto' });
    f.msg({ kind: 'event', event: { kind: 'consent', state: 'waiting' } });
    expect((await screen.findByTestId('desktop-consent')).textContent).toContain('Allow sharing on coffee');
    f.msg({ kind: 'event', event: { kind: 'started', backend: 'portal', width: 1920, height: 1080, view_only: false } });
    f.msg({ kind: 'event', event: { kind: 'config', codec: 'avc1.42e028', width: 1920, height: 1080 } });
    // A delta before any key frame is skipped by the decoder.
    f.frame(encodeFrame(1, false, 0, AU));
    f.frame(encodeFrame(2, true, 40, AU));
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'ack', seq: 2 }));
    expect(decoded.map((d) => d.type)).toEqual(['key']);
    // Fed as AVC (WebKitGTK only decodes that): avcC description + length-prefixed IDR.
    expect(configs.at(-1)!.codec).toBe('avc1.42e028');
    expect(Array.from(configs.at(-1)!.description!.slice(0, 6))).toEqual([1, 0x42, 0xc0, 0x28, 0xff, 0xe1]);
    expect(Array.from(decoded[0]!.data!)).toEqual([0, 0, 0, 4, 0x65, 0x88, 0x84, 0x00]);
    await waitFor(() => expect(screen.queryByTestId('desktop-connecting')).toBeNull());
    f.msg({ kind: 'event', event: { kind: 'stats', fps: 24, kbps: 1800, quality: 'auto', rtt_ms: 30 } });
    await waitFor(() => expect(screen.getByTestId('desktop-status-line').textContent).toBe('1920×1080 · 24 fps · 1.8 Mbps'));

    // Mouse: move + click map straight through.
    const canvas = screen.getByTestId('desktop-canvas');
    fireEvent.pointerDown(canvas, { pointerType: 'mouse', button: 0, clientX: 0, clientY: 0, pointerId: 1 });
    fireEvent.pointerUp(canvas, { pointerType: 'mouse', button: 0, clientX: 0, clientY: 0, pointerId: 1 });
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'button', button: 'left', down: false }));
    expect(f.cmds.some((c) => (c as { op: string }).op === 'pointer')).toBe(true);
    // Hardware keys go down/up as keysyms.
    fireEvent.keyDown(canvas, { key: 'a' });
    fireEvent.keyUp(canvas, { key: 'a' });
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'key', keysym: 0x61, down: false }));

    // Quality menu.
    fireEvent.click(screen.getByRole('button', { name: /Quality/ }));
    fireEvent.click(await screen.findByRole('menuitemradio', { name: /Low/ }));
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'quality', quality: 'low' }));

    // Keyboard row with sticky Ctrl.
    fireEvent.click(screen.getByRole('button', { name: 'Keyboard' }));
    const keys = await screen.findByTestId('desktop-keys');
    fireEvent.click(screen.getByRole('button', { name: 'Ctrl modifier' }));
    expect(screen.getByRole('button', { name: 'Ctrl modifier' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Escape' }));
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'key', keysym: 0xff1b, down: true }));
    const i = f.cmds.findIndex((c) => (c as { keysym?: number }).keysym === 0xffe3);
    expect(i).toBeGreaterThan(-1);
    expect(screen.getByRole('button', { name: 'Ctrl modifier' }).getAttribute('aria-pressed')).toBe('false');
    expect(keys).toBeTruthy();

    // View style: Scale adaptive → Original (1:1).
    expect(screen.getByTestId('desktop-zoom').getAttribute('aria-label')).toBe('View: Scale adaptive');
    fireEvent.click(screen.getByTestId('desktop-zoom'));
    fireEvent.click(await screen.findByRole('menuitemradio', { name: /Original/ }));
    expect(screen.getByTestId('desktop-zoom').getAttribute('aria-label')).toBe('View: Original');
  });

  it('UNI-271: a new host opens at Best quality; RustDesk presets, special keys, pin, escape hatch', async () => {
    localStorage.clear();
    const f = fake(status({ features: ['presets'] }));
    mount();
    await startLive(f);
    expect(f.opened[0]).toMatchObject({ quality: 'best' });
    // Presets menu.
    fireEvent.click(screen.getByTestId('desktop-quality'));
    const menu = await screen.findByTestId('desktop-quality-menu');
    expect(menu.textContent).toContain('Best quality');
    expect(menu.textContent).toContain('Balanced');
    expect(menu.textContent).toContain('Optimize reaction time');
    expect(screen.getByRole('menuitemradio', { name: /Best quality/ }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('menuitemradio', { name: /Balanced/ }));
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'quality', quality: 'balanced' }));
    // Remembered for next time.
    expect(JSON.parse(localStorage.getItem('unipi.desktopView.v1')!)).toMatchObject({ quality: 'balanced' });
    // Ctrl+Alt+Del: all down, then up in reverse.
    fireEvent.click(screen.getByTestId('desktop-special'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Ctrl + Alt + Del' }));
    const keys = f.cmds.filter((c) => (c as { op: string }).op === 'key');
    expect(keys).toEqual([
      { op: 'key', keysym: 0xffe3, down: true },
      { op: 'key', keysym: 0xffe9, down: true },
      { op: 'key', keysym: 0xffff, down: true },
      { op: 'key', keysym: 0xffff, down: false },
      { op: 'key', keysym: 0xffe9, down: false },
      { op: 'key', keysym: 0xffe3, down: false },
    ]);
    expect((await screen.findByTestId('desktop-toast')).textContent).toContain('Ctrl + Alt + Del');
    // Pin toolbar.
    fireEvent.click(screen.getByTestId('desktop-pin'));
    expect(screen.getByTestId('desktop-pin').getAttribute('aria-pressed')).toBe('true');
    // Keys held on the PC are released when the picture loses focus; Ctrl+Shift+F12 releases the keyboard.
    const canvas = screen.getByTestId('desktop-canvas');
    canvas.focus();
    fireEvent.keyDown(canvas, { key: 'Shift' });
    const before = f.cmds.length;
    fireEvent.keyDown(canvas, { key: 'F12', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(f.cmds.slice(before)).toContainEqual({ op: 'key', keysym: 0xffe1, down: false }));
    expect(f.cmds.slice(before)).not.toContainEqual({ op: 'key', keysym: 0xffc9, down: true });
    expect((await screen.findByTestId('desktop-toast')).textContent).toContain('Keyboard released');
  });

  it('UNI-271: old hosts keep the old quality list; display picker on multi-monitor hosts', async () => {
    localStorage.clear();
    let f = fake(status());
    mount();
    await startLive(f);
    expect(f.opened[0]).toMatchObject({ quality: 'auto' });
    expect(screen.queryByTestId('desktop-display')).toBeNull();
    fireEvent.click(screen.getByTestId('desktop-quality'));
    expect((await screen.findByTestId('desktop-quality-menu')).textContent).toContain('Update unipi-host');
    cleanup();
    localStorage.clear();
    f = fake(
      status({
        features: ['presets', 'displays'],
        displays: [
          { id: 'DP-1', name: 'Dell 34', width: 3440, height: 1440, primary: true },
          { id: 'HDMI-1', name: 'HDMI-1', width: 1920, height: 1080, primary: false },
        ],
      }),
    );
    mount();
    await startLive(f);
    fireEvent.click(screen.getByTestId('desktop-display'));
    const menu = await screen.findByTestId('desktop-display-menu');
    expect(menu.textContent).toContain('3440×1440 · primary');
    expect(screen.getByRole('menuitemradio', { name: /Dell 34/ }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('menuitemradio', { name: /HDMI-1/ }));
    await waitFor(() => expect(f.opened.length).toBe(2));
    expect(f.opened[1]).toMatchObject({ display: 'HDMI-1' });
  });

  it('UNI-271 phone: mouse mode = trackpad with L/R buttons; touch mode taps where you point', async () => {
    localStorage.clear();
    const mm = window.matchMedia;
    window.matchMedia = ((q: string) => ({ matches: q.includes('coarse'), media: q, addEventListener() {}, removeEventListener() {} })) as never;
    try {
      const f = fake(status({ features: ['presets'] }));
      mount();
      await startLive(f);
      expect(screen.getByTestId('desktop-fabs')).toBeTruthy();
      expect(screen.queryByRole('button', { name: /Left click/ })).toBeNull();
      fireEvent.click(screen.getByTestId('desktop-mouse-mode'));
      expect(screen.getByTestId('desktop-mouse-mode').getAttribute('aria-pressed')).toBe('true');
      const left = screen.getByRole('button', { name: /Left click/ });
      fireEvent.pointerDown(left, { pointerId: 9 });
      fireEvent.pointerUp(left, { pointerId: 9 });
      await waitFor(() => expect(f.cmds).toContainEqual({ op: 'button', button: 'left', down: false }));
      // The click lands at the cursor (centre until moved).
      expect(f.cmds).toContainEqual({ op: 'pointer', x: 0.5, y: 0.5 });
      const right = screen.getByRole('button', { name: 'Right click' });
      fireEvent.pointerDown(right, { pointerId: 10 });
      fireEvent.pointerUp(right, { pointerId: 10 });
      await waitFor(() => expect(f.cmds).toContainEqual({ op: 'button', button: 'right', down: false }));
      expect(JSON.parse(localStorage.getItem('unipi.desktopView.v1')!)).toMatchObject({ mouseMode: 'mouse' });
    } finally {
      window.matchMedia = mm;
    }
  });

  it('view-only: badge, no keyboard, no input sent', async () => {
    const f = fake(status({ view_only: true }));
    mount();
    expect((await screen.findByTestId('desktop-ready')).textContent).toContain('watch only');
    await startLive(f, { view_only: true });
    expect(screen.getByTestId('desktop-view-only')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Keyboard' })).toBeNull();
    expect(screen.queryByTestId('desktop-special')).toBeNull();
    const canvas = screen.getByTestId('desktop-canvas');
    fireEvent.pointerDown(canvas, { pointerType: 'mouse', button: 0, clientX: 5, clientY: 5, pointerId: 1 });
    fireEvent.keyDown(canvas, { key: 'a' });
    expect(f.cmds.filter((c) => !['ack'].includes((c as { op: string }).op))).toEqual([]);
    expect(f.opened[0]).toMatchObject({ viewOnly: true });
  });

  it('watch-only checkbox asks for view-only', async () => {
    const f = fake(status());
    mount();
    fireEvent.click(await screen.findByLabelText(/Watch only/));
    fireEvent.click(screen.getByTestId('desktop-start'));
    await waitFor(() => expect(f.opened[0]).toMatchObject({ viewOnly: true }));
  });

  it('declined, encoder failure, replaced and lost (auto-reconnect once)', async () => {
    const f = fake(status());
    mount();
    fireEvent.click(await screen.findByTestId('desktop-start'));
    await waitFor(() => expect(f.opened.length).toBe(1));
    f.msg({ kind: 'error', message: 'The share request was cancelled on the PC', code: 'E_DESKTOP_UNAVAILABLE', reason: 'denied', hint: 'Choose Share' });
    let p = await screen.findByTestId('desktop-problem');
    expect(p.textContent).toContain('Sharing was declined on the PC');
    expect(p.textContent).toContain('Choose Share');
    fireEvent.click(screen.getByRole('button', { name: /Try again/ }));
    await waitFor(() => expect(f.opened.length).toBe(2));
    f.msg({ kind: 'error', message: 'The H.264 encoder failed: boom', code: 'E_DESKTOP_UNAVAILABLE', reason: 'encoder_failed' });
    p = await screen.findByTestId('desktop-problem');
    expect(p.textContent).toContain('encoder failed');
    fireEvent.click(screen.getByRole('button', { name: /Try again/ }));
    await waitFor(() => expect(f.opened.length).toBe(3));
    f.msg({ kind: 'event', event: { kind: 'closed', reason: 'replaced' } });
    expect((await screen.findByTestId('desktop-problem')).textContent).toContain('Watching from another device');
    expect(screen.getByRole('button', { name: /Take over/ })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /Take over/ }));
    await waitFor(() => expect(f.opened.length).toBe(4));
    f.msg({ kind: 'closed', message: 'The PC closed the desktop stream' });
    expect((await screen.findByTestId('desktop-problem')).textContent).toContain('Connection lost');
    // One automatic reconnect.
    await waitFor(() => expect(f.opened.length).toBe(5), { timeout: 3000 });
  });

  it('refused at open (disabled since) shows the command', async () => {
    fake(status(), { ok: false, error: { code: 'E_DESKTOP_DISABLED', message: 'Desktop streaming is off on this PC', retryable: false } });
    mount();
    fireEvent.click(await screen.findByTestId('desktop-start'));
    const p = await screen.findByTestId('desktop-problem');
    expect(p.textContent).toContain('unipi-host desktop enable');
  });

  it('no WebCodecs: says so before and instead of connecting', async () => {
    delete (globalThis as { VideoDecoder?: unknown }).VideoDecoder;
    const f = fake(status());
    mount();
    expect((await screen.findByTestId('desktop-no-webcodecs')).textContent).toContain('WebCodecs');
    expect((screen.getByTestId('desktop-start') as HTMLButtonElement).disabled).toBe(true);
    expect(f.opened.length).toBe(0);
  });

  it('web mode: needs the app', async () => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    setPlatformOverride('web');
    fake(status());
    mount();
    // The whole Preview page needs the app in web mode.
    expect(await screen.findByTestId('preview-needs-app')).toBeTruthy();
  });
});
