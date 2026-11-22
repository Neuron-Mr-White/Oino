// @vitest-environment jsdom
// Workbench: the Review queue's filter by priority and label, its sort, and that the choices are kept per host.
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentReview } from '../../dev/mock/scenarios/superagent-review';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { boardUi, resetBoardUi } from '../../lib/board/ui';
import { boardStore, resetBoardStore } from '../../lib/board/store';
import { endQueue } from '../../lib/board/taskOrigin';
import { resetPendingMoves } from '../../lib/board/pendingMoves';
import { toasts } from '../../lib/toasts';
import { readWorkbenchView, resetWorkbenchState } from '../../lib/superagent/workbenchState';
import { filterQueue, sortQueue } from '../../lib/superagent/reviewQueue';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));

const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
/** Host order: HP-3 HP-5 HP-8 OIN-12 OIN-17 NOT-2 NOT-4 (the board's project order). */
const PRIORITY: Record<string, string> = { 'HP-3': 'low', 'HP-5': 'urgent', 'HP-8': 'high', 'OIN-12': 'medium', 'OIN-17': 'urgent', 'NOT-2': 'none', 'NOT-4': 'low' };
/** Minutes since each ticket last changed: HP-8 waits longest, NOT-4 changed most recently. */
const WAITING: Record<string, number> = { 'HP-3': 50, 'HP-5': 40, 'HP-8': 900, 'OIN-12': 30, 'OIN-17': 20, 'NOT-2': 60, 'NOT-4': 5 };
const REPORT = '## Findings\n\n- first point\n- second point\n\nCall `retryOnce()` here.\n\n```ts\nconst answer = 42;\n```\n\n[docs](https://example.com/docs)\n\n![shot](att:HP-3/shot.png)\n';

const spec = () => {
  const host = superagentReview.hosts![0]!;
  const board = structuredClone(host.board)!;
  for (const project of board) for (const task of project.tasks) {
    task.priority = PRIORITY[task.id] ?? 'medium';
    task.updated = ago(WAITING[task.id] ?? 40);
    if (task.id === 'HP-3' && task.statusReport) task.statusReport = { ...task.statusReport, text: REPORT };
  }
  return { ...superagentReview, hosts: [{ ...host, board }] };
};
function mount() {
  setHostClientFactory(createMockHost(spec()).client);
  setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
  localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'example-app' } }));
  location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
  render(() => <Root />);
}
/** Leaves the page like a route change does; `reload` also forgets what memory held (the stored copy stays). */
function leave() {
  cleanup(); resetPendingMoves(); endQueue(); boardUi().closeTask(); boardUi().setReader(null); resetBoardUi(); resetBoardStore();
  location.hash = '#/chat';
}
const detail = () => screen.getByTestId('task-detail').getAttribute('aria-label');
async function openWorkbench() {
  fireEvent.click(await screen.findByLabelText('Open review (7)'));
  await waitFor(() => expect(detail()).toBeTruthy());
  return screen.getByRole('dialog', { name: 'Workbench' });
}
const ids = (dialog: HTMLElement) => [...dialog.querySelectorAll<HTMLElement>('.sa-review-list-pane .sa-review-row')].map((r) => r.getAttribute('aria-label')!.replace('Open ', ''));
async function closeMenu() {
  fireEvent.click(await screen.findByTestId('anchored-menu-backdrop'));
  await waitFor(() => expect(screen.queryByTestId('anchored-menu-backdrop')).toBeNull());
}
/** Opens the Filter menu once the labels of every ticket have loaded. */
async function filterMenu(dialog: HTMLElement) {
  fireEvent.click(within(dialog).getByRole('button', { name: /^Filter tasks/ }));
  const menu = await screen.findByRole('menu', { name: 'Filter tasks' });
  await waitFor(() => expect(within(menu).getByTestId('filter-label-export')).toBeTruthy());
  return menu;
}
async function sortBy(dialog: HTMLElement, id: string) {
  fireEvent.click(within(dialog).getByRole('button', { name: /^Sort/ }));
  fireEvent.click(await screen.findByTestId(`sort-${id}`));
  await waitFor(() => expect(screen.queryByTestId('anchored-menu-backdrop')).toBeNull());
}

afterEach(() => {
  cleanup(); resetPendingMoves(); resetBoardStore(); toasts().clear(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout();
  setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); boardUi().setReader(null); endQueue(); resetBoardUi(); resetWorkbenchState(); location.hash = '';
});

