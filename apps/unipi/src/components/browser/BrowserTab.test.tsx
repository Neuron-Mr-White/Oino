// @vitest-environment jsdom
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import { call } from '../../lib/ipc';
import { encodeFrame, setBrowserTransport, type BrowserTransport, type StreamMsg } from '../../lib/browser';
import { resetBrowserPane } from '../../lib/browserPane';
import { composerDraft, resetComposerDrafts } from '../../lib/composer/drafts';
import { pendingAttachmentsFor, resetAttachments } from '../../lib/composer/attachments';
import type { PaneTabProps } from '../../lib/panes/kinds';
import { BrowserTab, browserTabTitle } from './BrowserTab';
import { closePhoneBrowser, phoneBrowserOpen } from './phoneLayer';

vi.mock('../../lib/ipc', () => ({ call: vi.fn() }));

class RO {
  observe() {}
  disconnect() {}
}
Object.assign(globalThis, { ResizeObserver: RO, createImageBitmap: vi.fn(async () => ({ width: 800, height: 600, close() {} })) });
HTMLCanvasElement.prototype.getContext = (() => ({ fillRect() {}, drawImage() {}, fillStyle: '' })) as never;
HTMLCanvasElement.prototype.toDataURL = (() => 'data:image/png;base64,iVBORw0KGgo=') as never;

type Fake = BrowserTransport & { cmds: Record<string, unknown>[]; msg: (m: StreamMsg) => void; frame: (b: Uint8Array) => void; opened: unknown[] };
function fake(): Fake {
  const f: Fake = {
    cmds: [],
    opened: [],
    msg: () => {},
    frame: () => {},
    status: vi.fn(async () => ({ ok: true as const, value: { enabled: true, running: true, headless: true, idle_minutes: 10, tabs: [] } })),
    close: vi.fn(async () => ({ ok: true as const, value: null })),
    window: vi.fn(),
    profiles: vi.fn(async () => ({ ok: true as const, value: { profiles: [{ name: 'default', created: 0, running: true, cookie_domains: [] }], default: 'default' } })),
    takeover: vi.fn(),
    open: vi.fn(async (_h, args, onMsg, onFrame) => {
      f.opened.push(args);
      f.msg = onMsg;
      f.frame = onFrame;
      return {
        ok: true as const,
        value: {
          streamId: 's1',
          open: { browser: { name: 'CloakBrowser', path: '/c' }, headless: false, target: 'T1', tabs: [{ id: 'T1', url: 'about:blank', title: '' }], engine: 'cloak', stealth: true, profile: 'default', agent: { connected: true, driving: false, taken_over: false }, display: 'hidden' },
        },
      };
    }),
    cmd: vi.fn(async (_id, cmd) => {
      f.cmds.push(cmd as Record<string, unknown>);
    }),
    stop: vi.fn(),
  } as Fake;
  setBrowserTransport(f);
  return f;
}

function mount(extra: Partial<PaneTabProps> = {}) {
  const props = {
    tab: { id: 't1', kind: 'browser', params: { hostId: 'h1' } },
    area: 'side' as const,
    hostId: 'h1',
    projectKey: '/work',
    sessionId: 's1',
    close: vi.fn(),
    setTitle: vi.fn(),
    ...extra,
  };
  render(() => <BrowserTab {...props} />);
  return props;
}

async function live(f: Fake) {
  await waitFor(() => expect(f.opened).toHaveLength(1));
  f.msg({ kind: 'event', event: { kind: 'page', url: 'http://127.0.0.1:8000/', title: 'Probe', loading: false, can_back: false, can_forward: false } });
  f.frame(encodeFrame(1, 800, 600, new Uint8Array([0xff, 0xd8, 0xff, 0])));
  await waitFor(() => expect(f.cmds).toContainEqual({ op: 'ack', seq: 1 }));
  await waitFor(() => expect(screen.queryByTestId('browser-starting')).toBeNull());
}

