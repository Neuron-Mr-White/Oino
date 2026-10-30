// @vitest-environment jsdom
/**
 * Task detail + every task / project action against the CLI-shaped board
 * (v2, WP-A22): exact argv composed by `lib/board/api.ts`, the comment /
 * confirm flows, note / dependency / order / duplicate writes, lane and
 * project actions, live `board_changed` refreshes. Summaries and the board
 * settings sheet are gone with the daemon (no CLI surface).
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Board from './Board';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setBoardTransport } from '../lib/board/transport';
import { setHostClientFactory } from '../lib/host/factory';
import { fakeBoard, task, type CliHandler } from '../lib/board/testing';
import { createMockBoard, type MockBoardProject } from '../dev/mock/board';
import { resetBoardStore } from '../lib/board/store';
import { resetBoardUi } from '../lib/board/ui';
import { resetPcAvailability } from '../lib/board/pcs';
import { resetBoardActions } from '../lib/board/actions';
import type { Task } from '../lib/board/api';
import type { AppError } from '@unipi/protocol';
import type { Closable, HostClient, HubEvent } from '../lib/host/client';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const STALE_HINT = 'why the stale run is being released';
const hosts = [{ id: 'pc1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

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

function boardProjects(): MockBoardProject[] {
  return [
    { slug: 'scratch-1', name: 'scratch', prefix: 'S', tasks: seed() },
    { slug: 'other', name: 'other project', prefix: 'O', tasks: [] },
  ];
}

/** A fake host: the live stream (`board_changed`) + the attachment staging ops. */
type FakeHost = {
  client: HostClient;
  emit: (event: HubEvent) => void;
  drop: (message?: string) => void;
  requests: Array<{ op: string; args?: Record<string, unknown> }>;
};

function fakeHost(): FakeHost {
  const sinks = new Set<(event: HubEvent) => void>();
  const closers = new Set<(error?: AppError) => void>();
  const requests: Array<{ op: string; args?: Record<string, unknown> }> = [];
  const client: HostClient = {
    hostId: 'pc1',
    conn: () => ({ state: 'connected' }),
    retry: () => Promise.resolve(),
    request: async <T,>(op: string, args?: Record<string, unknown>) => {
      requests.push({ op, args });
      if (op === 'board_watch') return { watching: args?.on !== false } as T;
      if (op === 'blob_put_bytes') return { sha256: 'mock-sha' } as T;
      return null as T;
    },
    openEvents: (sink: (event: HubEvent) => void, closed?: (error?: AppError) => void) => {
      sinks.add(sink);
      if (closed) closers.add(closed);
      const closable: Closable = {
        close: () => {
          sinks.delete(sink);
          closers.delete(closed ?? (() => {}));
        },
      };
      return closable;
    },
    openSession: () => {
      throw new Error('not needed');
    },
    openTerm: () => {
      throw new Error('not needed');
    },
    openLsp: () => {
      throw new Error('not needed');
    },
  };
  return {
    client,
    requests,
    emit: (event) => sinks.forEach((sink) => sink(event)),
    drop: (message) =>
      closers.forEach((closed) => closed(message ? ({ code: 'E_TRANSPORT', message, retryable: true } as AppError) : undefined)),
  };
}