describe('filterQueue / sortQueue', () => {
  const rows = [{ projectId: 'p', taskId: 'P-10' }, { projectId: 'p', taskId: 'P-9' }, { projectId: 'p', taskId: 'P-2' }];
  const meta = (r: { taskId: string }) => ({ 'P-10': { priority: 'low', labels: ['a'], updated: ago(5) }, 'P-9': { priority: 'urgent', labels: ['b'], updated: ago(50) }, 'P-2': undefined })[r.taskId as 'P-10'];
  const order = (list: { taskId: string }[]) => list.map((r) => r.taskId);
  it('filters by any chosen priority (a ticket without one counts as none) and any chosen label', () => {
    expect(order(filterQueue(rows, meta, ['urgent', 'none'], []))).toEqual(['P-9', 'P-2']);
    expect(order(filterQueue(rows, meta, [], ['a', 'zzz']))).toEqual(['P-10']);
    expect(order(filterQueue(rows, meta, ['urgent'], ['a']))).toEqual([]);
    expect(order(filterQueue(rows, meta, [], []))).toEqual(['P-10', 'P-9', 'P-2']);
  });
  it('sorts by priority, newest, oldest waiting and natural ticket id; default keeps the host order', () => {
    expect(order(sortQueue(rows, meta, 'default'))).toEqual(['P-10', 'P-9', 'P-2']);
    expect(order(sortQueue(rows, meta, 'priority'))).toEqual(['P-9', 'P-10', 'P-2']);
    expect(order(sortQueue(rows, meta, 'updated'))).toEqual(['P-10', 'P-9', 'P-2']);
    expect(order(sortQueue(rows, meta, 'waiting'))).toEqual(['P-9', 'P-10', 'P-2']);
    expect(order(sortQueue(rows, meta, 'id'))).toEqual(['P-2', 'P-9', 'P-10']);
  });
});

describe('Workbench report pane renders markdown like the chat', () => {
  it('lists, headings, code spans, fences, links and att: images go through the chat renderer', async () => {
    mount();
    await openWorkbench();
    await waitFor(() => expect(detail()).toBe('HP-3 details'));
    fireEvent.click(await screen.findByRole('button', { name: 'Read the full report' }));
    const reader = await screen.findByTestId('report-reader');
    const prose = reader.querySelector('.reader-body > .chat-prose.md.md-reader')!;
    expect(prose).not.toBeNull();
    await waitFor(() => expect(prose.querySelectorAll('.chat-block').length).toBeGreaterThan(3));
    expect(prose.querySelector('h2')?.textContent).toBe('Findings');
    expect([...prose.querySelectorAll('li')].map((li) => li.textContent)).toEqual(['first point', 'second point']);
    expect(prose.querySelector('p code')?.textContent).toBe('retryOnce()');
    await waitFor(() => expect(prose.querySelector('.cb pre code')?.textContent).toContain('const answer = 42;'));
    expect(prose.querySelector('.cb [data-copy]')).not.toBeNull();
    expect(prose.querySelector('a[href="https://example.com/docs"]')?.textContent).toBe('docs');
    const image = prose.querySelector<HTMLImageElement>('a.att-image img');
    expect(image?.dataset.attPath).toContain('HP-3/shot.png');
    expect(boardStore().target()?.slug).toBe('example-app');
  });
});