beforeEach(() => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  vi.mocked(call).mockImplementation(async (cmd: string) => (cmd === 'list_hosts' ? { ok: true, value: [{ id: 'h1', name: 'coffee' }] } : { ok: true, value: null }) as never);
});
afterEach(() => {
  cleanup();
  closePhoneBrowser();
  setBrowserTransport(undefined);
  resetBrowserPane();
  resetComposerDrafts();
  resetAttachments();
  vi.clearAllMocks();
});

describe('Browser pane tab (WP-B8)', () => {
  it('one toolbar row, live screencast with acks, titled by the page; renders frames only (no iframe/webview, H11)', async () => {
    const f = fake();
    const props = mount();
    expect(screen.getByTestId('browser-tab-head')).toBeTruthy();
    expect(screen.queryByTestId('browser-pane-head')).toBeNull(); // the pane strip names + closes the tab
    await live(f);
    expect(props.setTitle).toHaveBeenLastCalledWith('Probe');
    expect((screen.getByLabelText('Address') as HTMLInputElement).value).toBe('127.0.0.1:8000');
    expect(screen.getByTestId('browser-stealth-badge').getAttribute('title')).toContain('CloakBrowser');
    expect(screen.queryByTestId('browser-profile-chip')).toBeNull(); // one profile: no picker
    expect(document.querySelector('iframe, webview')).toBeNull();
    // Esc typed into the page does not close the tab.
    fireEvent.keyDown(screen.getByTestId('browser-canvas'), { key: 'Escape' });
    expect(props.close).not.toHaveBeenCalled();
  });

  it('click and type pass through to the PC', async () => {
    const f = fake();
    mount();
    await live(f);
    const canvas = screen.getByTestId('browser-canvas');
    canvas.getBoundingClientRect = () => ({ left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0, toJSON() {} }) as DOMRect;
    canvas.setPointerCapture = () => {};
    const wrap = canvas.parentElement!;
    wrap.getBoundingClientRect = canvas.getBoundingClientRect;
    fireEvent.pointerDown(canvas, { pointerId: 1, pointerType: 'mouse', clientX: 100, clientY: 50, button: 0, detail: 1 });
    fireEvent.pointerUp(canvas, { pointerId: 1, pointerType: 'mouse', clientX: 100, clientY: 50, button: 0, detail: 1 });
    expect(f.cmds).toContainEqual(expect.objectContaining({ op: 'mouse', kind: 'pressed', x: 100, y: 50, button: 'left' }));
    expect(f.cmds).toContainEqual(expect.objectContaining({ op: 'mouse', kind: 'released', x: 100, y: 50 }));
    fireEvent.keyDown(canvas, { key: 'a', code: 'KeyA' });
    expect(f.cmds.some((c) => c.op === 'key' || c.op === 'text')).toBe(true);
  });

  it('follows a tab the agent opens', async () => {
    const f = fake();
    mount();
    await live(f);
    f.msg({ kind: 'event', event: { kind: 'tabs', tabs: [{ id: 'T1', url: 'about:blank', title: '' }], active: 'T1' } });
    f.msg({ kind: 'event', event: { kind: 'tabs', tabs: [{ id: 'T1', url: 'about:blank', title: '' }, { id: 'T9', url: 'http://127.0.0.1:8000/', title: 'Probe' }], active: 'T1' } });
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'tab_activate', id: 'T9' }));
  });

  it('shows "Agent is using the browser · Take over" while the agent drives', async () => {
    const f = fake();
    mount();
    await live(f);
    f.msg({ kind: 'event', event: { kind: 'agent', agent: { connected: true, driving: true, taken_over: false } } });
    expect((await screen.findByTestId('browser-agent-strip')).textContent).toContain('Agent is using the browser');
    expect(screen.getByTestId('browser-takeover')).toBeTruthy();
  });

  it('element picker: hover box, click → chip + crop in this chat’s composer', async () => {
    const f = fake();
    mount();
    await live(f);
    const canvas = screen.getByTestId('browser-canvas');
    const rect = () => ({ left: 0, top: 0, width: 800, height: 600, right: 800, bottom: 600, x: 0, y: 0, toJSON() {} }) as DOMRect;
    canvas.getBoundingClientRect = rect;
    canvas.parentElement!.getBoundingClientRect = rect;
    canvas.setPointerCapture = () => {};
    fireEvent.click(screen.getByTestId('browser-picker'));
    expect(screen.getByTestId('browser-pick-hint').textContent).toContain('Click an element');
    fireEvent.pointerMove(canvas, { pointerId: 1, pointerType: 'mouse', clientX: 30, clientY: 20 });
    expect(f.cmds).toContainEqual({ op: 'inspect', x: 30, y: 20 });
    f.msg({ kind: 'event', event: { kind: 'element', pick: false, element: { x: 8, y: 8, width: 200, height: 37, label: 'h1#hd' } } });
    const box = await screen.findByTestId('browser-pick-box');
    expect(box.textContent).toBe('h1#hd');
    expect(box.style.width).toBe('200px');
    // The click picks instead of clicking the page.
    const before = f.cmds.filter((c) => c.op === 'mouse').length;
    fireEvent.pointerDown(canvas, { pointerId: 1, pointerType: 'mouse', clientX: 30, clientY: 20, button: 0 });
    fireEvent.pointerUp(canvas, { pointerId: 1, pointerType: 'mouse', clientX: 30, clientY: 20, button: 0 });
    expect(f.cmds.filter((c) => c.op === 'mouse').length).toBe(before);
    expect(f.cmds).toContainEqual({ op: 'inspect', x: 30, y: 20, pick: true });
    f.msg({
      kind: 'event',
      event: { kind: 'element', pick: true, element: { x: 8, y: 8, width: 200, height: 37, label: 'h1#hd', detail: { url: 'http://127.0.0.1:8000/', title: 'Probe', tag: 'h1', role: 'heading', name: 'Hello', selector: '#hd', xpath: '/html/body/h1', text: 'Hello', attributes: { id: 'hd' }, html: '<h1 id="hd">Hello</h1>' } } },
    });
    await waitFor(() => expect(composerDraft('h1/s1').chips).toHaveLength(1));
    expect(composerDraft('h1/s1').chips[0]).toMatchObject({ kind: 'web-element', label: '<h1> Hello' });
    expect(composerDraft('h1/s1').chips[0]!.markdown).toContain('Selector: `#hd`');
    expect(pendingAttachmentsFor('h1/s1')).toHaveLength(1);
    expect(screen.queryByTestId('browser-pick-hint')).toBeNull();
  });

  it('Esc cancels the picker; the screenshot button attaches the page', async () => {
    const f = fake();
    mount();
    await live(f);
    fireEvent.click(screen.getByTestId('browser-picker'));
    fireEvent.keyDown(screen.getByTestId('browser-canvas'), { key: 'Escape' });
    expect(screen.queryByTestId('browser-pick-hint')).toBeNull();
    fireEvent.click(screen.getByTestId('browser-capture'));
    await waitFor(() => expect(pendingAttachmentsFor('h1/s1')).toHaveLength(1));
    expect(pendingAttachmentsFor('h1/s1')[0]!.name).toBe('Probe screenshot.jpg');
  });

  it('titles: page title, else the address, else the kind’s default', () => {
    expect(browserTabTitle({ url: 'http://x/', title: 'X' })).toBe('X');
    expect(browserTabTitle({ url: 'http://x/a', title: 'http://x/a' })).toBe('x/a');
    expect(browserTabTitle({ url: 'about:blank', title: 'about:blank' })).toBeUndefined();
  });

  it('phone: Panes → Browser opens the full-height sheet', async () => {
    fake();
    const { openPhoneBrowser } = await import('./phoneLayer');
    openPhoneBrowser('h1');
    expect(phoneBrowserOpen()).toBe(true);
    expect(await screen.findByTestId('browser-sheet')).toBeTruthy();
  });
});
