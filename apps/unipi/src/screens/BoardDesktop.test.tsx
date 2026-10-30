// @vitest-environment jsdom
/**
 * UNI-117 kanboard migration, v2 (WP-A22): the Board as the desktop shell's
 * web UI — drag & drop between columns + reorder, keyboard shortcuts, the
 * Overview (dashboard), web-mode hiding of chat-only actions. Every call goes
 * through the CLI seam (`board_cli`, ext-abi §5.5): the assertions read the
 * argv `lib/board/api.ts` composed. Phone behaviour is covered by Board.test /
 * BoardActions.test.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Board from './Board';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setBoardTransport } from '../lib/board/transport';
import { fakeBoard, task, type CliHandler } from '../lib/board/testing';
import { createMockBoard, type MockBoardProject } from '../dev/mock/board';
import { resetBoardStore } from '../lib/board/store';
import { resetBoardUi } from '../lib/board/ui';
import { resetPcAvailability } from '../lib/board/pcs';
import { resetBoardActions } from '../lib/board/actions';
import { resetBoardDrag } from '../lib/board/dnd';
import { cancelCardDrag } from '../components/board/cardDrag';
import { resetLayout } from '../lib/layout';
import { setPlatformOverride } from '../lib/platform';
import type { Task } from '../lib/board/api';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [{ id: 'pc1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

function seed(): Task[] {
  return [
    task({ id: 'S-1', title: 'Offline login', status: 'todo', order: 1000, labels: ['auth'] }),
    task({ id: 'S-2', title: 'Token cache', status: 'todo', order: 2000, labels: ['auth'] }),
    task({ id: 'S-3', title: 'Dark mode polish', status: 'todo', order: 3000, labels: ['ui'] }),
    task({ id: 'S-4', title: 'Crash on rotate', status: 'in_progress', run: { session: 's1', started: new Date().toISOString() } }),
    task({
      id: 'S-6',
      title: 'Pairing polish',
      status: 'in_review',
      statusReport: { kind: 'review', text: '## Summary\nQR fixed', actor: 'agent', at: new Date(Date.now() - 3 * 3600_000).toISOString() },
      activity: [{ at: new Date(Date.now() - 3 * 3600_000).toISOString(), actor: 'agent', text: 'finished: QR fixed' }],
    }),
    task({ id: 'S-7', title: 'Ship relay', status: 'done' }),
  ];
}

function boardProjects(): MockBoardProject[] {
  return [{ slug: 'scratch-1', name: 'scratch', prefix: 'S', tasks: seed() }];
}

function setup(overrides: Record<string, CliHandler> = {}) {
  const board = createMockBoard(boardProjects());
  const fake = fakeBoard({ handlers: overrides, fallback: ({ args, project }) => board.cli(args, project) });
  setBoardTransport(fake.transport);
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
  return { fake, board };
}

function desktop(on: boolean) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: on ? 1440 : 412 });
  window.matchMedia = ((query: string) => ({ matches: on, media: query, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia;
  resetLayout();
}

beforeEach(() => {
  localStorage.clear();
  setPlatformOverride('web');
  desktop(true);
  resetBoardStore();
  resetBoardUi();
  resetPcAvailability();
  resetBoardActions();
  resetBoardDrag();
});
afterEach(() => {
  cancelCardDrag();
  cleanup();
  setBoardTransport(undefined);
  setPlatformOverride(undefined);
  desktop(false);
  delete (window as { matchMedia?: unknown }).matchMedia;
  resetLayout();
  vi.resetAllMocks();
  location.hash = '#/';
});

const card = (id: string) => document.querySelector<HTMLElement>(`.bcard[data-id="${id}"]`)!;
const column = (lane: string) => document.querySelector<HTMLElement>(`[data-lane-page=${lane}]`)!;
async function ready() {
  await waitFor(() => expect(card('S-2')).toBeTruthy());
}
/** Pointer drag (cardDrag.ts): press on the card, move past the threshold over `lane`, optionally release. */
function press(id: string) {
  fireEvent.pointerDown(card(id), { button: 0, clientX: 10, clientY: 10, pointerId: 1 });
}
function moveOver(lane: string, y = 9999) {
  fireEvent.pointerMove(column(lane), { clientX: 40, clientY: y, pointerId: 1 });
}
function release(lane: string, y = 9999) {
  fireEvent.pointerUp(column(lane), { clientX: 40, clientY: y, pointerId: 1 });
}
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
/** A human drag: the lanes re-render between phases (Backlog appears), so flush. */
async function drag(id: string, lane: string, y = 9999) {
  press(id);
  await flush();
  moveOver(lane, y);
  await flush();
  release(lane, y);
}