function setup(overrides: Record<string, CliHandler> = {}) {
  const board = createMockBoard(boardProjects());
  const fake = fakeBoard({ handlers: overrides, fallback: ({ args, project }) => board.cli(args, project) });
  setBoardTransport(fake.transport);
  const host = fakeHost();
  setHostClientFactory(() => host.client);
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts') return { ok: true, value: hosts };
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
  /** Calls whose argv starts with `prefix` (e.g. `move S-6`). */
  const sent = (prefix: string) => fake.calls.filter((item) => item.args.join(' ').startsWith(prefix));
  return { fake, board, host, sent };
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
  it('moves into a final lane with no undo (the CLI contract has no undo token)', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-6', 'in_review');
    fireEvent.click(within(detail).getByRole('button', { name: /Mark done/ }));
    await waitFor(() => expect(sent('move S-6')).toHaveLength(1));
    expect(sent('move S-6')[0]!.args).toEqual(['move', 'S-6', 'done']);
    await screen.findByText('Moved S-6 to Done');
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
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
    await waitFor(() => expect(sent('move S-4')).toHaveLength(1));
    expect(sent('move S-4')[0]!.args).toEqual(['move', 'S-4', 'todo', '--comment', 'session died']);
    await waitFor(() => expect(screen.queryByTestId('comment-sheet')).toBeNull());
  });

  it('a rule refusal that needs a comment opens the comment sheet and retries with the note', async () => {
    let calls = 0;
    const { sent } = setup({
      'move S-5': () => {
        calls += 1;
        if (calls === 1) return { ok: false, error: 'blocked → done requires --comment (why)', kind: 'rule' };
        return { ok: true, payload: task({ id: 'S-5', title: 'Upgrade iroh', status: 'done' }) };
      },
    });
    await ready();
    const detail = await openDetail('S-5', 'blocked');
    fireEvent.click(within(detail).getByLabelText('Status: Blocked'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Move to Done' }));
    // A menu move into a final lane asks first …
    const confirm = await screen.findByTestId('confirm-sheet');
    expect(sent('move S-5')).toHaveLength(0);
    fireEvent.click(within(confirm).getByRole('button', { name: 'Move to Done' }));
    // … then the refusal swaps in the comment sheet.
    const sheet = await screen.findByTestId('comment-sheet');
    fireEvent.input(within(sheet).getByLabelText('Note'), { target: { value: 'decided: pin 0.95' } });
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move to Done' }));
    await waitFor(() => expect(calls).toBe(2));
    expect(sent('move S-5').map((call) => call.args)).toEqual([
      ['move', 'S-5', 'done'],
      ['move', 'S-5', 'done', '--comment', 'decided: pin 0.95'],
    ]);
  });

  it('rework from In Review takes an optional note', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-6', 'in_review');
    fireEvent.click(within(detail).getByRole('button', { name: /Rework/ }));
    const sheet = await screen.findByTestId('comment-sheet');
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move without note' }));
    await waitFor(() => expect(sent('move S-6')).toHaveLength(1));
    expect(sent('move S-6')[0]!.args).toEqual(['move', 'S-6', 'todo']);
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
    await waitFor(() => expect(sent('edit S-2')).toHaveLength(1));
    expect(sent('edit S-2')[0]!.args).toEqual(['edit', 'S-2', '--title=Token cache v2']);

    fireEvent.click(within(detail).getByLabelText('Priority: Medium'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Urgent/ }));
    await waitFor(() => expect(sent('edit S-2')).toHaveLength(2));
    expect(sent('edit S-2')[1]!.args).toEqual(['edit', 'S-2', '--priority', 'urgent']);

    fireEvent.click(within(detail).getByLabelText('Edit labels'));
    const labels = await screen.findByTestId('labels-sheet');
    fireEvent.click(within(labels).getByRole('option', { name: /ui/ }));
    await waitFor(() => expect(sent('edit S-2')).toHaveLength(3));
    expect(sent('edit S-2')[2]!.args).toEqual(['edit', 'S-2', '--labels=auth,ui', '--new-label']);
    fireEvent.click(within(labels).getByRole('button', { name: 'Done' }));

    fireEvent.click(within(detail).getByLabelText('Edit description'));
    const editor = await screen.findByTestId('body-editor');
    fireEvent.input(within(editor).getByLabelText('Description (markdown)'), { target: { value: 'Cache it.' } });
    fireEvent.click(within(editor).getByLabelText('Bold'));
    fireEvent.click(within(editor).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(sent('edit S-2')).toHaveLength(4));
    // Bold with no selection inserts a placeholder at the caret (end of the text).
    expect(sent('edit S-2')[3]!.args).toEqual(['edit', 'S-2', '--body=Cache it.**bold**']);
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
    await waitFor(() => expect(sent('note S-5')).toHaveLength(1));
    expect(sent('note S-5')[0]!.args).toEqual(['note', 'S-5', '--', 'pin it, see S-3']);
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
    await waitFor(() => expect(sent('note S-5')).toHaveLength(1));
    await waitFor(() => expect(field.value).toBe(''));
    expect(within(detail).queryByRole('listbox', { name: 'Mention a task' })).toBeNull();
  });

  it('attachments upload into the note draft (host blob + CLI attach)', async () => {
    const { sent, host } = setup();
    await ready();
    const detail = await openDetail('S-3');
    const file = new File([new Uint8Array([1, 2, 3, 4])], 'shot.png', { type: 'image/png' });
    fireEvent.change(within(detail).getByLabelText('Attach to note: photos'), { target: { files: [file] } });
    const attach = await waitFor(() => {
      const call = sent('attach S-3').at(0);
      expect(call).toBeDefined();
      return call!;
    });
    expect(attach.args[0]).toBe('attach');
    expect(attach.args[1]).toBe('S-3');
    expect(attach.args[2]).toMatch(/^~\/.oino\/uploads\/board-[^/]+-shot\.png$/);
    expect(attach.args.slice(3)).toEqual(['--name', 'shot.png']);
    // The bytes crossed the host seam (`blob_put_bytes` → `fs_import`).
    expect(host.requests.map((request) => request.op)).toEqual(expect.arrayContaining(['blob_put_bytes', 'fs_import']));
    await waitFor(() =>
      expect((within(detail).getByLabelText('Add a note') as HTMLTextAreaElement).value).toMatch(/^!\[shot\.png\]\(att:S-3\/aaaa\d+-shot\.png\)$/),
    );
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
    await waitFor(() => expect(sent('link S-2')).toHaveLength(1));
    expect(sent('link S-2')[0]!.args).toEqual(['link', 'S-2', '--after', 'S-3']);
    await waitFor(() => expect(within(sheet).getByRole('option', { name: /S-3/ })).toHaveAttribute('aria-selected', 'true'));
    fireEvent.click(within(sheet).getByRole('option', { name: /S-3/ }));
    await waitFor(() => expect(sent('unlink S-2')).toHaveLength(1));
    expect(sent('unlink S-2')[0]!.args).toEqual(['unlink', 'S-2', '--after', 'S-3']);
  });

  it('unlinks from the dependency row', async () => {
    const { sent } = setup();
    await ready();
    const detail = await openDetail('S-1');
    fireEvent.click(within(detail).getByLabelText('Remove dependency S-2'));
    await waitFor(() => expect(sent('unlink S-1')).toHaveLength(1));
    expect(sent('unlink S-1')[0]!.args).toEqual(['unlink', 'S-1', '--after', 'S-2']);
  });

  it('card long-press menu: moves, one-tap priority, reorder payloads, duplicate', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.contextMenu(card('S-3'));
    let menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    expect(within(menu).getByRole('menuitem', { name: 'Move to Backlog' })).toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'Cancel task' })).toBeInTheDocument();
    // Lane drawn as [S-1 → S-2], S-3 last: nothing below → down/bottom disabled.
    expect(within(menu).getByRole('menuitem', { name: 'Move down' })).toBeDisabled();
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Move up' }));
    await waitFor(() => expect(sent('order S-3')).toHaveLength(1));
    expect(sent('order S-3')[0]!.args).toEqual(['order', 'S-3', '--before', 'S-2']);

    fireEvent.contextMenu(card('S-2'));
    menu = await screen.findByRole('menu', { name: 'S-2 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Move to bottom' }));
    await waitFor(() => expect(sent('order S-2')).toHaveLength(1));
    // The chain (S-2 → S-1) moves as a block: S-2 to the bottom, S-1 right after it.
    expect(sent('order S-2')[0]!.args).toEqual(['order', 'S-2', '--bottom']);
    await waitFor(() => expect(sent('order S-1')).toHaveLength(1));
    expect(sent('order S-1')[0]!.args).toEqual(['order', 'S-1', '--after-pos', 'S-2']);

    fireEvent.contextMenu(card('S-3'));
    menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitemradio', { name: 'Priority: Urgent' }));
    await waitFor(() => expect(sent('edit S-3')).toHaveLength(1));
    expect(sent('edit S-3')[0]!.args).toEqual(['edit', 'S-3', '--priority', 'urgent']);

    fireEvent.contextMenu(card('S-3'));
    menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Duplicate' }));
    await waitFor(() => expect(sent('duplicate S-3')).toHaveLength(1));
    expect(await screen.findByText(/^Duplicated S-3 as S-\d+$/)).toBeInTheDocument();
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
    expect(sent('move S-3')).toHaveLength(0);

    fireEvent.contextMenu(card('S-3'));
    menu = await screen.findByRole('menu', { name: 'S-3 actions' });
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Cancel task' }));
    confirm = await screen.findByTestId('confirm-sheet');
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel task' }));
    await waitFor(() => expect(sent('move S-3')).toHaveLength(1));
    expect(sent('move S-3')[0]!.args).toEqual(['move', 'S-3', 'cancelled']);
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
  });

  it('a running card lists no moves', async () => {
    const { sent } = setup({
      'list --all': () => ({
        ok: true,
        payload: { tasks: [task({ id: 'S-2', title: 'Token cache', status: 'in_progress', staleness: 'running', run: { session: 'live', pid: 1 } })], problems: [] },
      }),
    });
    await waitFor(() => expect(document.querySelector('[data-id="S-2"]')).not.toBeNull());
    fireEvent.contextMenu(card('S-2'));
    const menu = await screen.findByRole('menu', { name: 'S-2 actions' });
    expect(within(menu).getByText(/Claimed by session live/)).toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /Release/ })).toBeNull();
    expect(within(menu).queryByRole('menuitem', { name: 'Cancel task' })).toBeNull();
    expect(sent('move S-2')).toHaveLength(0);
  });
});

