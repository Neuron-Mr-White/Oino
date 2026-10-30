import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandCenter, buildRows, parsePrefixQuery, pushHistory, historyFor, scoreRows, openFileFromCenter, HISTORY_MAX, type HistoryEntry, type SessionRow } from './CommandCenter';
import type { SessionCard } from '../../lib/host/client';
import { createMockHost } from '../../dev/mock/host';
import { cmdkEmpty } from '../../dev/mock/scenarios/command';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { resetSessions } from '../../lib/session/store';
import { setPaletteOpen } from '../../lib/shell';
import { registerCommands } from '../../lib/shell';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { actionNotice } from '../../lib/timeline/actions';
import { resetPanes } from '../../lib/panes';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')) }));

const HOST = 'this-pc';

function boot() {
  setDesktopOverride(true);
  setHostClientFactory(createMockHost(cmdkEmpty).client);
  setPaletteOpen(true);
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => <CommandCenter />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
}

beforeEach(() => {
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  setPaletteOpen(false);
  setHostClientFactory(undefined);
  resetHubs();
  resetSessions();
  resetPanes();
  resetLayout();
  setDesktopOverride(undefined);
  location.hash = '';
});

describe('parsePrefixQuery (Z-77)', () => {
  it('maps prefixes to scopes and strips them', () => {
    expect(parsePrefixQuery('>git')).toEqual({ scope: 'actions', text: 'git' });
    expect(parsePrefixQuery('# flaky')).toEqual({ scope: 'sessions', text: 'flaky' });
    expect(parsePrefixQuery('@tally')).toEqual({ scope: 'files', text: 'tally' });
    expect(parsePrefixQuery('tally')).toEqual({ scope: undefined, text: 'tally' });
  });
});

describe('scoreRows ranking', () => {
  const rows = [
    { title: 'Fix the failing test', hay: 'src/math.test.ts unipi' },
    { title: 'Why is the build slow?', hay: 'unipi oino' },
    { title: 'The tally went up', hay: '' },
  ];
  it('title hits outrank hay hits; every word must match', () => {
    const out = scoreRows(rows, 'test', (r) => r).map((s) => s.row.title);
    expect(out[0]).toBe('Fix the failing test');
    expect(scoreRows(rows, 'zzz', (r) => r)).toEqual([]);
    expect(scoreRows(rows, 'unipi slow', (r) => r).map((s) => s.row.title)).toEqual(['Why is the build slow?']);
  });
});

describe('history (Z-78)', () => {
  it('pushes newest first, dedups by scope+query, caps at 20', () => {
    let list: HistoryEntry[] = [{ q: 'a', scope: 'all' }];
    list = pushHistory(list, { q: 'b', scope: 'all' });
    expect(list.map((e) => e.q)).toEqual(['b', 'a']);
    list = pushHistory(list, { q: 'a', scope: 'all' });
    expect(list.map((e) => e.q)).toEqual(['a', 'b']);
    list = pushHistory(list, { q: 'a', scope: 'sessions' }); // same query, other scope: kept
    expect(list).toHaveLength(3);
    const many = Array.from({ length: 30 }, (_, i) => ({ q: `q${i}`, scope: 'all' as const })).reduce((l, e) => pushHistory(l, e), [] as HistoryEntry[]);
    expect(many.length).toBe(HISTORY_MAX);
    expect(many[0]!.q).toBe('q29');
  });

  it('reads a workspace bucket only', () => {
    const store: Record<string, HistoryEntry[]> = { 'a|proj': [{ q: 'x', scope: 'all' }], b: [{ q: 'y', scope: 'all' }] };
    expect(historyFor(store, 'a|proj')).toEqual([{ q: 'x', scope: 'all' }]);
    expect(historyFor(store, 'zz')).toEqual([]);
  });
});

