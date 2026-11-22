// @vitest-environment jsdom
/**
 * Session row context menu (Z-16, WP-A5): right-click opens the menu; every
 * row does its §3.14 op (asserted on the mock host's log — the WP-A5
 * acceptance); Delete is a confirm step (Z-20); actions still run after the
 * dismiss unmounts the row (the UNI-226 class of bug, guarded on this menu);
 * Rename… opens the dialog and locks the title (Z-15).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { setDesktopOverride } from '../../lib/layout';
import { resetHubs } from '../../lib/session/hub';
import { closeRowMenu, openRowMenu, openRowMenuState } from '../../lib/session/actions';
import { closeRenameDialog } from '../session/RenameDialog';
import type { SessionCard } from '../../lib/host/client';
import { SessionActions } from '../session/SessionActions';
import { SessionList } from './SessionList';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));
vi.mock('../../lib/board/share', () => ({ copyText: vi.fn(async () => true), openExternal: () => false }));
import { copyText } from '../../lib/board/share';

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  file: `/home/you/.oino/s/${id}.jsonl`,
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
  ...over,
});

let world: ReturnType<typeof createMockHost>;
const boot = (cards: SessionCard[], archived: SessionCard[] = []) => {
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
          <Route path="*" component={() => (<><SessionList variant="sidebar" /><SessionActions /></>)} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const ops = () => world.log.filter((e) => e.kind === 'request').map((e) => `${e.op}`);
const opWith = (op: string) => world.log.find((e) => e.kind === 'request' && e.op === op);

const openMenuOn = async (id: string) => {
  const row = screen.getByTestId(`srow-${id}`);
  fireEvent.contextMenu(row);
  await waitFor(() => expect(screen.getByRole('menu', { name: /Session/ })).toBeTruthy());
};

beforeEach(() => {
  localStorage.clear();
  setDesktopOverride(false); // the phone shell: Reveal folder routes to /files
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  closeRowMenu();
  closeRenameDialog();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('SessionMenu (Z-16)', () => {
  it('right-click opens the row menu; Pin sends session_meta_set{pinned}', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Pin' }));
    await waitFor(() => expect(opWith('session_meta_set')?.args).toMatchObject({ session: 'a', pinned: true }));
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('Mark as unread sends session_meta_set{unread:true}', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mark as unread' }));
    await waitFor(() => expect(opWith('session_meta_set')?.args).toMatchObject({ session: 'a', unread: true }));
  });

  it('Archive sends session_archive (Z-13)', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: /Archive/ }));
    await waitFor(() => expect(opWith('session_archive')?.args).toMatchObject({ session: 'a' }));
    // The mock emitted session_removed: the row is gone from the live list.
    await waitFor(() => expect(screen.queryByTestId('srow-a')).toBeNull());
  });

  it('Fork opens a fork stream with a distinguishable "(fork)" title and routes to it (Z-17, D-10)', async () => {
    boot([card('a', { name: 'Named by slash' })]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Fork' }));
    await waitFor(() =>
      expect(
        world.log.some(
          (e) => e.kind === 'open' && e.op === 'session_fork' && (e.args as { name?: string }).name === 'Named by slash (fork)',
        ),
      ).toBe(true),
    );
  });

  it('Copy id / path / log copy the card fields (desktop shell)', async () => {
    setDesktopOverride(true);
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy session id' }));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith('a'));
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy log path' }));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith('/home/you/.oino/s/a.jsonl'));
  });

  it('the phone shell has no copy / reveal / export rows (phone spec §3.3, PH-19)', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    for (const name of ['Copy session id', 'Copy path', 'Copy log path', 'Reveal folder', 'Export…']) expect(screen.queryByRole('menuitem', { name })).toBeNull();
    expect(screen.getByRole('menuitem', { name: 'Fork' })).toBeTruthy();
  });

  it('Export opens the session and asks for the JSONL (desktop shell)', async () => {
    setDesktopOverride(true);
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export…' }));
    await waitFor(() => expect(world.log.some((e) => e.kind === 'open' && e.op === 'session_open')).toBe(true));
  });

  it('Delete asks first with full unclipped prompt + Cancel/Yes, delete, then archives + deletes (Z-20, D-04, D-16)', async () => {
    const longTitle = 'Hello Number 4 Reply With A Longer Title';
    boot([card('a', { name: longTitle })]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    expect(screen.queryByText('Archived list')).toBeNull();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete' }));
    // One click only armed the confirm row; full title + Yes, delete + Cancel are all visible.
    expect(ops()).not.toContain('session_archive');
    expect(screen.getByTestId('srow-delete-confirm').textContent).toContain(`Delete “${longTitle}”?`);
    expect(screen.getByRole('menuitem', { name: 'Cancel' })).toBeTruthy();
    fireEvent.click(screen.getByRole('menuitem', { name: /Yes, delete/ }));
    await waitFor(() => expect(opWith('session_archive')?.args).toMatchObject({ session: 'a' }));
    await waitFor(() => expect(opWith('session_delete')?.args).toMatchObject({ session: 'a' }));
  });

  it('actions run after the dismiss unmounts the menu (UNI-226 class)', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    const row = screen.getByTestId('srow-a');
    fireEvent.contextMenu(row);
    await waitFor(() => expect(openRowMenuState()).toBeTruthy());
    // Dismiss the menu, then fire the still-referenced row handler: the pinned
    // read must not be a stale Show read.
    closeRowMenu();
    expect(() => openRowMenu('this-pc', card('a'), row.getBoundingClientRect())).not.toThrow();
    await waitFor(() => expect(openRowMenuState()?.card.session_id).toBe('a'));
  });

  it('Rename… opens the dialog; saving renames and locks the title (Z-15)', async () => {
    boot([card('a')]);
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());
    await openMenuOn('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Rename…' }));
    const input = await screen.findByTestId('rename-input');
    expect((input as HTMLInputElement).value).toBe('Session a');
    fireEvent.input(input, { target: { value: 'Triage' } });
    fireEvent.submit(screen.getByTestId('rename-form'));
    await waitFor(() => expect(opWith('session_rename')?.args).toMatchObject({ session: 'a', name: 'Triage' }));
    await waitFor(() => expect(opWith('session_meta_set')?.args).toMatchObject({ session: 'a', title_locked: true }));
    // And the card carried the new name to the live list.
    await waitFor(() => expect(screen.getByText('Triage')).toBeTruthy());
    // The sheet's exit fade takes a moment before the form leaves the DOM.
    await waitFor(() => expect(screen.queryByTestId('rename-form')).toBeNull());
  });
});