describe('lane + project actions', () => {
  it('Done all asks first, then moves the lane (one CLI move per task)', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(pill('in_review'));
    fireEvent.click(screen.getByRole('button', { name: /Done all/ }));
    const sheet = await screen.findByTestId('confirm-sheet');
    expect(sent('move S-6')).toHaveLength(0);
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move 1 to Done' }));
    await waitFor(() => expect(sent('move S-6')).toHaveLength(1));
    expect(sent('move S-6')[0]!.args).toEqual(['move', 'S-6', 'done']);
    expect(await screen.findByText('Moved 1 to Done')).toBeInTheDocument();
  });

  it('Archive all from the Done lane menu archives each task', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(pill('done'));
    fireEvent.click(screen.getByLabelText('Done options'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Archive all \(1\)/ }));
    fireEvent.click(within(await screen.findByTestId('confirm-sheet')).getByRole('button', { name: 'Archive 1 task' }));
    await waitFor(() => expect(sent('move S-7')).toHaveLength(1));
    expect(sent('move S-7')[0]!.args).toEqual(['move', 'S-7', 'archived']);
    expect(await screen.findByText('Archived 1 task')).toBeInTheDocument();
  });

  it('Archive project, then Unarchive project from the same menu', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Archive project' }));
    await waitFor(() => expect(sent('project archive')).toHaveLength(1));
    expect(sent('project archive')[0]!.args).toEqual(['project', 'archive', 'scratch-1']);
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Unarchive project' }));
    await waitFor(() => expect(sent('project unarchive')).toHaveLength(1));
    expect(sent('project unarchive')[0]!.args).toEqual(['project', 'unarchive', 'scratch-1']);
  });

  it('UNI-134: long-press a project in the picker opens its context menu, Archive acts on that project (not the open one)', async () => {
    const { sent } = setup();
    await ready();
    fireEvent.click(screen.getByLabelText('Project: scratch'));
    const sheet = await screen.findByTestId('project-sheet');
    const row = within(sheet).getByText('other project').closest('button')!;
    fireEvent.contextMenu(row);
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Archive' }));
    await waitFor(() => expect(sent('project archive')).toHaveLength(1));
    expect(sent('project archive')[0]!.args).toEqual(['project', 'archive', 'other']);
    // The open project (scratch-1) wasn't touched.
    expect(sent('project archive scratch-1')).toHaveLength(0);
  });
});

