// @vitest-environment jsdom
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import { BrowserSheet, BrowserSidePane } from './BrowserSide';
import { browserPane, resetBrowserPane } from '../../lib/browserPane';
import { setBrowserTransport, type BrowserTransport } from '../../lib/browser';
import type { LiveTool, SessionEntry } from '../../lib/session/types';
import type { BrowserProfilesData } from '@unipi/protocol';

vi.mock('../../lib/ipc', () => ({ call: vi.fn() }));

// The chat's session state: tests push browser tool results into it.
const [chat, setChat] = createStore<{ entries: SessionEntry[]; liveTools: Record<string, LiveTool> }>({ entries: [], liveTools: {} });
vi.mock('../../lib/session/store', async (importOriginal) => ({ ...(await importOriginal<object>()), sessions: () => ({ peek: () => ({ state: chat }) }) }));

class RO {
  observe() {}
  disconnect() {}
}
Object.assign(globalThis, { ResizeObserver: RO, createImageBitmap: vi.fn(async () => ({ close() {} })) });
HTMLCanvasElement.prototype.getContext = (() => ({ fillRect() {}, drawImage() {}, fillStyle: '' })) as never;

const PROFILES: BrowserProfilesData = {
  default: 'default',
  profiles: [
    { name: 'default', created: 1, running: true, cookie_domains: [] },
    { name: 'ace-jobs', created: 2, running: true, cookie_domains: ['linkedin.com'] },
    { name: 'shop', created: 3, running: false, cookie_domains: [] },
  ],
};

type Fake = BrowserTransport & { opened: (string | undefined)[]; statusCalls: (string | undefined)[]; agent: { profile?: string } };
/** `agent.profile` = what the PC answers as `session_profile` (undefined = an older PC). */
function fake(agentProfile?: string): Fake {
  const f: Fake = {
    opened: [],
    statusCalls: [],
    agent: { profile: agentProfile },
    status: vi.fn(async (_h: string, sessionId?: string) => {
      f.statusCalls.push(sessionId);
      return { ok: true as const, value: { enabled: true, running: true, headless: false, idle_minutes: 10, tabs: [], can_show_window: true, ...(sessionId && f.agent.profile ? { session_profile: f.agent.profile } : {}) } };
    }),
    close: vi.fn(async () => ({ ok: true as const, value: null })),
    window: vi.fn(),
    profiles: vi.fn(async () => ({ ok: true as const, value: PROFILES })),
    takeover: vi.fn(),
    open: vi.fn(async (_h, args) => {
      f.opened.push(args.profile);
      return {
        ok: true as const,
        value: { streamId: 's1', open: { browser: { name: 'CloakBrowser', path: '/c' }, headless: false, target: 'T1', tabs: [], engine: 'cloak', stealth: true, profile: args.profile ?? 'default', agent: { connected: false, driving: false, taken_over: false } } },
      };
    }),
    cmd: vi.fn(async () => {}),
    stop: vi.fn(),
  };
  setBrowserTransport(f);
  return f;
}

const result = (id: string, profile: string): SessionEntry => ({
  type: 'message',
  id: `e-${id}`,
  message: { role: 'toolResult', toolCallId: id, toolName: 'browser_navigate', details: { browser: { profile, engine: 'cloak', generation: 1 } } },
});

beforeEach(() => {
  localStorage.clear();
  resetBrowserPane();
  setChat({ entries: [], liveTools: {} });
  document.documentElement.style.setProperty('--detail-left', '624px');
});
afterEach(() => {
  cleanup();
  setBrowserTransport(undefined);
  vi.clearAllMocks();
  delete document.documentElement.dataset.browserPane;
});

