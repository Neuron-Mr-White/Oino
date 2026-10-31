// @vitest-environment jsdom
/**
 * Draft view (Z-19, WP-A5): `/new/:host` renders the chat's own composer
 * (same toolbar) with a workspace row inside the card; the first prompt
 * starts the session in the chosen folder (`startSession`: `session_new`
 * stream + the prompt) and routes to `/s/:host/:id`. The draft's own stand-in
 * session keeps the permission / Plan / thinking choices for that start.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { setDesktopOverride } from '../../lib/layout';
import { composerDraft, resetComposerDrafts } from '../../lib/composer/drafts';
import { queueFirstPrompt, takeFirstPrompt } from '../../lib/session/firstPrompt';
import type { SessionCard } from '../../lib/host/client';
import { createDraftSession, draftKey } from './draftSession';
import { homeOf, parentOf } from './WorkspacePicker';

const start = vi.hoisted(() => vi.fn());
vi.mock('./startSession', () => ({ startSession: start, LINGER_MS: 0 }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

import { DraftView } from './DraftView';

const card = (id: string, cwd: string): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd,
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
});

const boot = () => {
  const scenario: MockScenario = {
    name: 't',
    hosts: [
      {
        id: 'this-pc',
        name: 'workstation',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [card('a', '/home/you/p/oino')],
        fs: { '/home/you': { kind: 'dir', children: { p: { kind: 'dir', children: {} } } }, '/srv': { kind: 'dir', children: {} } },
        settings: {
          pi: { defaultProvider: 'oino', defaultModel: 'deepseek-flash' },
          extensions: [{ id: 'unipi.workflow', kind: 'extension', enabled: true, loaded: true }],
        },
      },
    ],
  };
  const world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/new/this-pc';
  render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="/new/:host" component={() => <DraftView hostId="this-pc" />} />
          <Route path="/s/:host/:id" component={() => <div data-testid="chat-route" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
  return world;
};

const field = () => document.querySelector('.chip-field') as HTMLDivElement;
const typeInto = (text: string) => {
  const el = field();
  el.focus();
  el.textContent = text;
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(false);
  window.getSelection()!.removeAllRanges();
  window.getSelection()!.addRange(range);
  fireEvent.input(el);
};

beforeEach(() => {
  localStorage.clear();
  resetComposerDrafts();
  setDesktopOverride(true);
  start.mockReset();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('DraftView (Z-19)', () => {
  it('renders the chat composer with its toolbar and a workspace row', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('draft-view')).toBeTruthy());
    expect(screen.getByTestId('tl-composer')).toBeTruthy();
    expect(screen.getByTestId('composer-toolbar')).toBeTruthy();
    expect(screen.getByTestId('composer-plus')).toBeTruthy();
    expect(screen.getByTestId('send-send')).toBeTruthy();
    expect(screen.getByTestId('workspace-chip').textContent).toContain('No workspace');
    // The host's default model; the draft has no context to measure yet.
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    expect(screen.queryByTestId('context-ring')).toBeNull();
    // The workflow extension runs on this PC → its controls are there.
    await waitFor(() => expect(screen.getByTestId('permission-chip')).toBeTruthy());
    expect(screen.getByTestId('plan-toggle')).toBeTruthy();
    // No old pill rows and no bare textarea.
    expect(document.querySelector('textarea')).toBeNull();
    expect(screen.queryByTestId('draft-folders')).toBeNull();
  });

  it('the workspace menu lists recent folders and the roots; a pick shows in the chip', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('workspace-chip')).toBeTruthy());
    await new Promise((r) => setTimeout(r, 30));
    fireEvent.click(screen.getByTestId('workspace-chip'));
    const menu = await waitFor(() => screen.getByTestId('workspace-menu'));
    await waitFor(() => expect(menu.textContent).toContain('~/p/oino'));
    expect(menu.textContent).toContain('/srv');
    expect(menu.textContent).toContain('Open folder…');
    fireEvent.click([...menu.querySelectorAll('.ws-row')].find((r) => r.textContent?.includes('~/p/oino'))!);
    await waitFor(() => expect(screen.getByTestId('workspace-chip').textContent).toContain('oino'));
  });

  it('the first prompt starts the session in the workspace and routes to it', async () => {
    start.mockResolvedValue({ ok: true, sessionId: 's-new-1' });
    boot();
    await waitFor(() => expect(field()).toBeTruthy());
    typeInto('Triage the failing tests');
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    const [host, req] = start.mock.calls[0]!;
    expect(host).toBe('this-pc');
    expect(req.text).toBe('Triage the failing tests');
    // No workspace → the user's home folder on that PC.
    expect(req.cwd).toBe('/home/you');
    await waitFor(() => expect(location.hash).toBe('#/s/this-pc/s-new-1'));
  });

  it('a failed start keeps the message and says why', async () => {
    start.mockResolvedValue({ ok: false, error: { code: 'E_FAILED', message: 'The host is busy', retryable: true } });
    boot();
    await waitFor(() => expect(field()).toBeTruthy());
    typeInto('Hello there');
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(screen.getByTestId('draft-error').textContent).toContain('The host is busy'));
    expect(composerDraft(draftKey('this-pc')).text).toBe('Hello there');
  });

  it('an empty draft cannot send', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('send-send')).toBeTruthy());
    expect((screen.getByTestId('send-send') as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('draft session (stand-in handle)', () => {
  it('keeps permission / plan / thinking choices instead of sending them', async () => {
    const world = createMockHost({ name: 't', hosts: [{ id: 'h', name: 'h', conn: { state: 'connected' } }] });
    setHostClientFactory(world.client);
    const submitted: string[] = [];
    const s = createRoot(() => createDraftSession('h', (text) => submitted.push(text)));
    s.send('/permission full', { mode: 'prompt' });
    s.send('/plan on', { mode: 'prompt' });
    await s.cmd({ type: 'set_thinking_level', level: 'high' });
    expect(s.choices()).toEqual({ permission: 'full', plan: true, thinking: 'high' });
    expect((s.state.topics['unipi:workflow:status']?.data as { planMode: boolean }).planMode).toBe(true);
    expect(s.state.thinkingLevel).toBe('high');
    expect(submitted).toEqual([]);
    s.send('Go', { mode: 'prompt' });
    expect(submitted).toEqual(['Go']);
  });
});

describe('workspace helpers', () => {
  it('finds the home among the roots and walks up', () => {
    expect(homeOf(['/home/oi', '/tmp'])).toBe('/home/oi');
    expect(homeOf(['/srv'])).toBeUndefined();
    expect(parentOf('/home/oi/p')).toBe('/home/oi');
    expect(parentOf('/home')).toBe('/');
  });
});

describe('firstPrompt queue', () => {
  it('is taken exactly once', () => {
    queueFirstPrompt('h', 's1', { text: 'hi' });
    expect(takeFirstPrompt('h', 's1')).toEqual({ text: 'hi' });
    expect(takeFirstPrompt('h', 's1')).toBeUndefined();
    expect(takeFirstPrompt('h', 'other')).toBeUndefined();
  });
});
