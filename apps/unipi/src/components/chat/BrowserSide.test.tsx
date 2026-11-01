// @vitest-environment jsdom
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import { BrowserSheet, BrowserSidePane } from './BrowserSide';
import { browserPane, resetBrowserPane } from '../../lib/browserPane';
import { setBrowserTransport, type BrowserTransport, type StreamMsg } from '../../lib/browser';
import type { Result } from '../../lib/ipc';
import type { BrowserAgent, BrowserOpenData, BrowserProfilesData, BrowserStatusData } from '@unipi/protocol';

vi.mock('../../lib/ipc', () => ({ call: vi.fn() }));

class RO {
  observe() {}
  disconnect() {}
}
Object.assign(globalThis, { ResizeObserver: RO, createImageBitmap: vi.fn(async () => ({ close() {} })) });
HTMLCanvasElement.prototype.getContext = (() => ({ fillRect() {}, drawImage() {}, fillStyle: '' })) as never;

const PROFILES: BrowserProfilesData = {
  default: 'default',
  profiles: [
    { name: 'default', created: 1, running: true, engine: 'cloak', stealth: true, cookie_domains: [] },
    { name: 'shop', created: 2, running: false, cookie_domains: ['shop.example'] },
  ],
};

type Fake = BrowserTransport & { msg: (m: StreamMsg) => void; opened: { profile?: string }[]; takeovers: [boolean, string | undefined][] };
function fake(open: Partial<BrowserOpenData> = {}, takeoverResult?: (on: boolean) => Result<BrowserAgent>): Fake {
  const f: Fake = {
    opened: [],
    takeovers: [],
    msg: () => {},
    status: vi.fn(async (): Promise<Result<BrowserStatusData>> => ({ ok: true, value: { enabled: true, running: true, headless: false, idle_minutes: 10, tabs: [], can_show_window: true } })),
    close: vi.fn(async () => ({ ok: true as const, value: null })),
    window: vi.fn(),
    profiles: vi.fn(async () => ({ ok: true as const, value: PROFILES })),
    takeover: vi.fn(async (_h: string, on: boolean, profile?: string) => {
      f.takeovers.push([on, profile]);
      return takeoverResult ? takeoverResult(on) : { ok: true as const, value: { connected: true, driving: false, taken_over: on } };
    }),
    open: vi.fn(async (_h, args, onMsg) => {
      f.opened.push(args);
      f.msg = onMsg;
      return {
        ok: true as const,
        value: {
          streamId: 's1',
          open: { browser: { name: 'CloakBrowser', path: '/c' }, headless: false, target: 'T1', tabs: [], engine: 'cloak', stealth: true, profile: args.profile ?? 'default', agent: { connected: false, driving: false, taken_over: false }, ...open },
        },
      };
    }),
    cmd: vi.fn(async () => {}),
    stop: vi.fn(),
  };
  setBrowserTransport(f);
  return f;
}

beforeEach(() => {
  localStorage.clear();
  resetBrowserPane();
  document.documentElement.style.setProperty('--detail-left', '624px');
});
afterEach(() => {
  cleanup();
  setBrowserTransport(undefined);
  vi.clearAllMocks();
  delete document.documentElement.dataset.browserPane;
});

