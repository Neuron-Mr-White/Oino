// @vitest-environment jsdom
/**
 * UNI-131: task detail + every task / project action against a fake
 * board transport — exact request payloads, the comment / undo flows,
 * summaries, settings, project archive.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Board from './Board';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setBoardTransport } from '../lib/board/transport';
import { fakeBoard, project, task, type Handler } from '../lib/board/testing';
import { resetBoardStore } from '../lib/board/store';
import { resetBoardUi } from '../lib/board/ui';
import { resetPcAvailability } from '../lib/board/pcs';
import { resetBoardActions } from '../lib/board/actions';
import { resetSummaries } from '../lib/board/summarize';
import type { Rules, Task } from '../lib/board/api';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const STALE_HINT = 'why the stale run is being released';
const RULES: Rules = {
  statuses: [],
  allowedMoves: {
    backlog: ['todo', 'cancelled'],
    todo: ['backlog', 'cancelled'],
    in_progress: ['todo', 'backlog'],
    blocked: ['todo', 'done', 'cancelled'],
    in_review: ['done', 'todo', 'backlog', 'archived'],
    done: ['archived'],
    cancelled: ['archived'],
    archived: [],
  },
  commentRequired: { in_progress: { todo: STALE_HINT, backlog: STALE_HINT } },
  final: ['done', 'cancelled', 'archived'],
  chainGate: 'in_review',
  maxSessions: 2,
};

function seed(): Task[] {
  return [
    task({ id: 'S-1', title: 'Offline login', status: 'todo', priority: 'high', labels: ['auth'], deps: ['S-2'], waitingFor: ['S-2'], body: '## Goal\nWork **offline**.' }),
    task({ id: 'S-2', title: 'Token cache', status: 'todo', priority: 'medium', labels: ['auth'] }),
    task({ id: 'S-3', title: 'Dark mode polish', status: 'todo', labels: ['ui'] }),
    task({ id: 'S-4', title: 'Crash on rotate', status: 'in_progress', priority: 'urgent', staleness: 'stale', run: { session: 's1', pid: 42, host: 'pc', started: new Date().toISOString() } }),
    task({ id: 'S-5', title: 'Upgrade iroh', status: 'blocked', statusReport: { kind: 'blocked', text: 'Need **a decision**', actor: 'agent', at: '2026-10-07T00:00:00Z' } }),
    task({ id: 'S-6', title: 'Pairing polish', status: 'in_review', statusReport: { kind: 'review', text: '## Summary\nQR fixed', actor: 'agent', at: '2026-10-07T00:00:00Z' }, activity: [{ at: '2026-10-07T00:00:00Z', actor: 'agent', session: 's9', text: 'finished: ## Summary\nQR fixed' }] }),
    task({ id: 'S-7', title: 'Ship relay', status: 'done' }),
    task({ id: 'S-8', title: 'Old idea', status: 'backlog' }),
  ];
}

type Overrides = Record<string, Handler>;

function setup(overrides: Overrides = {}, options: { rules?: Rules } = {}) {
  let tasks = seed();
  const find = (id: string) => tasks.find((item) => item.id === id)!;
  const update = (id: string, patch: Partial<Task>) => {
    tasks = tasks.map((item) => (item.id === id ? { ...item, ...patch } : item));
    return find(id);
  };
  const routes: Record<string, Handler> = {
    'GET /api/projects': () => ({ json: [project({ slug: 'scratch-1', name: 'scratch', updatedAt: '2026-10-07T00:00:00Z' })] }),
    'GET /api/rules': () => ({ json: options.rules ?? RULES }),
    'GET /api/running': () => ({ json: { running: [] } }),
    'GET /api/projects/scratch-1/tasks': () => ({ json: { tasks, problems: [] } }),
    'GET /api/settings': () => ({
      json: {
        piCommand: ['pi'],
        models: [],
        summaryModel: '',
        summaryInstruction: 'Write a changelog.',
        defaultSummaryInstruction: 'Write a changelog.',
        sessions: { maxSessions: 2, turnAddLimit: 20, chainGate: 'in_review' },
        archive: { archiveAfterDays: 0, retentionDays: 90 },
      },
    }),
    'GET /api/models': () => ({ json: { models: ['anthropic/claude-x', 'openai/gpt-y'] } }),
    'PUT /api/settings': (req) => ({ json: req.json }),
    'PUT /api/projects/scratch-1': (req) => ({ json: project({ slug: 'scratch-1', name: 'scratch', archived: (req.json as { archived: boolean }).archived }) }),
    'POST /api/projects/scratch-1/done-lane': () => ({ json: { moved: ['S-6'] } }),
    'POST /api/projects/scratch-1/archive-lane': () => ({ json: { archived: ['S-7'], skipped: [] } }),
    'POST /api/projects/scratch-1/summarize': () => ({ json: { summary: '- feat: shipped relay (S-7)', taskIds: ['S-7'] } }),
    'POST /api/projects/scratch-1/archive-summary': () => ({ json: { path: '/pc/summaries/x.md', archived: ['S-7'], skipped: [] } }),
  };
  for (const id of tasks.map((item) => item.id)) {
    const base = `/api/tasks/scratch-1/${id}`;
    routes[`POST ${base}/move`] = (req) => {
      const body = req.json as { status: string };
      return { json: { ...update(id, { status: body.status }), undoToken: ['done', 'cancelled', 'archived'].includes(body.status) ? `tok-${id}` : undefined } };
    };
    routes[`POST ${base}/undo`] = () => ({ json: update(id, { status: seed().find((item) => item.id === id)!.status }) });
    routes[`POST ${base}/edit`] = (req) => ({ json: update(id, req.json as Partial<Task>) });
    routes[`POST ${base}/note`] = (req) => ({
      json: update(id, { activity: [...find(id).activity, { at: new Date().toISOString(), actor: 'user', text: (req.json as { text: string }).text }] }),
    });
    routes[`POST ${base}/link`] = (req) => ({ json: update(id, { deps: [...find(id).deps, (req.json as { dep: string }).dep] }) });
    routes[`POST ${base}/unlink`] = (req) => ({ json: update(id, { deps: find(id).deps.filter((dep) => dep !== (req.json as { dep: string }).dep) }) });
    routes[`POST ${base}/order`] = () => ({ json: find(id) });
    routes[`POST ${base}/duplicate`] = () => {
      const copy = task({ id: 'S-99', title: find(id).title, status: 'backlog' });
      tasks = [...tasks, copy];
      return { json: copy };
    };
    routes[`POST ${base}/attachments`] = (req) => {
      const name = new URLSearchParams(req.path.split('?')[1]).get('name') ?? 'file';
      return { json: { name: `abcd1234-${name}`, original: name, markdown: `![${name}](att:${id}/abcd1234-${name})`, kind: 'image', size: 4 } };
    };
  }
  Object.assign(routes, overrides);
  const fake = fakeBoard({ routes });
  setBoardTransport(fake.transport);
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts') return { ok: true, value: [{ id: 'pc1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' }] };
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
  const sent = (suffix: string) => fake.requests.filter((r) => r.path.split('?')[0]!.endsWith(suffix));
  return { fake, sent, find, update };
}

beforeEach(() => {
  localStorage.clear();
  resetBoardStore();
  resetBoardUi();
  resetPcAvailability();
  resetBoardActions();
  resetSummaries();
});
afterEach(() => {
  cleanup();
  setBoardTransport(undefined);
  vi.resetAllMocks();
  location.hash = '#/';
});

const card = (id: string) => document.querySelector<HTMLElement>(`.bcard[data-id="${id}"]`)!;
const pill = (lane: string) => document.querySelector<HTMLElement>(`.lane-pill[data-lane=${lane}]`)!;
async function ready() {
  await waitFor(() => expect(screen.getByText('Token cache')).toBeInTheDocument());
}
async function openDetail(id: string, lane?: string) {
  if (lane) fireEvent.click(pill(lane));
  fireEvent.click(card(id));
  return screen.findByTestId('task-detail');
}

describe('task detail', () => {
  it('re-fits the title height when its width changes (desktop resize / 900px boundary)', async () => {
    const observers: Array<{ cb: ResizeObserverCallback; el?: Element }> = [];
    vi.stubGlobal(
      'ResizeObserver',
      class {
        entry: { cb: ResizeObserverCallback; el?: Element };
        constructor(cb: ResizeObserverCallback) {
          this.entry = { cb };
          observers.push(this.entry);
        }
        observe(el: Element) {
          this.entry.el = el;
        }
        unobserve() {}
        disconnect() {}
      },
    );
    try {
      setup();
      await ready();
      const detail = await openDetail('S-1');
      const title = within(detail).getByLabelText('Task title') as HTMLTextAreaElement;
      const watcher = observers.find((o) => o.el === title);
      expect(watcher).toBeTruthy();
      let width = 400;
      let scroll = 60;
      Object.defineProperty(title, 'clientWidth', { configurable: true, get: () => width });
      Object.defineProperty(title, 'scrollHeight', { configurable: true, get: () => scroll });
      watcher!.cb([], {} as ResizeObserver);
      expect(title.style.height).toBe('60px');
      // Wider pane → fewer lines → shorter box (the bug: a stale tall box left a gap).
      width = 900;
      scroll = 30;
      watcher!.cb([], {} as ResizeObserver);
      expect(title.style.height).toBe('30px');
      // Same width (a height-only change, e.g. our own fit) does not loop.
      scroll = 99;
      watcher!.cb([], {} as ResizeObserver);
      expect(title.style.height).toBe('30px');
    } finally {
      vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    }
  });

  it('opens from a card: title, chips, rendered body, dependency state; Back returns to the board', async () => {
    setup();
    await ready();
    const detail = await openDetail('S-1');
    expect(within(detail).getByLabelText('Task title')).toHaveValue('Offline login');
    expect(within(detail).getByLabelText('Status: Todo')).toBeInTheDocument();
    expect(within(detail).getByLabelText('Priority: High')).toBeInTheDocument();
    expect(detail.querySelector('.body-card strong')?.textContent).toBe('offline');
    const dep = within(detail).getByLabelText('Open S-2');
    expect(within(dep).getByText('waiting')).toBeInTheDocument();
    // A dependency jump stacks: Back returns to S-1, then to the board.
    fireEvent.click(dep);
    await waitFor(() => expect(within(screen.getByTestId('task-detail')).getByLabelText('Task title')).toHaveValue('Token cache'));
    expect(within(screen.getByTestId('task-detail')).getByLabelText('Open S-1')).toBeInTheDocument(); // "Needed by"
    fireEvent.click(screen.getByLabelText('Back to the previous task'));
    await waitFor(() => expect(within(screen.getByTestId('task-detail')).getByLabelText('Task title')).toHaveValue('Offline login'));
    fireEvent.click(screen.getByLabelText('Back to the board'));
    await waitFor(() => expect(screen.queryByTestId('task-detail')).toBeNull());
  });

  it('shows the review report with its next steps and the activity report label', async () => {
    setup();
    await ready();
    const detail = await openDetail('S-6', 'in_review');
    const report = within(detail).getByLabelText('Review report');
    expect(within(report).getByText('QR fixed')).toBeInTheDocument();
    expect(within(report).getByRole('button', { name: /Mark done/ })).toBeInTheDocument();
    expect(within(detail).getByText('Finished')).toBeInTheDocument();
    fireEvent.click(within(report).getByRole('button', { name: /Read the full report/ }));
    const reader = await screen.findByTestId('report-reader');
    expect(within(reader).getByText('S-6 · Report')).toBeInTheDocument();
  });
});

describe('moves', () => {
  it('moves without a comment and offers Undo for final lanes', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-6', 'in_review');
    fireEvent.click(within(detail).getByRole('button', { name: /Mark done/ }));
    await waitFor(() => expect(sent('/S-6/move')).toHaveLength(1));
    expect(sent('/S-6/move')[0]!.json).toEqual({ status: 'done' });
    const undo = await screen.findByRole('button', { name: 'Undo' });
    expect(screen.getByText('Moved S-6 to Done')).toBeInTheDocument();
    fireEvent.click(undo);
    await waitFor(() => expect(sent('/S-6/undo')).toHaveLength(1));
    expect(sent('/S-6/undo')[0]!.json).toEqual({ token: 'tok-S-6' });
    await waitFor(() => expect(screen.getByText('Restored S-6 to In Review')).toBeInTheDocument());
  });

  it('the status menu lists only allowed moves', async () => {
    setup();
    await ready();
    const detail = await openDetail('S-2');
    fireEvent.click(within(detail).getByLabelText('Status: Todo'));
    const menu = await screen.findByRole('menu', { name: 'Move to' });
    const rows = within(menu).getAllByRole('menuitem').map((row) => row.textContent);
    expect(rows).toEqual(['Move to Backlog', 'Move to Cancelled']);
  });

  it('a stale run is released through the comment sheet (note required)', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-4', 'in_progress');
    fireEvent.click(within(detail).getByRole('button', { name: /Release to Todo/ }));
    const sheet = await screen.findByTestId('comment-sheet');
    expect(within(sheet).getByText(STALE_HINT)).toBeInTheDocument();
    const move = within(sheet).getByRole('button', { name: 'Release to Todo' });
    expect(move).toBeDisabled();
    fireEvent.input(within(sheet).getByLabelText('Note'), { target: { value: 'session died' } });
    expect(move).not.toBeDisabled();
    fireEvent.click(move);
    await waitFor(() => expect(sent('/S-4/move')).toHaveLength(1));
    expect(sent('/S-4/move')[0]!.json).toEqual({ status: 'todo', comment: 'session died' });
    await waitFor(() => expect(screen.queryByTestId('comment-sheet')).toBeNull());
  });

  it('a 409 needsComment refusal opens the comment sheet and retries with the note', async () => {
    let calls = 0;
    const { sent } = setup(
      {
        'POST /api/tasks/scratch-1/S-5/move': (req) => {
          calls += 1;
          const body = req.json as { status: string; comment?: string };
          if (!body.comment) return { status: 409, json: { error: 'blocked → done requires --comment (why)', needsComment: true } };
          return { json: task({ id: 'S-5', status: body.status }) };
        },
      },
      { rules: { ...RULES, commentRequired: {} } },
    );
    await ready();
    const detail = await openDetail('S-5', 'blocked');
    fireEvent.click(within(detail).getByLabelText('Status: Blocked'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Move to Done' }));
    // A menu move into a final lane asks first …
    const confirm = await screen.findByTestId('confirm-sheet');
    expect(sent('/S-5/move')).toHaveLength(0);
    fireEvent.click(within(confirm).getByRole('button', { name: 'Move to Done' }));
    // … then the 409 swaps in the comment sheet.
    const sheet = await screen.findByTestId('comment-sheet');
    fireEvent.input(within(sheet).getByLabelText('Note'), { target: { value: 'decided: pin 0.95' } });
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move to Done' }));
    await waitFor(() => expect(calls).toBe(2));
    expect(sent('/S-5/move').map((r) => r.json)).toEqual([{ status: 'done' }, { status: 'done', comment: 'decided: pin 0.95' }]);
  });

  it('rework from In Review takes an optional note', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-6', 'in_review');
    fireEvent.click(within(detail).getByRole('button', { name: /Rework/ }));
    const sheet = await screen.findByTestId('comment-sheet');
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move without note' }));
    await waitFor(() => expect(sent('/S-6/move')).toHaveLength(1));
    expect(sent('/S-6/move')[0]!.json).toEqual({ status: 'todo' });
  });
});

describe('edits', () => {
  it('title, priority, labels and description send exactly what changed', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-2');
    const title = within(detail).getByLabelText('Task title');
    fireEvent.input(title, { target: { value: 'Token cache v2' } });
    fireEvent.blur(title);
    await waitFor(() => expect(sent('/S-2/edit')).toHaveLength(1));
    expect(sent('/S-2/edit')[0]!.json).toEqual({ title: 'Token cache v2' });

    fireEvent.click(within(detail).getByLabelText('Priority: Medium'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Urgent/ }));
    await waitFor(() => expect(sent('/S-2/edit')).toHaveLength(2));
    expect(sent('/S-2/edit')[1]!.json).toEqual({ priority: 'urgent' });

    fireEvent.click(within(detail).getByLabelText('Edit labels'));
    const labels = await screen.findByTestId('labels-sheet');
    fireEvent.click(within(labels).getByRole('option', { name: /ui/ }));
    await waitFor(() => expect(sent('/S-2/edit')).toHaveLength(3));
    expect(sent('/S-2/edit')[2]!.json).toEqual({ labels: ['auth', 'ui'] });
    fireEvent.click(within(labels).getByRole('button', { name: 'Done' }));

    fireEvent.click(within(detail).getByLabelText('Edit description'));
    const editor = await screen.findByTestId('body-editor');
    fireEvent.input(within(editor).getByLabelText('Description (markdown)'), { target: { value: 'Cache it.' } });
    fireEvent.click(within(editor).getByLabelText('Bold'));
    fireEvent.click(within(editor).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(sent('/S-2/edit')).toHaveLength(4));
    // Bold with no selection inserts a placeholder at the caret (end of the text).
    expect(sent('/S-2/edit')[3]!.json).toEqual({ body: 'Cache it.**bold**' });
    await waitFor(() => expect(screen.queryByTestId('body-editor')).toBeNull());
  });

  it('a dirty description editor asks before discarding', async () => {
    setup();
    await ready();
    const detail = await openDetail('S-2');
    fireEvent.click(within(detail).getByLabelText('Edit description'));
    const editor = await screen.findByTestId('body-editor');
    fireEvent.input(within(editor).getByLabelText('Description (markdown)'), { target: { value: 'draft' } });
    fireEvent.click(within(editor).getByRole('button', { name: 'Cancel' }));
    expect(within(editor).getByText('Discard your changes?')).toBeInTheDocument();
    fireEvent.click(within(editor).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(screen.queryByTestId('body-editor')).toBeNull());
  });

  it('notes: @ mentions insert the id; Send posts the note; an unsent note guards Back', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-5', 'blocked');
    const field = within(detail).getByLabelText('Add a note') as HTMLTextAreaElement;
    field.value = 'pin it, see @S-';
    field.setSelectionRange(field.value.length, field.value.length);
    fireEvent.input(field);
    const strip = await within(detail).findByRole('listbox', { name: 'Mention a task' });
    fireEvent.click(within(strip).getByRole('option', { name: /S-3/ }));
    expect(field.value).toBe('pin it, see S-3 ');
    fireEvent.click(screen.getByLabelText('Back to the board'));
    expect(within(detail).getByText('Discard your unsent note?')).toBeInTheDocument();
    fireEvent.click(within(detail).getByRole('button', { name: 'Keep' }));
    fireEvent.click(within(detail).getByLabelText('Send note'));
    await waitFor(() => expect(sent('/S-5/note')).toHaveLength(1));
    expect(sent('/S-5/note')[0]!.json).toEqual({ text: 'pin it, see S-3' });
    await waitFor(() => expect(field.value).toBe(''));
  });

  it('notes: sending with the caret still on an @ mention closes the mention strip', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-5', 'blocked');
    const field = within(detail).getByLabelText('Add a note') as HTMLTextAreaElement;
    field.value = 'see @S-3';
    field.setSelectionRange(field.value.length, field.value.length);
    fireEvent.input(field);
    await within(detail).findByRole('listbox', { name: 'Mention a task' });
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(sent('/S-5/note')).toHaveLength(1));
    await waitFor(() => expect(field.value).toBe(''));
    expect(within(detail).queryByRole('listbox', { name: 'Mention a task' })).toBeNull();
  });

  it('attachments upload into the note draft', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-3');
    const file = new File([new Uint8Array([1, 2, 3, 4])], 'shot.png', { type: 'image/png' });
    fireEvent.change(within(detail).getByLabelText('Attach to note: photos'), { target: { files: [file] } });
    await waitFor(() => expect(sent('/S-3/attachments')).toHaveLength(1));
    expect(sent('/S-3/attachments')[0]!.path).toBe('/api/tasks/scratch-1/S-3/attachments?name=shot.png');
    await waitFor(() => expect((within(detail).getByLabelText('Add a note') as HTMLTextAreaElement).value).toBe('![shot.png](att:S-3/abcd1234-shot.png)'));
  });
});

describe('dependencies + reorder', () => {
  it('links / unlinks; the picker never offers a cycle', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-2');
    fireEvent.click(within(detail).getByLabelText('Add dependency'));
    const sheet = await screen.findByTestId('deps-sheet');
    // S-1 already runs after S-2: picking it would close a loop.
    expect(within(sheet).queryByRole('option', { name: /S-1/ })).toBeNull();
    fireEvent.click(within(sheet).getByRole('option', { name: /S-3/ }));
    await waitFor(() => expect(sent('/S-2/link')).toHaveLength(1));
    expect(sent('/S-2/link')[0]!.json).toEqual({ dep: 'S-3' });
    await waitFor(() => expect(within(sheet).getByRole('option', { name: /S-3/ })).toHaveAttribute('aria-selected', 'true'));
    fireEvent.click(within(sheet).getByRole('option', { name: /S-3/ }));
    await waitFor(() => expect(sent('/S-2/unlink')).toHaveLength(1));
    expect(sent('/S-2/unlink')[0]!.json).toEqual({ dep: 'S-3' });
  });

  it('unlinks from the dependency row', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-1');
    fireEvent.click(within(detail).getByLabelText('Remove dependency S-2'));
    await waitFor(() => expect(sent('/S-1/unlink')).toHaveLength(1));
    expect(sent('/S-1/unlink')[0]!.json).toEqual({ dep: 'S-2' });
  });

  it('card long-press menu: moves, one-tap priority, reorder payloads, duplicate', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.contextMenu(card('S-3'));
    let menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    expect(within(menu).getByRole('menuitem', { name: 'Move to Backlog' })).toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'Cancel task' })).toBeInTheDocument();
    // Lane drawn as [S-2 → S-1 chain], S-3: S-3 is last → down/bottom disabled.
    expect(within(menu).getByRole('menuitem', { name: 'Move down' })).toBeDisabled();
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Move up' }));
    await waitFor(() => expect(sent('/S-3/order')).toHaveLength(1));
    expect(sent('/S-3/order')[0]!.json).toEqual({ before: 'S-2' });

    fireEvent.contextMenu(card('S-2'));
    menu = await screen.findByRole('menu', { name: 'S-2 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Move to bottom' }));
    await waitFor(() => expect(sent('/S-1/order')).toHaveLength(1));
    // The chain moves as a block: S-2 to the bottom, S-1 right after it.
    expect(sent('/S-2/order')[0]!.json).toEqual({ bottom: true });
    expect(sent('/S-1/order')[0]!.json).toEqual({ after_pos: 'S-2' });

    fireEvent.contextMenu(card('S-3'));
    menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitemradio', { name: 'Priority: Urgent' }));
    await waitFor(() => expect(sent('/S-3/edit')).toHaveLength(1));
    expect(sent('/S-3/edit')[0]!.json).toEqual({ priority: 'urgent' });

    fireEvent.contextMenu(card('S-3'));
    menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Duplicate' }));
    await waitFor(() => expect(sent('/S-3/duplicate')).toHaveLength(1));
    expect(await screen.findByText('Duplicated S-3 as S-99')).toBeInTheDocument();
  });

  it('a final-lane move from the card menu asks first; Keep it sends nothing', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.contextMenu(card('S-3'));
    let menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Cancel task' }));
    let confirm = await screen.findByTestId('confirm-sheet');
    expect(within(confirm).getByText('Cancelled is final for a task — afterwards it can only be archived.')).toBeInTheDocument();
    fireEvent.click(within(confirm).getByRole('button', { name: 'Keep it' }));
    await waitFor(() => expect(screen.queryByTestId('confirm-sheet')).toBeNull());
    expect(sent('/S-3/move')).toHaveLength(0);

    fireEvent.contextMenu(card('S-3'));
    menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Cancel task' }));
    confirm = await screen.findByTestId('confirm-sheet');
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel task' }));
    await waitFor(() => expect(sent('/S-3/move')).toHaveLength(1));
    expect(sent('/S-3/move')[0]!.json).toEqual({ status: 'cancelled' });
    expect(await screen.findByRole('button', { name: 'Undo' })).toBeInTheDocument();
  });

  it('a running card lists no moves', async () => {
    setup({
      'GET /api/projects/scratch-1/tasks': () => ({
        json: { tasks: [task({ id: 'S-2', title: 'Token cache', status: 'in_progress', staleness: 'running', run: { session: 'live', pid: 1 } })], problems: [] },
      }),
    });
    await waitFor(() => expect(document.querySelector('[data-id="S-2"]')).not.toBeNull());
    fireEvent.contextMenu(card('S-2'));
    const menu = await screen.findByRole('menu', { name: 'S-2 actions' });
    expect(within(menu).getByText(/Claimed by session live/)).toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /Release/ })).toBeNull();
    expect(within(menu).queryByRole('menuitem', { name: 'Cancel task' })).toBeNull();
  });
});

describe('lane + project actions', () => {
  it('Done all asks first, then moves the lane', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(pill('in_review'));
    fireEvent.click(screen.getByRole('button', { name: /Done all/ }));
    const sheet = await screen.findByTestId('confirm-sheet');
    expect(sent('/done-lane')).toHaveLength(0);
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move 1 to Done' }));
    await waitFor(() => expect(sent('/done-lane')).toHaveLength(1));
    expect(await screen.findByText('Moved 1 to Done')).toBeInTheDocument();
  });

  it('Archive all from the Done lane menu', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(pill('done'));
    fireEvent.click(screen.getByLabelText('Done options'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /without summary/ }));
    fireEvent.click(within(await screen.findByTestId('confirm-sheet')).getByRole('button', { name: 'Archive 1 task' }));
    await waitFor(() => expect(sent('/archive-lane')).toHaveLength(1));
    expect(sent('/archive-lane')[0]!.json).toEqual({ status: 'done' });
  });

  it('Summarize & archive: instruction from settings → generate → save & archive', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(pill('done'));
    fireEvent.click(screen.getByRole('button', { name: /Summarize/ }));
    const sheet = await screen.findByTestId('summarize-sheet');
    await waitFor(() => expect(within(sheet).getByLabelText('Summary instruction')).toHaveValue('Write a changelog.'));
    fireEvent.click(within(sheet).getByRole('button', { name: /Generate summary/ }));
    await waitFor(() => expect(within(sheet).getByRole('button', { name: 'Save & archive 1 task' })).toBeInTheDocument());
    expect(sent('/summarize')[0]!.json).toEqual({ instruction: 'Write a changelog.' });
    fireEvent.click(within(sheet).getByRole('button', { name: 'Resummarize with note' }));
    fireEvent.input(within(sheet).getByLabelText('Note for the resummary'), { target: { value: 'shorter' } });
    fireEvent.click(within(sheet).getByRole('button', { name: /^Resummarize$/ }));
    await waitFor(() => expect(sent('/summarize')).toHaveLength(2));
    expect(sent('/summarize')[1]!.json).toEqual({ instruction: 'Write a changelog.', previous: '- feat: shipped relay (S-7)', note: 'shorter' });
    await waitFor(() => expect(within(sheet).getByRole('button', { name: 'Save & archive 1 task' })).not.toBeDisabled());
    fireEvent.click(within(sheet).getByRole('button', { name: 'Save & archive 1 task' }));
    await waitFor(() => expect(sent('/archive-summary')).toHaveLength(1));
    expect(sent('/archive-summary')[0]!.json).toEqual({ markdown: '- feat: shipped relay (S-7)', taskIds: ['S-7'] });
    expect(await screen.findByText('Archived 1 task · summary saved')).toBeInTheDocument();
  });

  it('Summarize board: scope board + prompt; needsAgent shows the configure banner', async () => {
    let first = true;
    const { sent } = setup({
      'POST /api/projects/scratch-1/summarize': () => {
        if (first) {
          first = false;
          return { status: 409, json: { error: 'no agent', needsAgent: true } };
        }
        return { json: { summary: 'All calm.', taskIds: [] } };
      },
    });
    await ready();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Summarize board' }));
    const sheet = await screen.findByTestId('summarize-sheet');
    fireEvent.input(within(sheet).getByLabelText('Summary prompt'), { target: { value: 'todo only' } });
    fireEvent.click(within(sheet).getByRole('button', { name: /Generate summary/ }));
    expect(await within(sheet).findByText('No agent configured')).toBeInTheDocument();
    fireEvent.click(within(sheet).getByRole('button', { name: /Generate summary/ }));
    await waitFor(() => expect(within(sheet).getByText('All calm.')).toBeInTheDocument());
    expect(sent('/summarize')[1]!.json).toEqual({ scope: 'board', instruction: 'todo only' });
  });

  it('Board settings: edits a limit and saves the flat patch', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Board settings' }));
    const sheet = await screen.findByTestId('board-settings-sheet');
    await waitFor(() => expect(within(sheet).getByLabelText('Summary model: pi default')).toBeInTheDocument());
    fireEvent.click(within(sheet).getByLabelText('Summary model: pi default'));
    fireEvent.click(await within(sheet).findByRole('option', { name: 'claude-x' }));
    fireEvent.click(within(sheet).getByRole('tab', { name: 'Sessions' }));
    fireEvent.click(within(sheet).getByLabelText('Increase Sessions at once'));
    fireEvent.click(within(sheet).getByRole('radio', { name: 'Done' }));
    fireEvent.click(within(sheet).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(sent('/api/settings').filter((r) => r.method === 'PUT')).toHaveLength(1));
    expect(sent('/api/settings').find((r) => r.method === 'PUT')!.json).toEqual({
      summaryModel: 'anthropic/claude-x',
      summaryInstruction: '',
      maxSessions: 3,
      turnAddLimit: 20,
      chainGate: 'done',
      archiveAfterDays: 0,
      retentionDays: 90,
    });
    expect(await screen.findByText('Settings saved')).toBeInTheDocument();
  });

  it('Archive project, then Undo unarchives it', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Archive project' }));
    await waitFor(() => expect(sent('/api/projects/scratch-1').filter((r) => r.method === 'PUT')).toHaveLength(1));
    expect(sent('/api/projects/scratch-1').find((r) => r.method === 'PUT')!.json).toEqual({ archived: true });
    fireEvent.click(await screen.findByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(sent('/api/projects/scratch-1').filter((r) => r.method === 'PUT')).toHaveLength(2));
    expect(sent('/api/projects/scratch-1').filter((r) => r.method === 'PUT')[1]!.json).toEqual({ archived: false });
  });

  it('UNI-134: long-press a project in the picker opens its context menu, Archive acts on that project (not the open one)', async () => {
    const { sent } = setup({
      'GET /api/projects': () => ({
        json: [
          project({ slug: 'scratch-1', name: 'scratch', updatedAt: '2026-10-07T00:00:00Z' }),
          project({ slug: 'other', name: 'other project', updatedAt: '2026-10-07T00:00:00Z' }),
        ],
      }),
      'PUT /api/projects/other': (req) => ({ json: project({ slug: 'other', name: 'other project', archived: (req.json as { archived: boolean }).archived }) }),
    });
    await ready();
    fireEvent.click(screen.getByLabelText('Project: scratch'));
    const sheet = await screen.findByTestId('project-sheet');
    const row = within(sheet).getByText('other project').closest('button')!;
    fireEvent.contextMenu(row);
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Archive' }));
    await waitFor(() => expect(sent('/api/projects/other').filter((r) => r.method === 'PUT')).toHaveLength(1));
    expect(sent('/api/projects/other').find((r) => r.method === 'PUT')!.json).toEqual({ archived: true });
    // The open project (scratch-1) wasn't touched.
    expect(sent('/api/projects/scratch-1').filter((r) => r.method === 'PUT')).toHaveLength(0);
  });
});

describe('live updates (UNI-132)', () => {
  it('a revision refreshes the open task without clobbering the note / description drafts', async () => {
    const { fake, update } = setup();
    await ready();
    await waitFor(() => expect(fake.open().map((s) => s.project)).toEqual(['scratch-1']));
    const stream = fake.open()[0]!;
    stream.state('open');
    const detail = await openDetail('S-2');
    const note = within(detail).getByLabelText('Add a note') as HTMLTextAreaElement;
    fireEvent.input(note, { target: { value: 'half-written note' } });
    fireEvent.click(within(detail).getByLabelText('Edit description'));
    const editor = await screen.findByTestId('body-editor');
    const body = within(editor).getByLabelText('Description (markdown)') as HTMLTextAreaElement;
    fireEvent.input(body, { target: { value: 'my draft body' } });

    // Someone else (CLI / agent) changes the task: the daemon bumps the revision.
    update('S-2', { title: 'Token cache (live)', body: 'server body', priority: 'urgent' });
    stream.revision('2');
    stream.revision('3');
    await waitFor(() => expect((within(detail).getByLabelText('Task title') as HTMLTextAreaElement).value).toBe('Token cache (live)'));
    expect(within(detail).getByLabelText('Priority: Urgent')).toBeInTheDocument();
    expect(note.value).toBe('half-written note');
    expect(body.value).toBe('my draft body');
    expect(screen.getByTestId('body-editor')).toBeInTheDocument();
  });

  it('the ⋯ menu shows the live state; a dropped stream shows reconnecting', async () => {
    const { fake } = setup();
    await ready();
    await waitFor(() => expect(fake.open()).toHaveLength(1));
    fake.open()[0]!.state('open');
    fireEvent.click(screen.getByLabelText('Board menu'));
    expect(await screen.findByText(/Live updates: on/)).toBeInTheDocument();
    expect(screen.getByLabelText('PC: coffee · live')).toBeInTheDocument();
    fake.open()[0]!.state('closed', 'The PC closed the board stream');
    expect(await screen.findByText(/Live updates: reconnecting/)).toBeInTheDocument();
    expect(document.querySelector('.pc-dot')!.getAttribute('data-live')).toBe('reconnecting');
  });
});