describe('browser beside the chat follows the chat’s agent profile', () => {
  it('opens the profile the PC names for the chat — never `default` first', async () => {
    const f = fake('ace-jobs');
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" sessionId="s1" />);
    await waitFor(() => expect(f.opened.length).toBeGreaterThan(0));
    expect(f.statusCalls).toContain('s1');
    expect(f.opened).toEqual(['ace-jobs']);
    await waitFor(() => expect(screen.getByTestId('browser-profile-chip').textContent).toContain('ace-jobs'));
  });

  it('opens on the chat’s last browser tool result when the PC is slow, then the PC settles it', async () => {
    setChat('entries', [result('c1', 'ace-jobs')]);
    const f = fake('ace-jobs');
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" sessionId="s1" />);
    await waitFor(() => expect(f.opened.length).toBeGreaterThan(0));
    expect(f.opened.every((p) => p === 'ace-jobs')).toBe(true);
  });

  it('switches when a later browser tool result names another profile (and re-reads the PC)', async () => {
    const f = fake('ace-jobs');
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" sessionId="s1" />);
    await waitFor(() => expect(f.opened).toEqual(['ace-jobs']));
    f.agent.profile = 'shop';
    setChat('entries', (e) => [...e, result('c2', 'shop')]);
    await waitFor(() => expect(f.opened).toEqual(['ace-jobs', 'shop']));
    expect(f.statusCalls.length).toBeGreaterThanOrEqual(2);
    // A live (still running) tool that finished counts the same.
    f.agent.profile = 'ace-jobs';
    setChat('liveTools', 'l1', { id: 'l1', name: 'browser_click', args: {}, startedAt: Date.now(), done: true, result: { details: { browser: { profile: 'ace-jobs' } } } });
    await waitFor(() => expect(f.opened).toEqual(['ace-jobs', 'shop', 'ace-jobs']));
  });

  it('the profile menu tags the agent’s profile; picking another is viewing only (per chat)', async () => {
    const f = fake('ace-jobs');
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" sessionId="s1" />);
    await waitFor(() => expect((screen.getByTestId('browser-profile-chip') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByTestId('browser-profile-chip'));
    expect(screen.getByTestId('browser-profile-note').textContent).toContain('the agent keeps using ace-jobs');
    const agentRow = await screen.findByRole('menuitem', { name: /ace-jobs/ });
    expect(agentRow.textContent).toContain('Agent');
    fireEvent.click(await screen.findByRole('menuitem', { name: /shop/ }));
    await waitFor(() => expect(f.opened).toEqual(['ace-jobs', 'shop']));
    // Remembered for this chat only — the PC's own choice is untouched and the agent is not told.
    expect(browserPane().chatProfileFor('coffee', 's1')).toEqual({ profile: 'shop', seen: 'ace-jobs' });
    expect(browserPane().profileFor('coffee')).toBe('default');
    expect(f.takeover).not.toHaveBeenCalled();
    // The agent switching afterwards wins over the old pick.
    f.agent.profile = 'default';
    setChat('entries', (e) => [...e, result('c3', 'default')]);
    await waitFor(() => expect(f.opened).toEqual(['ace-jobs', 'shop', 'default']));
  });

  it('an older PC without session_profile keeps today’s per-PC choice', async () => {
    const f = fake(undefined);
    browserPane().setProfileFor('coffee', 'shop');
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" sessionId="s1" />);
    await waitFor(() => expect(f.opened).toEqual(['shop']));
    fireEvent.click(await screen.findByTestId('browser-profile-chip'));
    expect(screen.queryByTestId('browser-profile-note')).toBeNull();
    fireEvent.click(await screen.findByRole('menuitem', { name: /ace-jobs/ }));
    await waitFor(() => expect(f.opened).toEqual(['shop', 'ace-jobs']));
    expect(browserPane().profileFor('coffee')).toBe('ace-jobs');
  });

  it('a pane without a chat keeps the per-PC choice and never asks the PC about a chat', async () => {
    const f = fake('ace-jobs');
    browserPane().setProfileFor('coffee', 'shop');
    render(() => <BrowserSidePane show hostId="coffee" hostName="coffee" />);
    await waitFor(() => expect(f.opened).toEqual(['shop']));
    expect(f.statusCalls.every((s) => s === undefined)).toBe(true);
  });

  it('phone sheet follows the chat too', async () => {
    const f = fake('ace-jobs');
    render(() => <BrowserSheet open hostId="coffee" hostName="coffee" sessionId="s1" onClose={() => {}} />);
    await waitFor(() => expect(f.opened).toEqual(['ace-jobs']));
  });

  it('saved state from before (per-PC only) still loads', async () => {
    localStorage.setItem('unipi.browserPane', JSON.stringify({ open: true, width: 480, profiles: { coffee: 'shop' } }));
    resetBrowserPane();
    expect(browserPane().profileFor('coffee')).toBe('shop');
    expect(browserPane().chatProfileFor('coffee', 's1')).toBeUndefined();
  });
});
