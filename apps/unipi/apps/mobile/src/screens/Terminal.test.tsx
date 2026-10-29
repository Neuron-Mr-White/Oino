// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, cleanup } from '@solidjs/testing-library';
import { Route, HashRouter } from '@solidjs/router';
import { SettingsProvider, useSettings } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { RecentsProvider } from '../lib/recentsContext';
import { call } from '../lib/ipc';
import { setTermTarget } from '../lib/termTarget';
import { closeLive, isLive, liveStatus, resetLiveSessions } from '../lib/liveSessions';
import type { TermEvent } from '../lib/term';
import { paletteOpen, setPaletteOpen } from '../lib/shell';
import { setPlatformOverride } from '../lib/platform';
import { resetLayout } from '../lib/layout';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));
// Desktop clipboard (navigator.clipboard in jsdom; the plugin path is never taken: tauri:false).
const clip = { writeText: vi.fn(async (_t: string) => {}), readText: vi.fn(async () => 'pasted') };

// Shared ordering log across the xterm and session mocks.
const log: string[] = [];
// Per-open behaviour: 'ok', 'fail-nonretryable', or 'fail-retryable'.
let openScript: ('ok' | 'fail-nonretryable' | 'fail-retryable')[] = [];
// Captured callbacks per TermSession so tests can emit host events.
const sessions: { callbacks: { onEvent: (e: TermEvent) => void } }[] = [];

vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    textarea: {
      setAttribute: (name: string, value: string) => void;
      addEventListener: (type: string, fn: () => void) => void;
      attrs: Record<string, string>;
    } | null;
    unicode = { activeVersion: '' };
    constructor() {
      log.push('xterm-new');
      const attrs: Record<string, string> = {};
      lastTextareaAttrs = attrs;
      this.textarea = {
        attrs,
        setAttribute: (name: string, value: string) => (attrs[name] = value),
        addEventListener: () => {},
      };
    }
    loadAddon() {}
    open() {}
    reset() {
      log.push('xterm-reset');
    }
    focus() {}
    clearSelection() {}
    refresh() {}
    dispose() {}
    onData(fn: (data: string) => void) {
      xtermData.push(fn);
    }
    onBinary() {}
    attachCustomKeyEventHandler(fn: (e: KeyboardEvent) => boolean) {
      xtermKeys.push(fn);
    }
    options: Record<string, unknown> = { fontSize: 14 };
    getSelection() {
      return 'selected text';
    }
  },
}));
const xtermData: Array<(data: string) => void> = [];
const inputs: string[] = [];
const xtermKeys: Array<(e: KeyboardEvent) => boolean> = [];
let lastTextareaAttrs: Record<string, string> = {};
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit() {} } }));
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    onContextLoss() {}
  },
}));
vi.mock('@xterm/addon-unicode11', () => ({ Unicode11Addon: class {} }));
vi.mock('@xterm/addon-web-links', () => ({ WebLinksAddon: class {} }));

vi.mock('../lib/termSession', () => ({
  TermSession: class {
    callbacks: { onEvent: (e: TermEvent) => void };
    constructor(
      _args: unknown,
      callbacks: { onEvent: (e: TermEvent) => void },
    ) {
      this.callbacks = callbacks;
      sessions.push(this);
    }
    open() {
      log.push('session-open');
      const behaviour = openScript.shift() ?? 'ok';
      if (behaviour === 'ok') {
        return Promise.resolve({
          ok: true as const,
          value: {
            termId: 'tid',
            effective_size_policy: 'fit_phone' as const,
            path: 'lan' as const,
          },
        });
      }
      return Promise.resolve({
        ok: false as const,
        error:
          behaviour === 'fail-nonretryable'
            ? { code: 'E_UNAUTHORIZED' as const, message: 'protected', retryable: false }
            : { code: 'E_TRANSPORT' as const, message: 'lost', retryable: true },
      });
    }
    input(data: string) {
      inputs.push(data);
    }
    inputBinary() {}
    resize() {}
    close() {
      log.push('session-close');
      return Promise.resolve();
    }
  },
}));

function renderTerm() {
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <RecentsProvider>
          <HashRouter root={LayerShell}>
            <Route path="/term" component={() => null} />
            <Route path="*" component={() => <nav class="bottom-nav" />} />
          </HashRouter>
        </RecentsProvider>
      </HostsProvider>
    </SettingsProvider>
  ));
}

