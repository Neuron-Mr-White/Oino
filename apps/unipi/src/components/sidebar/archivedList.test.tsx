// @vitest-environment jsdom
/**
 * Archived list (Z-13, WP-A5): opening lazily asks `sessions_list{archived:true}`;
 * Restore round-trips the card back into the live list; Delete asks first;
 * Delete all confirms and sends `trash_empty` (the WP-A5 acceptance).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { parseArchivedJsonl } from '../../lib/session/archivedSession';
import type { SessionCard } from '../../lib/host/client';
import { SessionList } from './SessionList';

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
  ...over,
});

let world: ReturnType<typeof createMockHost>;
const boot = (cards: SessionCard[], archived: SessionCard[]) => {
  const scenario: MockScenario = {
    name: 't',
    hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards, archived }],
  };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/';
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => <SessionList variant="sidebar" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const openArchived = async () => {
  fireEvent.click(screen.getByTestId('sb-archived-head-this-pc'));
  await waitFor(() => expect(screen.getByTestId('sb-archived-body-this-pc')).toBeTruthy());
};

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('ArchivedList (Z-13)', () => {
  it('lazily loads the archived list on open (sessions_list{archived:true})', async () => {
    boot([card('live')], [card('gone')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    // Not asked before opening.
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'sessions_list' && (e.args as { archived?: boolean } | undefined)?.archived === true)).toBe(false);
    await openArchived();
    await waitFor(() => expect(screen.getByTestId('sb-archived-row-gone')).toBeTruthy());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'sessions_list' && (e.args as { archived?: boolean } | undefined)?.archived === true)).toBe(true);
  });

  it('restore round trip: the row leaves the trash and reappears live', async () => {
    boot([card('live')], [card('gone')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    fireEvent.click(screen.getByTestId('sb-archived-row-gone').querySelector('.sb-archived-restore')!);
    await waitFor(() => expect(screen.queryByTestId('sb-archived-row-gone')).toBeNull());
    await waitFor(() => expect(screen.getByTestId('srow-gone')).toBeTruthy());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_restore' && (e.args as { session?: string })?.session === 'gone')).toBe(true);
  });

  it('delete asks once, then unlinks the archived file (Z-20)', async () => {
    boot([card('live')], [card('gone')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    fireEvent.click(screen.getByTestId('sb-archived-row-gone').querySelector('.sb-archived-del')!);
    // Armed, not sent.
    expect(world.log.some((e) => e.op === 'session_delete')).toBe(false);
    fireEvent.click(screen.getByTestId('sb-archived-row-gone').querySelector('.sb-archived-del')!);
    await waitFor(() => expect(screen.queryByTestId('sb-archived-row-gone')).toBeNull());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_delete' && (e.args as { session?: string })?.session === 'gone')).toBe(true);
  });

  it('delete all confirms, then empties the trash (trash_empty)', async () => {
    boot([card('live')], [card('g1'), card('g2')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(screen.getByTestId('sb-archived-row-g2')).toBeTruthy());
    fireEvent.click(screen.getByTestId('sb-archived-empty'));
    expect(world.log.some((e) => e.op === 'trash_empty')).toBe(false);
    fireEvent.click(screen.getByTestId('sb-archived-empty-confirm'));
    await waitFor(() => expect(screen.queryByTestId('sb-archived-row-g1')).toBeNull());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'trash_empty')).toBe(true);
  });

  it('an empty trash says so instead of rows', async () => {
    boot([card('live')], []);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(screen.getByText('Nothing archived')).toBeTruthy());
  });

  it('archived rows render an openable link with full title tooltip, and parseArchivedJsonl extracts read-only transcript entries (D-17)', async () => {
    boot([card('live')], [card('gone', { name: 'Hello Number 2 Reply' })]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    const link = await screen.findByTestId('sb-archived-open-gone');
    expect(link.tagName).toBe('A');
    expect(link.getAttribute('href')).toBe('#/s/this-pc/gone');
    expect(link.getAttribute('title')).toBe('Hello Number 2 Reply');

    const parsed = parseArchivedJsonl(
      [
        JSON.stringify({ type: 'session', version: 3, id: 'gone', cwd: '/home/oi' }),
        JSON.stringify({ type: 'session_info', id: 'i1', parentId: null, name: 'Hello Number 2 Reply' }),
        JSON.stringify({ type: 'message', id: 'u1', parentId: 'i1', message: { role: 'user', content: 'ping' } }),
        JSON.stringify({ type: 'message', id: 'a1', parentId: 'u1', message: { role: 'assistant', content: 'pong' } }),
      ].join('\n'),
    );
    expect(parsed.sessionName).toBe('Hello Number 2 Reply');
    expect(parsed.cwd).toBe('/home/oi');
    expect(parsed.entries).toHaveLength(3);
    expect(parsed.leafId).toBe('a1');
  });
});