describe('live updates (UNI-132)', () => {
  it('a board_changed event refreshes the open task without clobbering the note / description drafts', async () => {
    const { board, host } = setup();
    await ready();
    await waitFor(() => expect(host.requests.some((request) => request.op === 'board_watch')).toBe(true));
    const detail = await openDetail('S-2');
    const note = within(detail).getByLabelText('Add a note') as HTMLTextAreaElement;
    fireEvent.input(note, { target: { value: 'half-written note' } });
    fireEvent.click(within(detail).getByLabelText('Edit description'));
    const editor = await screen.findByTestId('body-editor');
    const body = within(editor).getByLabelText('Description (markdown)') as HTMLTextAreaElement;
    fireEvent.input(body, { target: { value: 'my draft body' } });

    // Someone else (the CLI, an agent) changes the task: the watcher answers board_changed.
    const live = board.tasks('scratch-1').find((item) => item.id === 'S-2')!;
    live.title = 'Token cache (live)';
    live.priority = 'urgent';
    host.emit({ kind: 'board_changed', project: 'scratch-1' });
    await waitFor(
      () => expect((within(detail).getByLabelText('Task title') as HTMLTextAreaElement).value).toBe('Token cache (live)'),
      { timeout: 3000 },
    );
    expect(within(detail).getByLabelText('Priority: Urgent')).toBeInTheDocument();
    expect(note.value).toBe('half-written note');
    expect(body.value).toBe('my draft body');
    expect(screen.getByTestId('body-editor')).toBeInTheDocument();
  });

  it('the ⋯ menu shows the live state; a dropped stream shows reconnecting', async () => {
    const { host } = setup();
    await ready();
    await waitFor(() => expect(host.requests.some((request) => request.op === 'board_watch' && request.args?.on === true)).toBe(true));
    fireEvent.click(screen.getByLabelText('Board menu'));
    expect(await screen.findByText(/Live updates: on/)).toBeInTheDocument();
    expect(screen.getByLabelText(/^PC: coffee · live/)).toBeInTheDocument();
    host.drop('The PC closed the board stream');
    expect(await screen.findByText(/Live updates: reconnecting/)).toBeInTheDocument();
    expect(document.querySelector('.pc-dot')!.getAttribute('data-live')).toBe('reconnecting');
  });
});
