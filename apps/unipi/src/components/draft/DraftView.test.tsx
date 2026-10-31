// @vitest-environment jsdom
/**
 * Draft view (Z-19, WP-A5): `/new/:host` shows the pickers + composer; the
 * first prompt creates the session (`session_new`) and routes to
 * `/s/:host/:id`; the prompt itself rides `firstPrompt` until that session is
 * live — taken exactly once (the WP-A5 acceptance: draft → first prompt
 * creates and routes).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { queueFirstPrompt, takeFirstPrompt } from '../../lib/session/firstPrompt';
import type { SessionCard } from '../../lib/host/client';
import { DraftView } from './DraftView';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, cwd: string): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd,
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
});

let world: ReturnType<typeof createMockHost>;
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
        fs: { '/home/you/p/oino': { kind: 'dir', children: {} }, '/home/you/p/website': { kind: 'dir', children: {} } },
      },
    ],
  };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/new/this-pc';
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="/new/:host" component={() => <DraftView hostId="this-pc" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('DraftView (Z-19)', () => {
  it('offers the host roots and recent cwds as folder chips', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('draft-view')).toBeTruthy());
    await waitFor(() => expect(screen.getByText(/oino/)).toBeTruthy());
    expect(screen.getByTestId('draft-input')).toBeTruthy();
  });

  it('the first prompt creates the session and routes to it', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('draft-input')).toBeTruthy());
    fireEvent.input(screen.getByTestId('draft-input'), { target: { value: 'Triage the failing tests' } });
    fireEvent.submit(screen.getByTestId('draft-form'));
    await waitFor(() => expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_new')).toBe(true));
    await waitFor(() => expect(location.hash).toMatch(/^#\/s\/this-pc\/s-new-/));
  });

  it('an empty draft cannot send', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('draft-send')).toBeTruthy());
    expect((screen.getByTestId('draft-send') as HTMLButtonElement).disabled).toBe(true);
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