describe('desktop / web board: drag & drop', () => {
  it('cards are draggable on the desktop shell, except running and final ones', async () => {
    setup();
    await ready();
    expect(card('S-1')).toHaveAttribute('data-draggable');
    // Running: pressable so the drag can say why it won't move.
    expect(card('S-4')).toHaveAttribute('data-draggable');
    expect(card('S-7')).not.toHaveAttribute('data-draggable');
  });

  it('a press without movement is a click (opens the task), not a drag', async () => {
    setup();
    await ready();
    press('S-1');
    fireEvent.pointerUp(card('S-1'), { clientX: 11, clientY: 11, pointerId: 1 });
    fireEvent.click(card('S-1'));
    await screen.findByTestId('task-detail');
  });

  it('while dragging: refused lanes collapse (data-drop=refused), the hovered lane is ok, a ghost follows, Esc cancels', async () => {
    const { fake } = setup();
    await ready();
    press('S-1');
    moveOver('todo');
    expect(document.querySelector('.bcard-ghost')).toBeTruthy();
    expect(card('S-1')).toHaveClass('dragging');
    expect(column('done')).toHaveAttribute('data-drop', 'refused');
    expect(column('in_review')).toHaveAttribute('data-drop', 'refused');
    expect(column('todo')).toHaveAttribute('data-drop', 'allowed');
    moveOver('cancelled');
    expect(column('cancelled')).toHaveAttribute('data-drop', 'ok');
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(document.querySelector('.bcard-ghost')).toBeNull();
    expect(column('done')).not.toHaveAttribute('data-drop');
    release('cancelled');
    expect(fake.sent('move S-1')).toHaveLength(0);
    // The click after a drag doesn't open the card.
    expect(screen.queryByTestId('task-detail')).toBeNull();
  });

  it('Backlog (hidden by default) shows while a card that may go there is dragged, and takes the drop', async () => {
    const { fake } = setup();
    await ready();
    expect(column('backlog')).toBeNull();
    press('S-2');
    moveOver('todo');
    await waitFor(() => expect(column('backlog')).toBeTruthy());
    expect(column('backlog')).toHaveAttribute('data-drop', 'allowed');
    moveOver('backlog');
    release('backlog');
    await waitFor(() => expect(fake.sent('move S-2')).toHaveLength(1));
    expect(fake.sent('move S-2')[0]!.args).toEqual(['move', 'S-2', 'backlog']);
    await waitFor(() => expect(column('backlog')).toBeNull());
  });

  it('a card that may not go to Backlog does not reveal it', async () => {
    setup();
    await ready();
    press('S-7');
    moveOver('done');
    expect(column('backlog')).toBeNull();
  });

  it('dropping In Review → Done moves it where it was dropped, with no undo (the drop is the confirmation)', async () => {
    const { fake } = setup();
    await ready();
    await drag('S-6', 'done');
    await waitFor(() => expect(fake.sent('move S-6')).toHaveLength(1));
    expect(fake.sent('move S-6')[0]!.args).toEqual(['move', 'S-6', 'done']);
    await screen.findByText('Moved S-6 to Done');
    // The drop position is applied after the move (the CLI move has no undo
    // token, so nothing needs the card's old position to stay valid).
    await waitFor(() => expect(fake.sent('order S-6')).toHaveLength(1));
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull();
  });

  it('a move reorders to where it was dropped', async () => {
    const { fake } = setup();
    await ready();
    await drag('S-6', 'todo');
    await waitFor(() => expect(fake.sent('move S-6')).toHaveLength(1));
    expect(fake.sent('move S-6')[0]!.args).toEqual(['move', 'S-6', 'todo']);
    await waitFor(() => expect(fake.sent('order S-6')).toHaveLength(1));
    expect(fake.sent('order S-6')[0]!.args).toEqual(['order', 'S-6', '--bottom']);
  });

  it('reorders within a lane (bottom) and refuses a disallowed lane with the rule', async () => {
    const { fake } = setup();
    await ready();
    await drag('S-1', 'todo');
    await waitFor(() => expect(fake.sent('order S-1')).toHaveLength(1));
    expect(fake.sent('order S-1')[0]!.args).toEqual(['order', 'S-1', '--bottom']);
    await screen.findByText('Reordered S-1');
    // A refused lane still accepts the drop (dropEffect 'move'): with 'none'
    // a real browser never fires drop and the refusal is silent.
    press('S-2');
    moveOver('done');
    expect(column('done')).toHaveAttribute('data-drop', 'refused');
    release('done');
    await screen.findByText("S-2 can't move from Todo to Done");
    expect(fake.sent('move S-2')).toHaveLength(0);
  });

  it('a running card refuses to be picked up and says why', async () => {
    setup();
    await ready();
    press('S-4');
    moveOver('todo');
    expect(document.querySelector('.bcard-ghost')).toBeNull();
    await screen.findByText(/claimed by session s1/);
  });
});

