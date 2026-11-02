// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Board, { pickProject } from './Board';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setBoardTransport, type BoardAvailability } from '../lib/board/transport';
import { setHostClientFactory } from '../lib/host/factory';
import { fakeBoard, task } from '../lib/board/testing';
import { createMockBoard, type MockBoardProject } from '../dev/mock/board';
import { resetBoardStore } from '../lib/board/store';
import { resetBoardUi } from '../lib/board/ui';
import { resetPcAvailability } from '../lib/board/pcs';
import { resetBoardActions, setBoardActions } from '../lib/board/actions';
import type { Task } from '../lib/board/api';
import { Toasts } from '../components/Toasts';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [
  { id: 'pc1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const },
  { id: 'pc2', name: 'laptop', ticket: 't', paired_at: 0, last_path: 'relay' as const },
];

function boardTasks(): Task[] {
  return [
    task({ id: 'S-1', title: 'Offline login', status: 'todo', priority: 'high', labels: ['auth'], deps: ['S-2'], waitingFor: ['S-2'] }),
    task({ id: 'S-2', title: 'Token cache', status: 'todo', priority: 'medium', labels: ['auth'] }),
    task({ id: 'S-3', title: 'Dark mode polish', status: 'todo', labels: ['ui'], body: 'settings screen', deps: ['S-4'], waitingFor: ['S-4'] }),
    task({ id: 'S-4', title: 'Crash on rotate', status: 'in_progress', priority: 'urgent', run: { session: 's1', started: new Date().toISOString() } }),
    task({ id: 'S-5', title: 'Upgrade iroh', status: 'blocked', blockedReason: { text: 'Need **a decision**', actor: 'agent', at: '' } }),
    task({ id: 'S-6', title: 'Pairing polish', status: 'in_review', deps: ['S-7'], lockedBy: ['S-7'] }),
    task({ id: 'S-7', title: 'Old idea', status: 'backlog' }),
  ];
}

function boardProjects(): MockBoardProject[] {
  return [
    { slug: 'scratch-1', name: 'scratch', prefix: 'S', tasks: boardTasks() },
    { slug: 'other-2', name: 'other', prefix: 'O', tasks: [task({ id: 'O-1', title: 'Older work', updated: '2026-10-01T00:00:00Z' })] },
    { slug: 'old-3', name: 'old', prefix: 'X', archived: true, tasks: [] },
  ];
}

function setup(options: { hostList?: typeof hosts; availability?: Record<string, BoardAvailability>; failingTasks?: boolean } = {}) {
  const projects = boardProjects();
  const board = createMockBoard(projects);
  const fake = fakeBoard({
    availability: options.availability,
    handlers: options.failingTasks
      ? { 'list --all': () => ({ ok: false, error: 'tasks dir unreadable', kind: 'io' }) }
      : undefined,
    fallback: ({ args, project }) => board.cli(args, project),
  });
  setBoardTransport(fake.transport);
  // Attachment staging (blob_put_bytes → fs_import → attach → fs_delete).
  setHostClientFactory((hostId) => ({
    hostId,
    conn: () => ({ state: 'connected' }),
    retry: async () => {},
    request: async <T,>(op: string) => (op === 'blob_put_bytes' ? ({ sha256: 'mock-sha' } as T) : (null as T)),
    openEvents: () => ({ close() {} }),
    openSession: () => {
      throw new Error('not needed');
    },
    openTerm: () => {
      throw new Error('not needed');
    },
    openLsp: () => {
      throw new Error('not needed');
    },
  }));
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts') return { ok: true, value: options.hostList ?? hosts };
    return { ok: true, value: null };
  });
  render(() => (
    <HashRouter>
      <Route
        path="*"
        component={() => (
          <SettingsProvider>
            <HostsProvider>
              <Board />
              {/* Z-85: board toasts render on the app-wide stack */}
              <Toasts />
            </HostsProvider>
          </SettingsProvider>
        )}
      />
    </HashRouter>
  ));
  return { fake, board };
}

beforeEach(() => {
  localStorage.clear();
  resetBoardStore();
  resetBoardUi();
  resetPcAvailability();
  resetBoardActions();
});
afterEach(() => {
  cleanup();
  setBoardTransport(undefined);
  setHostClientFactory(undefined);
  vi.resetAllMocks();
  location.hash = '#/';
});