describe('Workbench filters and sort', () => {
  it('filtering by priority narrows the queue, and Clear filters brings it back', async () => {
    mount();
    const dialog = await openWorkbench();
    expect(ids(dialog)).toHaveLength(7);
    await waitFor(() => expect(ids(dialog)).toHaveLength(7));
    const menu = await filterMenu(dialog);
    expect(['urgent', 'high', 'medium', 'low', 'none'].map((p) => within(menu).getByTestId(`filter-priority-${p}`).textContent)).toEqual(['Urgent', 'High', 'Medium', 'Low', 'No priority']);
    fireEvent.click(within(menu).getByTestId('filter-priority-urgent'));
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-5', 'OIN-17']));
    fireEvent.click(within(menu).getByTestId('filter-priority-none'));
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-5', 'OIN-17', 'NOT-2']));
    expect(within(dialog).getByRole('button', { name: 'Filter tasks (2 on)' })).toBeTruthy();
    fireEvent.click(within(menu).getByTestId('filter-clear'));
    await waitFor(() => expect(ids(dialog)).toHaveLength(7));
  });

  it('filtering by labels is multi-select (any of them)', async () => {
    mount();
    const dialog = await openWorkbench();
    await waitFor(() => expect(ids(dialog)).toHaveLength(7));
    const menu = await filterMenu(dialog);
    fireEvent.click(within(menu).getByTestId('filter-label-ui'));
    await waitFor(() => expect(ids(dialog)).toEqual(['OIN-12', 'NOT-2']));
    fireEvent.click(within(menu).getByTestId('filter-label-export'));
    await waitFor(() => expect(ids(dialog)).toEqual(['OIN-12', 'NOT-2', 'NOT-4']));
    expect(within(menu).getByTestId('filter-label-ui').getAttribute('aria-checked')).toBe('true');
    fireEvent.click(within(menu).getByTestId('filter-priority-low'));
    await waitFor(() => expect(ids(dialog)).toEqual(['NOT-4'])); // labels AND priority
  });

  it('sorts by priority, newest updated, oldest waiting and ticket id', async () => {
    mount();
    const dialog = await openWorkbench();
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-3', 'HP-5', 'HP-8', 'OIN-12', 'OIN-17', 'NOT-2', 'NOT-4']));
    // Sorting needs every ticket's detail; the menu's labels show they have loaded.
    await closeMenuAfter(dialog);
    await sortBy(dialog, 'priority');
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-5', 'OIN-17', 'HP-8', 'OIN-12', 'HP-3', 'NOT-4', 'NOT-2']));
    await sortBy(dialog, 'updated');
    await waitFor(() => expect(ids(dialog)).toEqual(['NOT-4', 'OIN-17', 'OIN-12', 'HP-5', 'HP-3', 'NOT-2', 'HP-8']));
    await sortBy(dialog, 'waiting');
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-8', 'NOT-2', 'HP-3', 'HP-5', 'OIN-12', 'OIN-17', 'NOT-4']));
    await sortBy(dialog, 'id');
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-3', 'HP-5', 'HP-8', 'NOT-2', 'NOT-4', 'OIN-12', 'OIN-17']));
    expect(within(dialog).getByRole('button', { name: 'Sort: Ticket id' })).toBeTruthy();
    await sortBy(dialog, 'default');
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-3', 'HP-5', 'HP-8', 'OIN-12', 'OIN-17', 'NOT-2', 'NOT-4']));
  });

  it('the chosen filters and sort are kept per host and come back after leaving the page', async () => {
    mount();
    const dialog = await openWorkbench();
    await waitFor(() => expect(ids(dialog)).toHaveLength(7));
    const menu = await filterMenu(dialog);
    fireEvent.click(within(menu).getByTestId('filter-priority-urgent'));
    fireEvent.click(within(menu).getByTestId('filter-priority-low'));
    fireEvent.click(within(menu).getByTestId('filter-label-sync'));
    await waitFor(() => expect(ids(dialog)).toEqual(['HP-3', 'HP-5']));
    await closeMenu();
    await sortBy(dialog, 'waiting');
    expect(readWorkbenchView('pc-coffee')).toMatchObject({ priorities: ['urgent', 'low'], labels: ['sync'], sort: 'waiting' });
    leave();

    mount();
    const back = await screen.findByRole('dialog', { name: 'Workbench' });
    await waitFor(() => expect(ids(back)).toEqual(['HP-3', 'HP-5']));
    expect(within(back).getByRole('button', { name: 'Filter tasks (3 on)' })).toBeTruthy();
    expect(within(back).getByRole('button', { name: 'Sort: Oldest waiting' })).toBeTruthy();
  });

  it('the default is unchanged: nothing stored, the host order, no active chip', async () => {
    mount();
    const dialog = await openWorkbench();
    expect(ids(dialog)).toEqual(['HP-3', 'HP-5', 'HP-8', 'OIN-12', 'OIN-17', 'NOT-2', 'NOT-4']);
    expect(within(dialog).getByRole('button', { name: 'Filter tasks' })).toBeTruthy();
    expect(within(dialog).getByRole('button', { name: 'Sort: Default order' })).toBeTruthy();
    const view = readWorkbenchView('pc-coffee');
    expect(view.priorities).toBeUndefined(); expect(view.labels).toBeUndefined(); expect(view.sort).toBeUndefined();
  });
});

/** Waits until every ticket's detail (priority, labels, updated) has arrived by watching the Filter menu's labels, then closes it. */
async function closeMenuAfter(dialog: HTMLElement) {
  await filterMenu(dialog);
  await closeMenu();
}
