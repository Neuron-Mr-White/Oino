// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Board, { pickProject } from './Board';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setBoardTransport } from '../lib/board/transport';
import { fakeBoard, project, task, type FakeBoardOptions } from '../lib/board/testing';
import { resetBoardStore } from '../lib/board/store';
import { resetBoardUi } from '../lib/board/ui';
import { resetPcAvailability } from '../lib/board/pcs';
import { resetBoardActions, setBoardActions } from '../lib/board/actions';
import type { Task } from '../lib/board/api';

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

function setup(options: { hostList?: typeof hosts; availability?: FakeBoardOptions['availability']; tasksStatus?: number } = {}) {
  let tasks = boardTasks();
  const fake = fakeBoard({
    availability: options.availability,
    routes: {
      'GET /api/projects': () => ({
        json: [
          project({ slug: 'scratch-1', name: 'scratch', counts: { todo: 3 }, updatedAt: '2026-10-07T00:00:00Z' }),
          project({ slug: 'other-2', name: 'other', updatedAt: '2026-10-01T00:00:00Z' }),
          project({ slug: 'old-3', name: 'old', archived: true }),
        ],
      }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: {}, commentRequired: {}, final: ['done'], chainGate: 'in_review', maxSessions: 2 } }),
      'GET /api/running': () => ({ json: { running: [] } }),
      'GET /api/projects/scratch-1/tasks': () =>
        options.tasksStatus ? { status: options.tasksStatus, json: { error: 'tasks dir unreadable' } } : { json: { tasks, problems: [] } },
      'POST /api/tasks/scratch-1/S-99/attachments': () => ({ json: { name: 'abc12345-shot.png', markdown: '![shot.png](att:S-99/abc12345-shot.png)' } }),
      'POST /api/tasks/scratch-1/S-99/edit': (req) => ({ json: task({ id: 'S-99', body: (req.json as { body: string }).body }) }),
      'POST /api/tasks/scratch-1/create': (req) => {
        const body = req.json as { title: string; status: string };
        const created = task({ id: 'S-99', title: body.title, status: body.status });
        tasks = [...tasks, created];
        return { json: created };
      },
    },
  });
  setBoardTransport(fake.transport);
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
            </HostsProvider>
          </SettingsProvider>
        )}
      />
    </HashRouter>
  ));
  return fake;
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
  vi.resetAllMocks();
  location.hash = '#/';
});

const pill = (lane: string) => document.querySelector<HTMLElement>(`.lane-pill[data-lane=${lane}]`)!;

describe('Board', () => {
  it('opens the most recently active project and shows lanes with counts', async () => {
    setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    expect(screen.getByLabelText('Project: scratch')).toBeInTheDocument();
    expect(screen.getByLabelText('PC: coffee')).toBeInTheDocument();
    // Backlog is off by default (settings.board.showBacklog), archive hidden.
    expect(pill('backlog')).toBeNull();
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
    expect(screen.getByText(/6 tasks/)).toBeInTheDocument();
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
    expect(screen.getByText('1 of 6 tasks')).toBeInTheDocument();
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
    const fake = setup();
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
    await waitFor(() => expect(fake.requests.some((r) => r.path === '/api/tasks/scratch-1/create')).toBe(true));
    const sent = fake.requests.find((r) => r.path === '/api/tasks/scratch-1/create')!;
    expect(sent.hostId).toBe('pc1');
    expect(sent.json).toEqual({ title: 'Retry banner copy', status: 'todo', priority: 'urgent', after: ['S-3'], labels: ['auth', 'copy'] });
    await waitFor(() => expect(screen.getByText('Created S-99')).toBeInTheDocument());
    await waitFor(() => expect(screen.queryByTestId('new-task-sheet')).toBeNull());
  });

  it('new task: picked files upload after creation and land in the body', async () => {
    const fake = setup();
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('New task in Todo'));
    const sheet = await screen.findByTestId('new-task-sheet');
    fireEvent.input(within(sheet).getByLabelText('Description'), { target: { value: 'See the screenshot' } });
    const file = new File([new Uint8Array([137, 80, 78, 71])], 'shot.png', { type: 'image/png' });
    fireEvent.change(within(sheet).getByLabelText('Attach files'), { target: { files: [file] } });
    expect(within(sheet).getByText('shot.png')).toBeInTheDocument();
    fireEvent.click(within(sheet).getByRole('button', { name: 'Create task' }));
    await waitFor(() => expect(fake.requests.some((r) => r.path.endsWith('/S-99/edit'))).toBe(true));
    const upload = fake.requests.find((r) => r.path.includes('/attachments'))!;
    expect(upload.path).toBe('/api/tasks/scratch-1/S-99/attachments?name=shot.png');
    expect(upload.body).toBeInstanceOf(Uint8Array);
    expect(fake.requests.find((r) => r.path.endsWith('/S-99/edit'))!.json).toEqual({
      body: 'See the screenshot\n\n![shot.png](att:S-99/abc12345-shot.png)',
    });
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
    expect(screen.getByText('Is unipi-host running on the PC?')).toBeInTheDocument();
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
    setup({ tasksStatus: 500 });
    await waitFor(() => expect(screen.getByText("Couldn't load the board")).toBeInTheDocument());
    expect(screen.getByText('tasks dir unreadable')).toBeInTheDocument();
  });

  it('in a browser without paired hosts the dev PC is used', async () => {
    setup({ hostList: [] });
    await waitFor(() => expect(screen.getByLabelText('PC: dev (vite proxy)')).toBeInTheDocument());
    await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
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