// Direct import is fine: vi.mock hoists above it.
import TerminalScreenLazy from './Terminal';

/** Like the app shell: routes plus the always-mounted terminal layer. */
function LayerShell(props: { children?: import('solid-js').JSX.Element }) {
  return (
    <>
      {props.children}
      <TerminalScreenLazy />
    </>
  );
}

async function flush(times = 8) {
  for (let i = 0; i < times; i += 1) await vi.advanceTimersByTimeAsync(0);
}

/** Emits a host-side event on the most recent session. */
function emit(event: TermEvent) {
  sessions.at(-1)?.callbacks.onEvent(event);
}

beforeEach(() => {
  vi.useFakeTimers();
  cleanup();
  log.length = 0;
  openScript = [];
  sessions.length = 0;
  xtermData.length = 0;
  xtermKeys.length = 0;
  inputs.length = 0;
  localStorage.clear();
  resetLiveSessions();
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts')
      return { ok: true, value: [{ id: 'abc', name: 'PC', ticket: 't', paired_at: 0, last_path: 'lan' }] };
    return { ok: true, value: null };
  });
  setTermTarget({
    hostId: 'abc',
    target: { mux: 'herdr', session: 'unipi-m2' },
    label: 'unipi',
    sublabel: 'unipi-m2 · herdr',
  });
  location.hash = '#/term';
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe('Terminal background sessions (UNI-128 follow-up)', () => {
  const herdrTarget = {
    hostId: 'abc',
    target: { mux: 'herdr' as const, session: 'unipi-m2' },
    label: 'unipi',
    sublabel: 'unipi-m2 · herdr',
  };

  it('leaving /term keeps the connection and xterm; coming back reuses them', async () => {
    renderTerm();
    await flush();
    expect(log).toEqual(['xterm-new', 'session-open']);
    expect(liveStatus('abc::' + JSON.stringify(herdrTarget.target))).toBe('live');
    location.hash = '#/';
    await flush();
    // Hidden, not torn down.
    expect(document.querySelector('.term-layer')).toHaveAttribute('hidden');
    expect(log).not.toContain('session-close');
    setTermTarget(herdrTarget);
    location.hash = '#/term';
    await flush();
    expect(document.querySelector('.term-layer')).not.toHaveAttribute('hidden');
    expect(log).toEqual(['xterm-new', 'session-open']);
  });

  it('the "–" exit button only backgrounds the session', async () => {
    renderTerm();
    await flush();
    screen.getByRole('button', { name: 'Exit to Home' }).click();
    await flush();
    await vi.advanceTimersByTimeAsync(50);
    await flush();
    expect(log).not.toContain('session-close');
    expect(isLive('abc::' + JSON.stringify(herdrTarget.target))).toBe(true);
  });

  it('Close connection (closeLive) closes our stream only', async () => {
    renderTerm();
    await flush();
    location.hash = '#/';
    await flush();
    closeLive('abc::' + JSON.stringify(herdrTarget.target));
    await flush();
    expect(log).toEqual(['xterm-new', 'session-open', 'session-close']);
    expect(document.querySelector('.term-layer')).toBeNull();
  });

  it('switching targets keeps the first one connected in the background', async () => {
    renderTerm();
    await flush();
    setTermTarget({ hostId: 'abc', target: { mux: 'tmux', session: 'work' }, label: 'work', sublabel: 'tmux' });
    await flush();
    expect(log.filter((entry) => entry === 'session-open')).toHaveLength(2);
    expect(log).not.toContain('session-close');
    const layers = [...document.querySelectorAll('.term-layer')];
    expect(layers).toHaveLength(2);
    expect(layers.filter((layer) => !layer.hasAttribute('hidden'))).toHaveLength(1);
    expect(screen.getByText('PC: work')).toBeInTheDocument();
  });
});