const pill = (lane: string) => document.querySelector<HTMLElement>(`.lane-pill[data-lane=${lane}]`)!;

describe('Board', () => {
  it('opens the most recently active project and shows lanes with counts', async () => {
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    expect(screen.getByLabelText('Project: scratch')).toBeInTheDocument();
    expect(screen.getByLabelText(/^PC: coffee/)).toBeInTheDocument();
    // Backlog is off by default but shows by itself while it has cards (S-7); archive hidden.
    expect(within(pill('backlog')).getByText('1')).toBeInTheDocument();
    expect(pill('archived')).toBeNull();
    expect(within(pill('todo')).getByText('3')).toBeInTheDocument();
    expect(within(pill('in_progress')).getByText('1')).toBeInTheDocument();
    expect(within(pill('cancelled')).getByText('0')).toBeInTheDocument();
    // Card vocabulary: dep state, agent chip, blocked reason as plain text, priority.
    const card = document.querySelector<HTMLElement>('[data-id="S-1"]')!;
    // S-1 runs after S-2 in the same lane: drawn as a chain, no "after" tag.
    expect(card.dataset.chain).toBe('last');
    expect(document.querySelector<HTMLElement>('[data-id="S-2"]')!.dataset.chain).toBe('first');
    expect(card.querySelector('[data-dep-state]')).toBeNull();
    expect(document.querySelector('[data-id="S-3"] [data-dep-state=waiting]')).not.toBeNull();
    expect(document.querySelector('[data-id="S-6"] [data-dep-state=locked]')).not.toBeNull();
    expect(within(card).getByText('High')).toBeInTheDocument();
    expect(document.querySelector('[data-id="S-4"] .agent-chip')).not.toBeNull();
    expect(within(document.querySelector<HTMLElement>('[data-id="S-5"]')!).getByText('Need a decision')).toBeInTheDocument();
    expect(screen.getByText('Nothing finished yet')).toBeInTheDocument(); // per-lane empty state
    expect(screen.getByText('Nothing cancelled')).toBeInTheDocument();
    expect(screen.getByText(/7 tasks/)).toBeInTheDocument();
  });

  it('remembers the last PC + project and falls back to settings.board.defaultProject', () => {
    const projects = [
      { slug: 'a-1', name: 'alpha', updatedAt: '2026-10-01T00:00:00Z' },
      { slug: 'b-2', name: 'beta', updatedAt: '2026-10-05T00:00:00Z' },
      { slug: 'c-3', name: 'gamma', archived: true, updatedAt: '2026-10-09T00:00:00Z' },
    ];
    expect(pickProject(projects, 'a-1', 'beta')).toBe('a-1');
    expect(pickProject(projects, 'gone', 'beta')).toBe('b-2');
    expect(pickProject(projects, undefined, 'A-1')).toBe('a-1');
    expect(pickProject(projects, undefined, null)).toBe('b-2');
    expect(pickProject([], undefined, null)).toBeUndefined();
  });

  it('filters: toggling a priority narrows the lanes and the header count', async () => {
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: /Filter/ }));
    const sheet = await screen.findByTestId('filter-sheet');
    fireEvent.click(within(sheet).getByRole('button', { name: /^High$/ }));
    await waitFor(() => expect(within(pill('todo')).getByText('1')).toBeInTheDocument());
    expect(screen.getByText('1 of 7 tasks')).toBeInTheDocument();
    expect(within(sheet).getByRole('button', { name: 'Show 1 task' })).toBeInTheDocument();
    fireEvent.click(within(sheet).getByRole('button', { name: 'Clear' }));
    await waitFor(() => expect(within(pill('todo')).getByText('3')).toBeInTheDocument());
  });

  it('card tap / long-press go through the action hooks', async () => {
    const openTask = vi.fn();
    const openCardMenu = vi.fn();
    setBoardActions({ openTask, openCardMenu });
    setup();
    const card = await waitFor(() => {
      const element = document.querySelector<HTMLElement>('[data-id="S-2"]');
      expect(element).not.toBeNull();
      return element!;
    });
    fireEvent.click(card);
    expect(openTask).toHaveBeenCalledWith(expect.objectContaining({ id: 'S-2' }));
    fireEvent.contextMenu(card);
    expect(openCardMenu).toHaveBeenCalledWith(expect.objectContaining({ id: 'S-2' }), expect.anything());
  });

  it('new task: validation, defaults and the submitted payload', async () => {
    const { fake } = setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    // On the Todo page the FAB creates into Todo.
    fireEvent.click(screen.getByLabelText('New task in Todo'));
    const sheet = await screen.findByTestId('new-task-sheet');
    const create = within(sheet).getByRole('button', { name: 'Create task' });
    expect(create).toBeDisabled(); // title or description required
    fireEvent.input(within(sheet).getByLabelText('Title (optional)'), { target: { value: '  Retry banner copy ' } });
    expect(create).not.toBeDisabled();
    expect(within(sheet).getByRole('radio', { name: /Todo/ })).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(within(sheet).getByRole('radio', { name: 'Urgent' }));
    // Labels picker: pick an existing one and create a new one.
    fireEvent.click(within(sheet).getByRole('button', { name: /Add labels/ }));
    fireEvent.click(await within(sheet).findByRole('option', { name: /auth/ }));
    fireEvent.input(within(sheet).getByLabelText('Filter labels'), { target: { value: 'copy' } });
    fireEvent.click(within(sheet).getByRole('button', { name: /Create copy/ }));
    fireEvent.click(within(sheet).getByRole('button', { name: 'Back to the form' }));
    // Dependencies picker.
    fireEvent.click(within(sheet).getByRole('button', { name: /Add dependency/ }));
    fireEvent.click(await within(sheet).findByRole('option', { name: /S-3/ }));
    fireEvent.click(within(sheet).getByRole('button', { name: 'Back to the form' }));
    fireEvent.click(within(sheet).getByRole('button', { name: 'Create task' }));
    await waitFor(() => expect(fake.sent('add').length).toBeGreaterThan(0));
    const createCall = fake.sent('add').at(-1)!;
    expect(createCall.hostId).toBe('pc1');
    expect(createCall.project).toBe('scratch-1');
    expect(createCall.args).toEqual([
      'add', 'Retry banner copy', '--status', 'todo', '--priority', 'urgent', '--after', 'S-3', '--label', 'auth', '--label', 'copy', '--new-label',
    ]);
    await waitFor(() => expect(screen.getByText(/Created S-\d+/)).toBeInTheDocument());
    await waitFor(() => expect(screen.queryByTestId('new-task-sheet')).toBeNull());
  });

  it('new task: picked files upload after creation and land in the body', async () => {
    const { fake } = setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('New task in Todo'));
    const sheet = await screen.findByTestId('new-task-sheet');
    fireEvent.input(within(sheet).getByLabelText('Description'), { target: { value: 'See the screenshot' } });
    const file = new File([new Uint8Array([137, 80, 78, 71])], 'shot.png', { type: 'image/png' });
    fireEvent.change(within(sheet).getByLabelText('Attach files'), { target: { files: [file] } });
    expect(within(sheet).getByText('shot.png')).toBeInTheDocument();
    fireEvent.click(within(sheet).getByRole('button', { name: 'Create task' }));
    // The bytes are staged on the host (`blob_put_bytes` + `fs_import`) and handed
    // to the CLI's `attach`, then the markdown lands in the body through `edit`.
    const upload = await waitFor(() => {
      const call = fake.sent('attach').at(0);
      expect(call).toBeDefined();
      return call!;
    });
    expect(upload.args[0]).toBe('attach');
    expect(upload.args.at(-1)).toBe('shot.png');
    expect(upload.args.at(-2)).toBe('--name');
    const edit = await waitFor(() => {
      const call = fake.sent('edit').at(0);
      expect(call).toBeDefined();
      return call!;
    });
    expect(edit.args.join(' ')).toMatch(/^edit S-\d+ --body=See the screenshot\n\n!\[shot\.png\]\(att:S-\d+\/aaaa\d+-shot\.png\)$/);
  });

  it('new task: closing a dirty draft asks first', async () => {
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('New task in Todo'));
    const sheet = await screen.findByTestId('new-task-sheet');
    fireEvent.input(within(sheet).getByLabelText('Description'), { target: { value: 'draft' } });
    fireEvent.click(within(sheet).getByRole('button', { name: 'Close' }));
    expect(within(sheet).getByText('Discard this draft?')).toBeInTheDocument();
    fireEvent.click(within(sheet).getByRole('button', { name: 'Keep editing' }));
    expect(within(sheet).queryByText('Discard this draft?')).toBeNull();
    fireEvent.click(within(sheet).getByRole('button', { name: 'Close' }));
    fireEvent.click(within(sheet).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(screen.queryByTestId('new-task-sheet')).toBeNull());
  });

  it('search finds tasks by body text and jumps to them', async () => {
    const openTask = vi.fn();
    setBoardActions({ openTask });
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('Search'));
    const sheet = await screen.findByTestId('search-sheet');
    fireEvent.input(within(sheet).getByLabelText('Search'), { target: { value: 'settings screen' } });
    const result = await within(sheet).findByRole('option', { name: /S-3/ });
    expect(within(sheet).queryByRole('option', { name: /S-2/ })).toBeNull();
    fireEvent.click(result);
    await waitFor(() => expect(openTask).toHaveBeenCalledWith(expect.objectContaining({ id: 'S-3' })));
  });

  it('search "Filter the board by" keeps the text as a board filter', async () => {
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('Search'));
    const sheet = await screen.findByTestId('search-sheet');
    fireEvent.input(within(sheet).getByLabelText('Search'), { target: { value: 'auth' } });
    fireEvent.click(within(sheet).getByRole('button', { name: /Filter the board by/ }));
    await waitFor(() => expect(within(pill('todo')).getByText('2')).toBeInTheDocument());
    expect(screen.getByLabelText('Clear search filter')).toBeInTheDocument();
  });

  it('an offline PC shows its state with Retry, other PCs stay pickable', async () => {
    localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc2', projects: {} }));
    setup({ availability: { pc2: { state: 'offline' } } });
    await waitFor(() => expect(screen.getByText("Can't reach laptop")).toBeInTheDocument());
    // F60: the same OfflineState as Files/Chat/Preview (+ Switch PC with more than one PC).
    expect(screen.getByTestId('offline-state')).toBeInTheDocument();
    expect(screen.getByText('Is Unipi running on the PC?')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Switch PC' })).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('PC: laptop'));
    const sheet = await screen.findByTestId('pc-sheet');
    await waitFor(() => expect(within(sheet).getByText('Offline')).toBeInTheDocument());
    fireEvent.click(within(sheet).getByText('coffee'));
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
  });

  it('a not-installed PC shows the install hint', async () => {
    localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc2', projects: {} }));
    setup({ availability: { pc2: { state: 'not_installed', hint: 'Install unipi on this PC: pi install npm:@pi-unipi/unipi@alpha' } } });
    await waitFor(() => expect(screen.getByText("The board isn't set up on laptop")).toBeInTheDocument());
    expect(screen.getByText('Install unipi on this PC: pi install npm:@pi-unipi/unipi@alpha')).toBeInTheDocument();
  });

  it('a failing tasks request shows the error with Retry', async () => {
    setup({ failingTasks: true });
    await waitFor(() => expect(screen.getByText("Couldn't load the board")).toBeInTheDocument());
    expect(screen.getByText('tasks dir unreadable')).toBeInTheDocument();
  });

  it('without paired hosts the board says so (no synthetic web PC in v2)', async () => {
    setup({ hostList: [] });
    await waitFor(() => expect(screen.getByText('No PC paired yet')).toBeInTheDocument());
    expect(screen.getByTestId('no-host-state')).toBeInTheDocument();
  });

  it('list view groups by lane; Review queue scopes it', async () => {
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('radio', { name: 'List' }));
    const list = await screen.findByTestId('board-list');
    expect(within(list).getByLabelText('Todo, 3 tasks')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Review queue/ }));
    await waitFor(() => expect(within(screen.getByTestId('board-list')).queryByLabelText('Todo, 3 tasks')).toBeNull());
    expect(within(screen.getByTestId('board-list')).getByLabelText('In Review, 1 tasks')).toBeInTheDocument();
  });
});
