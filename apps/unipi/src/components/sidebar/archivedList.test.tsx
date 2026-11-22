// @vitest-environment jsdom
/**
 * Archived sessions (Z-13, WP-A5; owner 2026-11-05: the fixed "Archived" button opens a
 * searchable modal instead of a sidebar footer list): opening lazily asks
 * `sessions_list{archived:true}`; Restore round-trips the card back into the live list;
 * Delete asks first; Delete all confirms and sends `trash_empty`.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { parseArchivedJsonl } from '../../lib/session/archivedSession';
import { isGeneralWorkspaceCwd } from '../../lib/session/generalWorkspace';
import type { SessionCard } from '../../lib/host/client';
import { SessionList } from './SessionList';
import { ArchivedButton, ArchivedModalLayer, archivedView } from './ArchivedModal';

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
          <Route path="*" component={() => (<><SessionList variant="sidebar" /><ArchivedButton hostId="this-pc" /><ArchivedModalLayer /></>)} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const openArchived = async () => {
  fireEvent.click(screen.getByTestId('sb-archived-open'));
  await waitFor(() => expect(document.querySelector('[data-testid=archived-modal]')).toBeTruthy());
};
const q = (id: string) => document.querySelector<HTMLElement>(`[data-testid="${id}"]`);

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('Archived modal (Z-13)', () => {
  it('lazily loads the archived list on open (sessions_list{archived:true})', async () => {
    boot([card('live')], [card('gone')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    // Not asked before opening.
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'sessions_list' && (e.args as { archived?: boolean } | undefined)?.archived === true)).toBe(false);
    await openArchived();
    await waitFor(() => expect(q('arcm-row-gone')).toBeTruthy());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'sessions_list' && (e.args as { archived?: boolean } | undefined)?.archived === true)).toBe(true);
  });

  it('restore round trip: the row leaves the trash and reappears live', async () => {
    boot([card('live')], [card('gone')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(q('arcm-restore-gone')).toBeTruthy());
    fireEvent.click(q('arcm-restore-gone')!);
    await waitFor(() => expect(q('arcm-row-gone')).toBeNull());
    await waitFor(() => expect(screen.getByTestId('srow-gone')).toBeTruthy());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_restore' && (e.args as { session?: string })?.session === 'gone')).toBe(true);
  });

  it('delete asks once, then unlinks the archived file (Z-20)', async () => {
    boot([card('live')], [card('gone')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(q('arcm-row-gone')).toBeTruthy());
    fireEvent.click(q('arcm-row-gone')!.querySelector('[aria-label="Delete for good"]')!);
    // Armed, not sent.
    expect(world.log.some((e) => e.op === 'session_delete')).toBe(false);
    fireEvent.click([...q('arcm-row-gone')!.querySelectorAll('button')].find((b) => b.textContent === 'Delete')!);
    await waitFor(() => expect(q('arcm-row-gone')).toBeNull());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_delete' && (e.args as { session?: string })?.session === 'gone')).toBe(true);
  });

  it('delete all confirms, then empties the trash (trash_empty)', async () => {
    boot([card('live')], [card('g1'), card('g2')]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(q('arcm-row-g2')).toBeTruthy());
    fireEvent.click(q('arcm-empty')!);
    expect(world.log.some((e) => e.op === 'trash_empty')).toBe(false);
    fireEvent.click(q('arcm-empty-confirm')!);
    await waitFor(() => expect(q('arcm-row-g1')).toBeNull());
    expect(world.log.some((e) => e.kind === 'request' && e.op === 'trash_empty')).toBe(true);
  });

  it('an empty trash says so instead of rows', async () => {
    boot([card('live')], []);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(document.querySelector('[data-testid=archived-modal]')!.textContent).toContain('Nothing archived.'));
  });

  it('project options fuzzy-search, sort by latest archived update, and collapse general workspaces', async () => {
    const archived = [
      card('old-proj', { cwd: '/work/alpha', updated_at: 10 }),
      card('new-proj', { cwd: '/work/beta', updated_at: 30 }),
      card('general-a', { cwd: '/home/you', updated_at: 20 }),
      card('general-b', { cwd: '/home/you/.local/share/oino/scratch/11494a503f24', updated_at: 40 }),
      card('uuid', { cwd: '/work/.od-projects/06f493f8-cd36-4e13-a012-123456789abc', updated_at: 50 }),
      card('hex', { cwd: '/work/pytest-run/jobs/11494a503f24', updated_at: 60 }),
    ];
    expect(archivedView(archived, '', '__general__', 'newest')).toHaveLength(4);
    expect(isGeneralWorkspaceCwd('/work/.od-projects/06f493f8-cd36-4e13-a012-123456789abc')).toBe(true);
    boot([], archived);
    await openArchived();
    fireEvent.focus(screen.getByTestId('arcm-project'));
    const options = () => [...document.querySelectorAll('[role=option]')].map((el) => el.textContent);
    await waitFor(() => expect(options()).toEqual(['All projects', 'General workspace', 'beta', 'alpha']));
    expect(archivedView(archived, '', 'all', 'newest').map((row) => row.session_id).slice(0, 3)).toEqual(['hex', 'uuid', 'general-b']);
    fireEvent.input(screen.getByTestId('arcm-project'), { target: { value: 'gen' } });
    await waitFor(() => expect(options()).toEqual(['General workspace']));
    fireEvent.keyDown(screen.getByTestId('arcm-project'), { key: 'Enter' });
    await waitFor(() => expect(q('arcm-row-uuid')).toBeTruthy());
    expect((q('arcm-project') as HTMLInputElement).value).toBe('General workspace');
    expect(q('arcm-row-hex')).toBeTruthy();
    fireEvent.input(screen.getByTestId('arcm-project'), { target: { value: 'alph' } });
    await waitFor(() => expect(options()).toEqual(['alpha']));
    fireEvent.keyDown(screen.getByTestId('arcm-project'), { key: 'Escape' });
    expect(q('arcm-project')!.getAttribute('aria-expanded')).toBe('false');
    expect(q('archived-modal')).toBeTruthy();
    fireEvent.focus(screen.getByTestId('arcm-project'));
    fireEvent.click(screen.getByText('alpha', { selector: '[role=option]' }));
    await waitFor(() => expect(q('arcm-row-old-proj')).toBeTruthy());
    expect(q('arcm-row-general-a')).toBeNull();
  });

  it('General workspace sorts by its newest session (not pinned) and no uuid/tmp folder name appears in the menu or rows', async () => {
    const archived = [
      card('beta', { cwd: '/work/beta', updated_at: 90 }),
      card('tmp', { cwd: '/tmp/9f8e7d6c5b4a3210', updated_at: 5 }),
      card('py', { cwd: '/var/pytest-of-you/pytest-3/case0', updated_at: 6 }),
      card('alpha', { cwd: '/work/alpha', updated_at: 50 }),
    ];
    boot([], archived);
    await openArchived();
    fireEvent.focus(screen.getByTestId('arcm-project'));
    const options = () => [...document.querySelectorAll('[role=option]')].map((el) => el.textContent);
    await waitFor(() => expect(options()).toEqual(['All projects', 'beta', 'alpha', 'General workspace']));
    const text = q('archived-modal')!.textContent ?? '';
    expect(text).not.toMatch(/9f8e7d6c5b4a3210|pytest-3|case0/);
    expect(q('arcm-row-tmp')!.textContent).toContain('General workspace');
    fireEvent.input(screen.getByTestId('arcm-project'), { target: { value: 'wrkspace' } });
    await waitFor(() => expect(options()).toEqual(['General workspace']));
  });

  it('Esc and an outside pointer close only the project dropdown, never the modal', async () => {
    boot([], [card('a', { cwd: '/work/alpha' })]);
    await openArchived();
    const input = q('arcm-project')!;
    const expanded = () => input.getAttribute('aria-expanded');
    const back = vi.fn();
    window.addEventListener('keydown', back);
    fireEvent.focus(input);
    expect(expanded()).toBe('true');
    // Esc from an option (focus on the list, not the input) is swallowed by the picker.
    fireEvent.keyDown(document.querySelector('[role=option]')!, { key: 'Escape', bubbles: true });
    expect(expanded()).toBe('false');
    expect(back).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(back).not.toHaveBeenCalled();
    window.removeEventListener('keydown', back);
    fireEvent.focus(input);
    expect(expanded()).toBe('true');
    fireEvent.pointerDown(q('arcm-search')!);
    expect(expanded()).toBe('false');
    expect(q('archived-modal')).toBeTruthy();
    fireEvent.focus(input);
    fireEvent.pointerDown(document.body);
    expect(expanded()).toBe('false');
    expect(q('archived-modal')).toBeTruthy();
  });

  it('closes the project dropdown when focus leaves the picker', async () => {
    boot([], [card('a', { cwd: '/work/alpha' })]);
    await openArchived();
    const input = q('arcm-project')!;
    fireEvent.focus(input);
    expect(input.getAttribute('aria-expanded')).toBe('true');
    fireEvent.focusOut(input, { relatedTarget: q('arcm-search') });
    await waitFor(() => expect(input.getAttribute('aria-expanded')).toBe('false'));
    expect(q('archived-modal')).toBeTruthy();
  });

  it('archived rows render an openable link with full title tooltip, and parseArchivedJsonl extracts read-only transcript entries (D-17)', async () => {
    boot([card('live')], [card('gone', { name: 'Hello Number 2 Reply' })]);
    await waitFor(() => expect(screen.getByTestId('srow-live')).toBeTruthy());
    await openArchived();
    await waitFor(() => expect(q('arcm-row-gone')).toBeTruthy());
    expect(q('arcm-row-gone')!.querySelector('.arcm-open')!.getAttribute('title')).toBe('Hello Number 2 Reply');
    // search + project filter
    const many = [card('a', { name: 'Fix login', cwd: '/p/web' }), card('b', { name: 'Docs', cwd: '/p/site' })];
    expect(archivedView(many, 'login', 'all', 'newest').map((c) => c.session_id)).toEqual(['a']);
    expect(archivedView(many, '', 'site', 'newest').map((c) => c.session_id)).toEqual(['b']);

    const parsed = parseArchivedJsonl(
      [
        JSON.stringify({ type: 'session', version: 3, id: 'gone', cwd: '/home/example' }),
        JSON.stringify({ type: 'session_info', id: 'i1', parentId: null, name: 'Hello Number 2 Reply' }),
        JSON.stringify({ type: 'message', id: 'u1', parentId: 'i1', message: { role: 'user', content: 'ping' } }),
        JSON.stringify({ type: 'message', id: 'a1', parentId: 'u1', message: { role: 'assistant', content: 'pong' } }),
      ].join('\n'),
    );
    expect(parsed.sessionName).toBe('Hello Number 2 Reply');
    expect(parsed.cwd).toBe('/home/example');
    expect(parsed.entries).toHaveLength(3);
    expect(parsed.leafId).toBe('a1');
  });
});
