/**
 * Sessions sidebar tests (WP-A4): host groups (Z-8), grouping and ordering
 * (Z-9/D-9), paging (Z-11), row content (Z-12), the Pinned group (Z-14) and
 * pin/unpin through the hub meta op (§3.14). All against the mock host
 * (`createMockHost`), like the shell suite.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { resetHostCollapse } from './HostGroup';
import type { SessionCard } from '../../lib/host/client';
import { SessionList } from './SessionList';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
  ...over,
});

const scenario = (cards: SessionCard[]): MockScenario => ({
  name: 't',
  hosts: [
    { id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards },
    { id: 'pc-x', name: 'coffee', conn: { state: 'connected' }, cards: [card('x1', { cwd: '/home/me/other' })] },
  ],
});

let world: ReturnType<typeof createMockHost>;
const boot = (cards: SessionCard[], hash = '#/') => {
  world = createMockHost(scenario(cards));
  setHostClientFactory(world.client);
  location.hash = hash;
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

const rowsOf = (base: HTMLElement) => [...base.querySelectorAll('.srow')].map((r) => r.getAttribute('data-testid')!.slice('srow-'.length));

beforeEach(() => {
  localStorage.clear();
  resetHostCollapse();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('SessionList', () => {
  it('groups by host (This computer first, Z-8) and by project (D-9), ordered pinned → running → needs-you → recent (Z-9)', async () => {
    const cards = [
      card('idle-old', { updated_at: 100 }),
      card('working', { run_state: 'working', updated_at: 200 }),
      card('needsyou', { needs_you: true, updated_at: 150 }),
      card('recent', { updated_at: 900 }),
      card('other-proj', { cwd: '/home/you/p/website' }),
    ];
    const { container } = boot(cards);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    const hosts = [...container.querySelectorAll('.sb-host')].map((h) => h.getAttribute('data-testid'));
    expect(hosts).toEqual(['sb-host-this-pc', 'sb-host-pc-x']);
    expect(screen.getByText('This computer')).toBeTruthy();
    const oino = screen.getByTestId('sb-rows-oino');
    expect(rowsOf(oino)).toEqual(['working', 'needsyou', 'recent', 'idle-old']);
    expect(rowsOf(screen.getByTestId('sb-rows-website')!)).toEqual(['other-proj']);
    // Projects sort by latest activity (Z-9).
    expect(container.querySelector('.sb-project [data-testid="sb-project-head"]')?.textContent).toContain('website');
  });

  it('pins: a separate Pinned group on top, not repeated in the projects (Z-14); pin/unpin sends session_meta_set (§3.14)', async () => {
    const cards = [card('a', { pinned: true }), card('b'), card('c')];
    boot(cards);
    await waitFor(() => expect(screen.getByTestId('sb-pinned')).toBeTruthy());
    expect(rowsOf(screen.getByTestId('sb-pinned')!)).toEqual(['a']);
    expect(rowsOf(screen.getByTestId('sb-rows-oino')!)).toEqual(['b', 'c']);
    // Pin b from its row hover button → the op goes out and the card follows.
    fireEvent.click(screen.getByTestId('srow-pin-b'));
    await waitFor(() => expect(world.log.some((e) => e.op === 'session_meta_set' && (e.args as { session?: string }).session === 'b' && (e.args as { pinned?: boolean }).pinned === true)).toBe(true));
    await waitFor(() => expect(rowsOf(screen.getByTestId('sb-pinned')!)).toEqual(['a', 'b']));
    fireEvent.click(screen.getByTestId('srow-pin-a'));
    await waitFor(() => expect(rowsOf(screen.getByTestId('sb-pinned')!)).toEqual(['b']));
  });

  it('shows 5 per project, "Show more" adds 5, hidden at the end, and reset by collapsing the host (Z-11)', async () => {
    const cards = Array.from({ length: 12 }, (_, i) => card(`s${String(i).padStart(2, '0')}`, { updated_at: 1000 - i }));
    boot(cards);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    const rows = screen.getByTestId('sb-rows-oino')!;
    expect(rows.querySelectorAll('.srow')).toHaveLength(5);
    const more = screen.getByTestId('sb-more-oino');
    expect(more.getAttribute('title')).toContain('5 more of 7');
    fireEvent.click(more);
    expect(rows.querySelectorAll('.srow')).toHaveLength(10);
    fireEvent.click(screen.getByTestId('sb-more-oino'));
    expect(rows.querySelectorAll('.srow')).toHaveLength(12);
    expect(screen.queryByTestId('sb-more-oino')).toBeNull();
    // Collapsing the host hides the rows; expanding starts at 5 again (WSTP-05).
    fireEvent.click(screen.getByTestId('sb-host-head-this-pc'));
    expect(screen.queryByTestId('sb-rows-oino')).toBeNull();
    fireEvent.click(screen.getByTestId('sb-host-head-this-pc'));
    expect(screen.getByTestId('sb-rows-oino')!.querySelectorAll('.srow')).toHaveLength(5);
  });

  it('a project over 50 sessions renders its rows windowed, not all at once', async () => {
    const cards = Array.from({ length: 60 }, (_, i) => card(`v${String(i).padStart(2, '0')}`, { updated_at: 5000 - i }));
    boot(cards);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    const more = () => screen.getByTestId('sb-more-oino');
    for (let i = 0; i < 11; i++) fireEvent.click(more());
    const rows = screen.getByTestId('sb-rows-oino')!;
    const rendered = rows.querySelectorAll('.srow').length;
    expect(rendered).toBeGreaterThan(10);
    expect(rendered).toBeLessThan(60);
    expect(rows.classList.contains('sb-rows-virtual')).toBe(true);
  });

  it('row content: state word + relative time + changes + unread dot + cron badge + tooltip (Z-12)', async () => {
    const cards = [
      card('w', { run_state: 'working' }),
      card('n', { needs_you: true }),
      card('u', { unread: true, changes: { add: 12, del: 4 }, source: 'cron', name: 'Nightly build' }),
      card('z'),
    ];
    boot(cards);
    await waitFor(() => expect(screen.getByTestId('srow-u')).toBeTruthy());
    const w = screen.getByTestId('srow-w');
    expect(w.getAttribute('data-state')).toBe('working');
    expect(within(w).getByText('Working')).toBeTruthy();
    expect(within(w).getByText('2m ago')).toBeTruthy();
    const n = screen.getByTestId('srow-n');
    expect(within(n).getByText('Needs you')).toBeTruthy();
    const u = screen.getByTestId('srow-u');
    expect(u.querySelector('.srow-unread')).toBeTruthy();
    expect(within(u).getByText('+12')).toBeTruthy();
    expect(within(u).getByText('−4')).toBeTruthy();
    expect(u.querySelector('.srow-cron')).toBeTruthy();
    expect(u.getAttribute('title')).toBe('Nightly build');
    // Idle rows show no state word and omit .srow-sub when empty; never a π glyph.
    const idleRow = screen.getByTestId('srow-z');
    expect(within(idleRow).queryByText('Idle')).toBeNull();
    expect(idleRow.querySelector('.srow-sub')).toBeNull();
    expect(idleRow.textContent).not.toContain('π');
    expect(idleRow.querySelector('.srow-glyph svg')).toBeTruthy();
  });

  it('links to /s/:host/:session and marks the routed session active', async () => {
    boot([card('a'), card('b')], '#/s/this-pc/a');
    await waitFor(() => expect(screen.getByTestId('srow-a').getAttribute('aria-current')).toBe('page'));
    expect(screen.getByTestId('srow-b').getAttribute('href')).toBe('#/s/this-pc/b');
  });

  it('does not repeat folder name as subline when pinned session title already fell back to folder', async () => {
    boot([card('nameless', { name: '', cwd: '/tmp', pinned: true })]);
    await waitFor(() => expect(screen.getByTestId('srow-nameless')).toBeTruthy());
    const row = screen.getByTestId('srow-nameless');
    expect(row.getAttribute('title')).toBe('tmp');
    expect(row.querySelector('.srow-cwd')).toBeNull();
    expect(row.querySelector('.srow-sub')).toBeNull();
  });

  it('updates row title live on session_changed, groups home cwd under No workspace, and keeps pinned sidebar rows single-line (D-03, D-18, D-19)', async () => {
    boot([
      card('home-chat', { name: undefined, cwd: '/home/oi' }),
      card('pinned-proj', { name: 'Pinned reply', cwd: '/home/oi/qa1ws', pinned: true }),
    ]);
    await waitFor(() => expect(screen.getByTestId('sb-project-No workspace')).toBeTruthy());
    const homeRow = screen.getByTestId('srow-home-chat');
    expect(homeRow.getAttribute('title')).toBe('New chat');
    const pinnedRow = screen.getByTestId('srow-pinned-proj');
    expect(pinnedRow.querySelector('.srow-sub')).toBeNull();
    // Host pushes session_changed with the auto-generated title -> row updates live.
    world.emit({
      kind: 'session_changed',
      card: card('home-chat', { name: 'Ping Pong Reply', cwd: '/home/oi' }),
    });
    const hostEl = screen.getByTestId('sb-host-this-pc');
    await waitFor(() => expect(within(hostEl).getByTestId('srow-home-chat').getAttribute('title')).toBe('Ping Pong Reply'));
    expect(within(within(hostEl).getByTestId('srow-home-chat')).getByText('Ping Pong Reply')).toBeTruthy();
  });
});
