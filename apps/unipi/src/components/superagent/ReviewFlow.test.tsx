// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentReview } from '../../dev/mock/scenarios/superagent-review';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { boardUi } from '../../lib/board/ui';
import { boardStore } from '../../lib/board/store';
import { endQueue, taskOrigin } from '../../lib/board/taskOrigin';
import { setReviewAll, setReviewOpen } from './reviewUi';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));

const spec = () => { const host = superagentReview.hosts![0]!; return { ...superagentReview, hosts: [{ ...host, board: structuredClone(host.board) }] }; };
function mount(hash = '#/superagent?session=superagent-chat&host=pc-coffee') {
  setHostClientFactory(createMockHost(spec()).client);
  setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
  localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'heartpick' } }));
  location.hash = hash;
  render(() => <Root />);
}
const pos = async () => (await screen.findByTestId('task-queue')).querySelector('.task-queue-pos')!.textContent;
const detail = () => screen.getByTestId('task-detail').getAttribute('aria-label');
async function openReview(task: string) {
  await screen.findByTestId('superagent-page');
  const review = await screen.findByTestId('sa-review');
  const head = within(review).getByText('Review').closest('button')!;
  if (head.getAttribute('aria-expanded') !== 'true') fireEvent.click(head);
  if (!screen.queryByLabelText(`Open ${task}`)) fireEvent.click(await within(review).findByText(/Show \d+ more/));
  fireEvent.click(await screen.findByLabelText(`Open ${task}`));
  await screen.findByTestId('task-detail');
}

beforeEach(() => { setReviewOpen(false); setReviewAll(false); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); endQueue(); location.hash = ''; });

describe('OIN-40: reviewing tasks from Superagent', () => {
  it('opens over the Superagent page with the origin and "N of M" in the header', async () => {
    mount();
    await openReview('HP-5');
    expect(taskOrigin()?.from).toBe('superagent');
    expect(await pos()).toBe('2 of 7');
    expect(detail()).toBe('HP-5 details');
    expect(location.hash).toContain('/superagent'); // no route change: Kanboard is never visited
    expect(screen.getByLabelText('Back to Superagent')).toBeTruthy();
  });

  it('Prev / Next move through the snapshot (across projects) without stacking history', async () => {
    mount();
    await openReview('HP-8');
    expect(await pos()).toBe('3 of 7');
    fireEvent.click(screen.getByLabelText('Next task'));
    await waitFor(() => expect(detail()).toBe('OIN-12 details'));
    expect(await pos()).toBe('4 of 7');
    fireEvent.click(screen.getByLabelText('Previous task'));
    await waitFor(() => expect(detail()).toBe('HP-8 details'));
    expect(boardUi().detail()).toEqual(['HP-8']); // replaced, not pushed: one Back leaves the detail
    expect((screen.getByLabelText('Previous task') as HTMLButtonElement).disabled).toBe(false);
  });

  it('disables Prev on the first and Next on the last item', async () => {
    mount();
    await openReview('HP-3');
    expect((await screen.findByLabelText('Previous task') as HTMLButtonElement).disabled).toBe(true);
    expect(await pos()).toBe('1 of 7');
    fireEvent.click(screen.getByLabelText('Back to Superagent'));
    await waitFor(() => expect(screen.queryByTestId('task-detail')).toBeNull());
    await openReview('NOT-4');
    expect((await screen.findByLabelText('Next task') as HTMLButtonElement).disabled).toBe(true);
    expect(await pos()).toBe('7 of 7');
  });

  it('Mark done shows the next task in place, with no dialog close/reopen and no navigation', async () => {
    mount();
    await openReview('HP-3');
    const dialog = screen.getByTestId('task-detail');
    fireEvent.click(await screen.findByText('Mark done'));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(screen.getByTestId('task-detail')).toBe(dialog); // the same page element: it never closed
    expect(await pos()).toBe('1 of 6'); // HP-3 left the queue
    expect(location.hash).toContain('/superagent');
    // …and again, crossing into the next project.
    fireEvent.click(await screen.findByText('Mark done'));
    await waitFor(() => expect(detail()).toBe('HP-8 details'));
    fireEvent.click(await screen.findByText('Mark done'));
    await waitFor(() => expect(detail()).toBe('OIN-12 details'));
    expect(await pos()).toBe('1 of 4');
  });

  it('Rework advances the same way', async () => {
    mount();
    await openReview('OIN-12');
    fireEvent.click(await screen.findByText('Rework…'));
    fireEvent.click(await screen.findByText('Move without note'));
    await waitFor(() => expect(detail()).toBe('OIN-17 details'));
    expect(await pos()).toBe('4 of 6');
  });

  it('after the last item it falls back to the nearest earlier one still waiting', async () => {
    mount();
    await openReview('NOT-4');
    fireEvent.click(await screen.findByText('Mark done'));
    await waitFor(() => expect(detail()).toBe('NOT-2 details'));
    expect(await pos()).toBe('6 of 6');
  });

  it('an exhausted queue returns to the Superagent page with a "queue done" toast', async () => {
    mount();
    await openReview('HP-3');
    for (let i = 0; i < 7; i += 1) {
      fireEvent.click(await screen.findByText('Mark done'));
      if (i < 6) await waitFor(() => expect(taskOrigin()?.items.length).toBe(6 - i));
    }
    await waitFor(() => expect(screen.queryByTestId('task-detail')).toBeNull());
    expect(taskOrigin()).toBeUndefined();
    expect(boardUi().toasts().some((t) => t.message === 'Review queue done')).toBe(true);
    expect(location.hash).toContain('/superagent');
    expect(screen.getByTestId('superagent-page')).toBeTruthy();
    // Review is still open, and now empty.
    await screen.findByText('Nothing to review.');
  });

  it('Back returns to Superagent with the Review accordion still expanded and its rows intact', async () => {
    mount();
    await openReview('HP-5');
    fireEvent.click(screen.getByLabelText('Back to Superagent'));
    await waitFor(() => expect(screen.queryByTestId('task-detail')).toBeNull());
    expect(taskOrigin()).toBeUndefined();
    expect(location.hash).toContain('/superagent');
    expect(within(screen.getByTestId('sa-review')).getByText('Review').closest('button')!.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByLabelText('Open HP-5')).toBeTruthy();
  });

  it('leaving the page closes an open ticket and forgets the queue', async () => {
    mount();
    await openReview('HP-5');
    location.hash = '#/cron';
    await waitFor(() => expect(taskOrigin()).toBeUndefined());
    expect(boardUi().openedTask()).toBeUndefined();
  });
});

describe('OIN-40: a task opened from the Kanboard is unchanged', () => {
  it('has no queue controls and Back stays on the board', async () => {
    mount('#/board');
    await waitFor(() => expect(boardStore().state.loaded).toBe(true), { timeout: 4000 });
    boardUi().openTask('HP-3');
    await screen.findByTestId('task-detail');
    expect(screen.queryByTestId('task-queue')).toBeNull();
    expect(screen.getByLabelText('Back to the board')).toBeTruthy();
    fireEvent.click(screen.getByLabelText('Back to the board'));
    await waitFor(() => expect(screen.queryByTestId('task-detail')).toBeNull());
    expect(location.hash).toBe('#/board');
  });
});
