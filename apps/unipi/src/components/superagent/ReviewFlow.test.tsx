// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
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
import { StatusSchema, superagentApi } from './api';

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
  if (!screen.queryByRole('dialog', { name: 'Review' })) fireEvent.click(await screen.findByLabelText('Open review (7)'));
  fireEvent.click(await screen.findByLabelText(`Open ${task}`));
  await screen.findByTestId('task-detail');
}

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
    expect(await screen.findByLabelText('Open review (6)')).toBeTruthy();
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
    expect(screen.getByRole('dialog', { name: 'Review' })).toBeTruthy();
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

describe('OIN-40: review capsule and workspace', () => {
  it('excludes archived projects and hides zero escalations', async () => {
    const status = StatusSchema.parse({ enabled: true, concurrencyCap: 1, leads: [], needsYou: [], review: [
      { projectId: 'live', taskId: 'LIVE-1', status: 'in_review' },
      { projectId: 'old', taskId: 'OLD-1', status: 'in_review' },
      { projectId: 'live', taskId: 'LIVE-2', status: 'archived' },
    ], projects: [{ projectId: 'live', autoStart: true }, { projectId: 'old', autoStart: false, archived: true }] });
    vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: status });
    mount();
    expect(await screen.findByLabelText('Open review (1)')).toBeTruthy();
    expect(screen.queryByLabelText(/Open escalations/)).toBeNull();
    fireEvent.click(screen.getByLabelText('Open review (1)'));
    expect(await screen.findByLabelText('Open LIVE-1')).toBeTruthy();
    expect(screen.queryByLabelText('Open OLD-1')).toBeNull();
  });
  it('opens each segment with its filter, supports empty search and restores focus on Escape', async () => {
    const status = StatusSchema.parse({ enabled: true, concurrencyCap: 1, leads: [],
      needsYou: [{ projectId: 'heartpick', taskId: 'HP-3', status: 'blocked', question: 'What next?' }],
      review: [{ projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }], projects: [] });
    vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: status });
    mount();
    const segment = await screen.findByLabelText('Open escalations (1)');
    segment.focus(); fireEvent.click(segment);
    expect((await screen.findByRole('tab', { name: 'Escalate · 1' })).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByLabelText('Open HP-3')).toBeTruthy();
    expect(screen.queryByLabelText('Open OIN-12')).toBeNull();
    fireEvent.input(screen.getByLabelText('Search review queue'), { target: { value: 'not-found' } });
    expect(screen.getByText('Nothing to review.')).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Review' }), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Review' })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(segment));
    fireEvent.click(screen.getByLabelText('Open review (1)'));
    expect((await screen.findByRole('tab', { name: 'Review · 1' })).getAttribute('aria-selected')).toBe('true');
  });
  it('restores a deep-linked task and replaces the task query on Next', async () => {
    mount('#/superagent?session=superagent-chat&host=pc-coffee&review=review&task=HP-5');
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    fireEvent.click(screen.getByLabelText('Next task'));
    await waitFor(() => expect(detail()).toBe('HP-8 details'));
    await waitFor(() => expect(location.hash).toContain('task=HP-8'));
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