describe('browser beside the chat (UNI-296)', () => {
  it('pane: header with profile + stealth badge; agent driving → Take over → You have control → Give back', async () => {
    const f = fake();
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" />);
    expect(await screen.findByTestId('browser-side-pane')).toBeTruthy();
    // jsdom's window is 1024 wide: list (to 624) + chat + pane don't fit, so the list folds.
    expect(document.documentElement.dataset.browserPane).toBe('fold');
    await waitFor(() => expect(f.opened).toHaveLength(1));
    expect(f.opened[0]!.profile).toBe('default');
    await waitFor(() => expect(screen.getByTestId('browser-stealth-badge').textContent).toContain('Stealth'));
    expect(screen.getByTestId('browser-profile-chip').textContent).toContain('default');
    expect(screen.queryByTestId('browser-agent-strip')).toBeNull();
    // URL / back / reload controls are there.
    expect(screen.getByLabelText('Address')).toBeTruthy();
    expect(screen.getByLabelText('Page back')).toBeTruthy();
    expect(screen.getByLabelText('Reload')).toBeTruthy();

    // The agent starts driving.
    f.msg({ kind: 'event', event: { kind: 'agent', agent: { connected: true, driving: true, taken_over: false } } });
    const strip = await screen.findByTestId('browser-agent-strip');
    expect(strip.textContent).toContain('Agent is using the browser');
    fireEvent.click(screen.getByTestId('browser-takeover'));
    await waitFor(() => expect(screen.getByTestId('browser-agent-strip').textContent).toContain('You have control'));
    expect(f.takeovers).toEqual([[true, 'default']]);
    fireEvent.click(screen.getByTestId('browser-giveback'));
    await waitFor(() => expect(screen.queryByTestId('browser-agent-strip')).toBeNull());
    expect(f.takeovers).toEqual([[true, 'default'], [false, 'default']]);
  });

  it('pane: not stealthy shows the Chrome fallback reason', async () => {
    fake({ engine: 'chrome', stealth: false, stealth_reason: 'CloakBrowser failed to start: exited before DevTools came up', browser: { name: 'Google Chrome', path: '/usr/bin/google-chrome' } });
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" />);
    await waitFor(() => expect(screen.getByTestId('browser-stealth-badge').textContent).toContain('Not stealthy'));
    expect(screen.getByTestId('browser-stealth-note').textContent).toContain('Not stealthy — Chrome fallback: CloakBrowser failed to start');
  });

  it('pane: the profile picker re-opens the view on the chosen profile (remembered per PC)', async () => {
    const f = fake();
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" />);
    await waitFor(() => expect((screen.getByTestId('browser-profile-chip') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByTestId('browser-profile-chip'));
    const shop = await screen.findByRole('menuitem', { name: /shop/ });
    expect(shop.textContent).toContain('1 site signed in');
    fireEvent.click(shop);
    await waitFor(() => expect(f.opened.map((o) => o.profile)).toEqual(['default', 'shop']));
    expect(browserPane().profileFor('coffee')).toBe('shop');
    await waitFor(() => expect(screen.getByTestId('browser-profile-chip').textContent).toContain('shop'));
  });

  it('pane: an older PC refuses take over → plain words, strip stays', async () => {
    // The open data already says an agent drives.
    const f = fake({ agent: { connected: true, driving: true, taken_over: false } }, () => ({ ok: false, error: { code: 'E_SCHEMA', message: 'x', retryable: false } }));
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" />);
    fireEvent.click(await screen.findByTestId('browser-takeover'));
    expect((await screen.findByTestId('browser-takeover-error')).textContent).toBe('Update UniPi on the PC to take over from the agent.');
    expect(screen.getByTestId('browser-agent-strip').textContent).toContain('Agent is using the browser');
    expect(f.takeovers).toHaveLength(1);
  });

  it('pane: ✕ closes it (remembered)', async () => {
    fake();
    browserPane().setOpen(true);
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" />);
    fireEvent.click(await screen.findByTestId('browser-pane-close'));
    expect(browserPane().open()).toBe(false);
  });

  it('phone sheet: the same view, closes with ✕', async () => {
    fake({ agent: { connected: true, driving: true, taken_over: false } });
    const onClose = vi.fn();
    render(() => <BrowserSheet open hostId="coffee" hostName="coffee" onClose={onClose} />);
    expect(await screen.findByTestId('browser-sheet')).toBeTruthy();
    expect((await screen.findByTestId('browser-agent-strip')).textContent).toContain('Agent is using the browser');
    fireEvent.click(screen.getByTestId('browser-pane-close'));
    expect(onClose).toHaveBeenCalled();
  });
});