describe('desktop / web board: layout', () => {
  it('New task is a toolbar button (no FAB over the cards); column heads carry name, count, ⓘ and no lane-pill strip duty', async () => {
    setup();
    await ready();
    expect(document.querySelector('.board-fab')).toBeNull();
    const button = screen.getByRole('button', { name: 'New task in Todo' });
    expect(button).toHaveClass('btn', 'btn-primary');
    const head = column('todo').querySelector('.lane-head-desk')!;
    expect(head.querySelector('.lane-title')!.textContent).toBe('Todo');
    expect(head.querySelector('.lane-count')!.textContent).toBe('3');
    expect(head.querySelector('.lane-info-dot')!.getAttribute('aria-label')).toMatch(/^Ready to work/);
    expect(column('todo').querySelector('.lane-info')).toBeNull();
    fireEvent.click(button);
    await screen.findByRole('dialog', { name: /New task/ });
  });
});

describe('desktop / web board: keyboard', () => {
  it('J/K select, Enter opens, C new task, ? shortcuts, B/L views', async () => {
    setup();
    await ready();
    fireEvent.keyDown(window, { key: 'j' });
    await waitFor(() => expect(card('S-1')).toHaveClass('kb-selected'));
    fireEvent.keyDown(window, { key: 'j' });
    await waitFor(() => expect(card('S-2')).toHaveClass('kb-selected'));
    fireEvent.keyDown(window, { key: 'k' });
    await waitFor(() => expect(card('S-1')).toHaveClass('kb-selected'));
    fireEvent.keyDown(window, { key: 'Enter' });
    await screen.findByTestId('task-detail');
    fireEvent.keyDown(window, { key: 'Escape' });
  });

  it('? opens the shortcuts sheet; L switches to the list', async () => {
    setup();
    await ready();
    fireEvent.keyDown(window, { key: '?' });
    const sheet = await screen.findByTestId('shortcuts-sheet');
    expect(within(sheet).getByText('Next / previous task')).toBeInTheDocument();
  });

  it('L → list, B → board, C → New task sheet', async () => {
    setup();
    await ready();
    fireEvent.keyDown(window, { key: 'l' });
    await screen.findByTestId('board-list');
    fireEvent.keyDown(window, { key: 'b' });
    await waitFor(() => expect(screen.queryByTestId('board-list')).toBeNull());
    fireEvent.keyDown(window, { key: 'c' });
    await waitFor(() => expect(document.querySelector('.new-task-title')).toBeTruthy());
  });

  it('keys do nothing while typing', async () => {
    setup();
    await ready();
    const input = document.createElement('input');
    document.body.appendChild(input);
    fireEvent.keyDown(input, { key: 'l' });
    expect(screen.queryByTestId('board-list')).toBeNull();
    input.remove();
  });
});

describe('Overview (kanboard dashboard)', () => {
  it('opens from the ⋯ menu: greeting, KPIs, inbox, up next; Approve moves to Done', async () => {
    const { fake } = setup();
    await ready();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByText('Overview of all projects'));
    const dash = await screen.findByTestId('dashboard');
    await waitFor(() => expect(within(dash).getByLabelText('Approve S-6')).toBeInTheDocument());
    expect(within(dash).getByText('Waiting on you')).toBeInTheDocument();
    expect(within(dash).getByText(/1 review waiting/)).toBeInTheDocument();
    expect(within(dash).getByText('QR fixed', { selector: '.dash-row-excerpt' })).toBeInTheDocument();
    expect(screen.queryByLabelText(/New task in/)).toBeNull();
    fireEvent.click(within(dash).getByLabelText('Approve S-6'));
    await waitFor(() => expect(fake.sent('move S-6')).toHaveLength(1));
    expect(fake.sent('move S-6')[0]!.args).toEqual(['move', 'S-6', 'done']);
    await screen.findByText('Moved S-6 to Done');
  });

  it('a row opens the task in its project and leaves the Overview', async () => {
    setup();
    await ready();
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByText('Overview of all projects'));
    const dash = await screen.findByTestId('dashboard');
    await waitFor(() => expect(within(dash).getByText('Offline login')).toBeInTheDocument());
    fireEvent.click(within(dash).getByText('Offline login'));
    await screen.findByTestId('task-detail');
    expect(screen.queryByTestId('dashboard')).toBeNull();
  });

  it('shows an error with Retry when the CLI refuses `project list`', async () => {
    const { fake } = setup();
    await ready();
    fake.handlers['project list'] = () => ({ ok: false, error: 'board home is unreadable', kind: 'io' });
    fireEvent.click(screen.getByLabelText('Board menu'));
    fireEvent.click(await screen.findByText('Overview of all projects'));
    await screen.findByText(/Couldn't load the overview/);
    expect(screen.getByText(/board home is unreadable/)).toBeInTheDocument();
  });
});

describe('web mode', () => {
  it('hides chat-only actions (Go to a chat)', async () => {
    setup();
    await ready();
    expect(screen.queryByLabelText('Go to a chat')).toBeNull();
  });
});