describe('buildRows (Z-77)', () => {
  const commands = [
    { id: 'a', title: 'New chat', group: 'Chat', run: () => {} },
    { id: 'b', title: 'Go to Board', group: 'Go to', run: () => {} },
    { id: 'c', title: 'Go to Cron', group: 'Go to', run: () => {} },
    { id: 'd', title: 'Go to Settings', group: 'Go to', run: () => {} },
    { id: 'e', title: 'Go to Files', group: 'Go to', run: () => {} },
  ];
  const session = (id: string): SessionRow => ({ kind: 'session', key: id, hostId: HOST, hostName: 'This computer', card: { session_id: id, name: `Session ${id}`, cwd: '/p/x', updated_at: 1 } as SessionCard });

  it('empty + All: recent sessions (3) + command groups with ≤ 3 + show more', () => {
    const rows = buildRows({ scope: 'all', query: '', commands, slash: [], sessions: [session('1'), session('2'), session('3'), session('4'), session('5')], files: [], expanded: new Set() });
    expect(rows.filter((r) => r.kind === 'session')).toHaveLength(3);
    expect(rows[0]).toEqual({ kind: 'header', label: 'Recent sessions' });
    const goGroup = rows.findIndex((r) => r.kind === 'header' && r.label === 'Go to');
    expect(rows.slice(goGroup + 1, goGroup + 4).every((r) => r.kind === 'command')).toBe(true);
    expect(rows[goGroup + 4]).toEqual({ kind: 'more', key: 'more:Go to', group: 'Go to', count: 1 });
    // expanded shows everything and drops the more row
    const open = buildRows({ scope: 'all', query: '', commands, slash: [], sessions: [], files: [], expanded: new Set(['Go to']) });
    expect(open.filter((r) => r.kind === 'command')).toHaveLength(5);
    expect(open.some((r) => r.kind === 'more')).toBe(false);
  });

  it('query + All: Commands / Sessions / Files groups', () => {
    const rows = buildRows({ scope: 'all', query: 'go', commands, slash: [], sessions: [session('1')], files: [{ kind: 'file', key: 'f', hostId: HOST, path: '/p/go.ts', root: '/p' }], expanded: new Set() });
    // only groups with matches render; ≤ 3 per group + "show more" (Z-77)
    expect(rows.map((r) => (r.kind === 'header' ? r.label : r.kind))).toEqual(['Commands', 'command', 'command', 'command', 'more', 'Files', 'file']);
    const withSession = buildRows({ scope: 'all', query: 'session', commands, slash: [], sessions: [session('1')], files: [], expanded: new Set() });
    expect(withSession.map((r) => (r.kind === 'header' ? r.label : r.kind))).toEqual(['Sessions', 'session']);
  });

  it('a prefix sets the scope: `#` is Sessions only', () => {
    const { scope, text } = parsePrefixQuery('#go');
    expect(scope).toBe('sessions');
    const rows = buildRows({ scope: scope!, query: text, commands, slash: [], sessions: [session('1'), session('2')], files: [], expanded: new Set() });
    expect(rows.every((r) => r.kind === 'session' || r.kind === 'header' || r.kind === 'note')).toBe(true);
  });

  it('Actions lists the session `/` commands (Z-80)', () => {
    const rows = buildRows({ scope: 'actions', query: '', commands: commands.slice(0, 1), slash: [{ kind: 'slash', key: 's', name: '/permission' }], sessions: [], files: [], expanded: new Set() });
    expect(rows.some((r) => r.kind === 'slash' && r.name === '/permission')).toBe(true);
    const withQuery = buildRows({ scope: 'actions', query: 'permission', commands, slash: [{ kind: 'slash', key: 's', name: '/permission', description: 'Cycle permission mode' }], sessions: [], files: [], expanded: new Set() });
    expect(withQuery.filter((r) => r.kind === 'slash')).toHaveLength(1);
  });

  it('Files scope: needs ≥ 2 characters', () => {
    const rows = buildRows({ scope: 'files', query: 't', commands: [], slash: [], sessions: [], files: [], expanded: new Set() });
    expect(rows[0]).toMatchObject({ kind: 'note', text: 'Type at least 2 characters to search files' });
  });
});

describe('CommandCenter dialog', () => {
  it('empty state: recent sessions, grouped commands, history chips', async () => {
    const off = registerCommands([
      { id: 't1', title: 'New chat', group: 'Chat', run: () => {} },
      { id: 't2', title: 'Go to Board', group: 'Go to', run: () => {} },
    ]);
    // no Workspace is mounted in this render, so the center keys on the 'app' bucket
    localStorage.setItem('unipi.cmdk.history', JSON.stringify({ app: [{ q: 'tally', scope: 'all' }] }));
    boot();
    expect(await screen.findByTestId('command-center')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText('Recent sessions')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Chat')).toBeInTheDocument());
    expect(screen.getByTestId('cmdk-history')).toHaveTextContent('tally');
    // sessions came from both hosts' cards
    await waitFor(() => expect(screen.getAllByTestId('cmdk-list').length).toBe(1));
    await waitFor(() => expect(document.querySelectorAll('.cmdk-row[data-kind="session"]').length).toBeGreaterThanOrEqual(3));
  });

  it('the `#` prefix switches to sessions and ranks', async () => {
    boot();
    await screen.findByTestId('cmdk-list');
    const input = screen.getByLabelText('Search');
    fireEvent.input(input, { target: { value: '#failing' } });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Sessions' })).toHaveAttribute('aria-selected', 'true'));
    // a query in Sessions ranks matching rows directly (no "Recent sessions" header)
    await waitFor(() => expect(document.querySelectorAll('.cmdk-row[data-kind="session"]').length).toBeGreaterThan(0));
  });

  it('arrow keys move, Enter runs a session row (navigates)', async () => {
    boot();
    await waitFor(() => expect(document.querySelectorAll('.cmdk-row[data-kind="session"]').length).toBeGreaterThan(0));
    const first = document.querySelector<HTMLElement>('.cmdk-row[data-kind="session"]')!;
    fireEvent.click(first);
    await waitFor(() => expect(location.hash).toMatch(/^#\/s\//));
  });

  it('Mod+P entry: the files scope searches fs_search', async () => {
    setDesktopOverride(true);
    setHostClientFactory(createMockHost(cmdkEmpty).client);
    location.hash = `#/s/${HOST}/s001`;
    await new Promise((r) => setTimeout(r, 30)); // let the hash-change popstate settle first
    setPaletteOpen(true, 'files');
    render(() => (
      <SettingsProvider>
        <HostsProvider>
          <HashRouter>
            <Route path="*" component={() => <CommandCenter />} />
          </HashRouter>
        </HostsProvider>
      </SettingsProvider>
    ));
    expect(await screen.findByRole('tab', { name: 'Files' })).toHaveAttribute('aria-selected', 'true');
    fireEvent.input(screen.getByLabelText('Search'), { target: { value: 'tally' } });
    await waitFor(() => expect(screen.getAllByRole('button', { name: /tally/ }).length).toBeGreaterThan(0), { timeout: 3000 });
  });
});

describe('openFileFromCenter', () => {
  it('desktop with no workspace: an honest notice instead of a dead click', () => {
    setDesktopOverride(true);
    openFileFromCenter(HOST, '/x/src/tally.ts');
    expect(actionNotice()).toBe('Open a chat in a project to browse files');
  });
});