describe('Terminal screen', () => {
  it('herdr: slim top chrome with exit, session picker, `host: session` title and the route badge', async () => {
    renderTerm();
    await flush();
    expect(screen.getByRole('button', { name: 'Exit to Home' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Switch session' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Collapse header' })).toBeInTheDocument();
    expect(screen.getByText('PC: unipi')).toBeInTheDocument();
    expect(screen.getByLabelText('Route: LAN')).toBeInTheDocument();
  });

  it('the knob collapses the chrome to a strip and remembers it', async () => {
    renderTerm();
    await flush();
    screen.getByRole('button', { name: 'Collapse header' }).click();
    await flush();
    expect(screen.queryByRole('button', { name: 'Exit to Home' })).not.toBeInTheDocument();
    expect(localStorage.getItem('unipi:term-chrome-collapsed')).toBe('true');
    screen.getByRole('button', { name: 'Expand header' }).click();
    await flush();
    expect(screen.getByRole('button', { name: 'Exit to Home' })).toBeInTheDocument();
  });

  it('tmux: the same chrome shows the session title', async () => {
    setTermTarget({
      hostId: 'abc',
      target: { mux: 'tmux', session: 'work' },
      label: 'work',
      sublabel: 'tmux',
    });
    renderTerm();
    await flush();
    expect(screen.getByText('PC: work')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Exit to Home' })).toBeInTheDocument();
  });

  it('creates xterm once; reconnect closes the old session, resets before the second open', async () => {
    renderTerm();
    await flush();
    expect(log.slice(0, 2)).toEqual(['xterm-new', 'session-open']);
    expect(log.filter((entry) => entry === 'xterm-new')).toHaveLength(1);

    // Stream drops (retryable closed) → reconnect → retry fails once
    // → second retry succeeds.
    emit({ type: 'closed' });
    await flush();
    expect(screen.getByRole('status')).toBeInTheDocument();
    await vi.advanceTimersByTimeAsync(500);
    await flush();

    expect(log).toEqual([
      'xterm-new',
      'session-open',
      'session-close',
      'xterm-reset',
      'session-open',
    ]);
    // Banner cleared once live again.
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('shows "Session ended" after an exit event', async () => {
    renderTerm();
    await flush();
    emit({ type: 'exit', code: 0 });
    expect(await screen.findByText('Session ended')).toBeInTheDocument();
  });

  it('a successful open records exactly one recent; a reconnect does not duplicate it', async () => {
    renderTerm();
    await flush();
    await vi.advanceTimersByTimeAsync(200); // recents persist debounce
    const stored = JSON.parse(localStorage.getItem('unipi:recents') ?? '{}');
    expect(stored.recents).toHaveLength(1);
    expect(stored.recents[0]).toMatchObject({
      hostId: 'abc',
      target: { mux: 'herdr', session: 'unipi-m2' },
      label: 'unipi',
      sublabel: 'unipi-m2 · herdr',
    });

    // Reconnect: same target again — no duplicate entry.
    emit({ type: 'closed' });
    await vi.advanceTimersByTimeAsync(500);
    await flush();
    await vi.advanceTimersByTimeAsync(200);
    const after = JSON.parse(localStorage.getItem('unipi:recents') ?? '{}');
    expect(after.recents).toHaveLength(1);
    expect(after.recents[0].label).toBe('unipi');
  });

  it('a failed open adds no recent; a later successful retry does', async () => {
    openScript = ['fail-nonretryable'];
    renderTerm();
    await flush();
    await vi.advanceTimersByTimeAsync(200);
    const stored = JSON.parse(localStorage.getItem('unipi:recents') ?? '{}');
    expect(stored.recents ?? []).toHaveLength(0);

    // Retry the target: now the open succeeds and the recent lands.
    cleanup();
    openScript = [];
    renderTerm();
    await flush();
    await vi.advanceTimersByTimeAsync(200);
    const after = JSON.parse(localStorage.getItem('unipi:recents') ?? '{}');
    expect(after.recents).toHaveLength(1);
  });

  it('shows the ErrorBanner when open fails non-retryably', async () => {
    openScript = ['fail-nonretryable'];
    renderTerm();
    expect(await screen.findByText('protected')).toBeInTheDocument();
  });

  it('a settings change after mount does not open a second session', async () => {
    let probeUpdate: ((path: string, value: unknown) => void) | undefined;
    function Probe() {
      const [, update] = useSettings();
      probeUpdate = update;
      return null;
    }
    render(() => (
      <SettingsProvider>
        <HostsProvider>
          <RecentsProvider>
            <HashRouter>
              <Route path="/term" component={TerminalScreenLazy} />
              <Probe />
            </HashRouter>
          </RecentsProvider>
        </HostsProvider>
      </SettingsProvider>
    ));
    await flush();
    const opensBefore = log.filter((entry) => entry === 'session-open').length;
    probeUpdate!('terminal.fontSize', 20);
    await flush();
    const opensAfter = log.filter((entry) => entry === 'session-open').length;
    expect(opensAfter).toBe(opensBefore);
  });
});


describe('desktop terminal (UNI-117)', () => {
  const key = (k: string, m: Partial<KeyboardEventInit> = {}) => new KeyboardEvent('keydown', { key: k, cancelable: true, ...m });
  beforeEach(() => {
    // Desktop mode, but no native core in jsdom (stores/clipboard go their browser paths).
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: false });
    Object.defineProperty(navigator, 'clipboard', { value: clip, configurable: true });
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: false, media: q, addEventListener() {}, removeEventListener() {} }));
    resetLayout();
  });
  afterEach(() => {
    setPlatformOverride(undefined);
    vi.unstubAllGlobals();
    vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    Object.defineProperty(window, 'innerWidth', { value: 1024, configurable: true });
    resetLayout();
  });

  it('no phone toolbar; xterm takes the keyboard (onData → PTY) and the live sessions show as tabs', async () => {
    renderTerm();
    await flush();
    expect(document.querySelector('.term-toolbar, [data-testid=term-toolbar]')).toBeNull();
    expect(screen.queryByRole('button', { name: /keyboard/i })).toBeNull();
    xtermData[0]!('ls\r');
    expect(inputs).toContain('ls\r');
    expect(screen.getByRole('tablist', { name: 'Open terminals' })).toBeTruthy();
    expect(screen.getAllByRole('tab')).toHaveLength(1);
  });

  it('Ctrl+Shift+C copies the selection, Ctrl+= grows the font, plain Ctrl+C goes to the shell', async () => {
    renderTerm();
    await flush();
    const handler = xtermKeys[0]!;
    expect(handler(key('C', { ctrlKey: true, shiftKey: true }))).toBe(false);
    await flush();
    expect(clip.writeText).toHaveBeenCalledWith('selected text');
    expect(handler(key('=', { ctrlKey: true }))).toBe(false);
    expect(handler(key('c', { ctrlKey: true }))).toBe(true);
    expect(handler(key('V', { ctrlKey: true, shiftKey: true }))).toBe(false);
    await flush();
    expect(inputs.some((i) => i.includes('pasted'))).toBe(true);
  });

  it('Ctrl+Shift+K is left to the app shell (the terminal must not open the palette itself, or the shell toggle closes it)', async () => {
    renderTerm();
    await flush();
    const handler = xtermKeys[0]!;
    setPaletteOpen(false);
    const event = key('K', { ctrlKey: true, shiftKey: true });
    expect(handler(event)).toBe(false); // not sent to the PTY
    expect(event.defaultPrevented).toBe(false);
    expect(paletteOpen()).toBe(false); // DesktopChrome's window handler opens it (once)
  });

  it('UNI-311: Shift+Enter, Ctrl+Enter and Ctrl+Shift+<letter> reach the PTY as CSI-u, not the app or xterm\'s own (wrong) encoding', async () => {
    renderTerm();
    await flush();
    const handler = xtermKeys[0]!;
    expect(handler(key('Enter', { shiftKey: true }))).toBe(false);
    expect(inputs).toContain('\x1b[13;2u');
    expect(handler(key('Enter', { ctrlKey: true }))).toBe(false);
    expect(inputs).toContain('\x1b[13;5u');
    expect(handler(key('p', { ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(inputs).toContain('\x1b[112;6u');
    // Plain Enter stays xterm's own job (handler returns true = not handled here).
    expect(handler(key('Enter'))).toBe(true);
  });

  it('UNI-311: right-click inside the terminal is not shown as the webview\'s own context menu (herdr still gets the mouse event)', async () => {
    renderTerm();
    await flush();
    const area = document.querySelector('.term-touch')!;
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    area.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it('UNI-313: the xterm helper textarea has autocomplete/autocorrect/autocapitalize/spellcheck off', async () => {
    renderTerm();
    await flush();
    expect(lastTextareaAttrs).toMatchObject({
      autocomplete: 'off',
      autocorrect: 'off',
      autocapitalize: 'off',
      spellcheck: 'false',
    });
  });
});
