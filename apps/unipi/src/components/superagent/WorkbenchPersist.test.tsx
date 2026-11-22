// @vitest-environment jsdom
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
import { flushMoves, isPending, resetPendingMoves } from '../../lib/board/pendingMoves';
import { toasts } from '../../lib/toasts';
import { readDraft, readWorkbenchView } from '../../lib/superagent/workbenchState';
import { StatusSchema, superagentApi } from './api';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));

const HASH = '#/superagent?session=superagent-chat&host=pc-coffee';
/** `gone` leaves the review lane while the owner is away. */
const spec = (gone: string[] = []) => {
  const host = superagentReview.hosts![0]!;
  const board = structuredClone(host.board)!;
  for (const project of board) for (const task of project.tasks) if (gone.includes(task.id)) task.status = 'done';
  return { ...superagentReview, hosts: [{ ...host, board }] };
};
function mount(gone: string[] = []) {
  setHostClientFactory(createMockHost(spec(gone)).client);
  setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
  localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'example-app' } }));
  location.hash = HASH;
  render(() => <Root />);
}
/** Leaves the page like a route change does, and forgets everything held in memory (as a reload would). */
function leave(reload = false) {
  cleanup(); resetPendingMoves(); endQueue(); boardUi().closeTask(); boardUi().setReader(null);
  if (reload) { resetBoardUi(); resetBoardStore(); }
  location.hash = '#/chat';
}
const detail = () => screen.getByTestId('task-detail').getAttribute('aria-label');
const dockNote = () => screen.getByLabelText('Add a note') as HTMLTextAreaElement;
async function pickExampleApp(dialog: HTMLElement) {
  fireEvent.click(within(dialog).getByRole('button', { name: 'Project: All projects' }));
  const menu = await screen.findByRole('menu', { name: 'Filter by project' });
  fireEvent.click(await within(menu).findByText('ExampleApp'));
  await waitFor(() => expect(within(dialog).getByRole('button', { name: 'Project: ExampleApp' })).toBeTruthy());
}
async function openExampleAppHp5() {
  mount();
  fireEvent.click(await screen.findByLabelText('Open review (7)'));
  const dialog = await screen.findByRole('dialog', { name: 'Workbench' });
  await waitFor(() => expect(detail()).toBeTruthy());
  await pickExampleApp(dialog);
  fireEvent.click(await within(dialog).findByLabelText('Open HP-5'));
  await waitFor(() => expect(detail()).toBe('HP-5 details'));
  return dialog;
}

afterEach(() => { cleanup(); resetPendingMoves(); resetBoardStore(); toasts().clear(); vi.useRealTimers(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); boardUi().setReader(null); endQueue(); resetBoardUi(); location.hash = ''; });

describe('Workbench keeps its place across a visit elsewhere', () => {
  it('the project filter, the open ticket and a half-typed reply survive unmount and remount (and a reload)', async () => {
    await openExampleAppHp5();
    fireEvent.input(dockNote(), { target: { value: 'tighten the retry copy' } });
    expect(readDraft('pc-coffee', 'example-app/HP-5')).toBe('tighten the retry copy');
    leave(true);

    mount();
    const dialog = await screen.findByRole('dialog', { name: 'Workbench' });
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(within(dialog).getByRole('button', { name: 'Project: ExampleApp' })).toBeTruthy();
    expect(within(dialog).queryByLabelText('Open OIN-12')).toBeNull();
    await waitFor(() => expect(dockNote().value).toBe('tighten the retry copy'));
  });

  it('a ticket that left the queue while away falls back to the next one, not the first', async () => {
    await openExampleAppHp5();
    leave(true);

    mount(['HP-5']);
    await screen.findByRole('dialog', { name: 'Workbench' });
    await waitFor(() => expect(detail()).toBe('HP-8 details'));
  });

  it('with the last ticket gone it falls back to the one before it', async () => {
    await openExampleAppHp5();
    fireEvent.click(await screen.findByLabelText('Open HP-8'));
    await waitFor(() => expect(detail()).toBe('HP-8 details'));
    leave(true);

    mount(['HP-8']);
    await screen.findByRole('dialog', { name: 'Workbench' });
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
  });

  it('closing the Workbench on the page is remembered: it stays closed on return', async () => {
    const dialog = await openExampleAppHp5();
    fireEvent.keyDown(dialog, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Workbench' })).toBeNull());
    expect(readWorkbenchView('pc-coffee').open).toBeUndefined();
    leave(true);

    mount();
    await screen.findByLabelText(/Open review/);
    expect(screen.queryByRole('dialog', { name: 'Workbench' })).toBeNull();
  });

  it('the reply draft is cleared once it is sent (Rework)', async () => {
    await openExampleAppHp5();
    fireEvent.input(dockNote(), { target: { value: 'use the debounce helper' } });
    expect(readDraft('pc-coffee', 'example-app/HP-5')).toBe('use the debounce helper');
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    await waitFor(() => expect(isPending('example-app', 'HP-5')).toBe(true));
    expect(readDraft('pc-coffee', 'example-app/HP-5')).toBe('');
    await flushMoves();
    expect(boardStore().taskById('HP-5')?.status).not.toBe('in_review');
  });
});

describe('Workbench: Escalate replies', () => {
  const status = () => StatusSchema.parse({
    enabled: true, leads: [],
    needsYou: [{ projectId: 'cloud', taskId: 'CLO-6', status: 'blocked', question: 'Ship to all tenants?' }, { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should the staging deploy use?' }],
    review: [], projects: [{ projectId: 'cloud', name: 'Cloud', autoStart: false }, { projectId: 'website', name: 'Website', autoStart: true }],
  });
  function mountEscalate() {
    mount();
    vi.spyOn(superagentApi, 'status').mockImplementation(async () => ({ ok: true, value: status() }));
  }
  it('a half-typed reply and the pager position come back, and sending clears the reply', async () => {
    mountEscalate();
    fireEvent.click(await screen.findByLabelText('Open escalations (2)'));
    fireEvent.click(await screen.findByLabelText('Next question'));
    fireEvent.input(await screen.findByLabelText('Reply to WEB-8'), { target: { value: 'staging.example.com' } });
    expect(readDraft('pc-coffee', 'answer:website/WEB-8')).toBe('staging.example.com');
    leave(true);

    mountEscalate();
    const reply = await screen.findByLabelText('Reply to WEB-8') as HTMLTextAreaElement;
    expect(reply.value).toBe('staging.example.com');
    expect(screen.getByRole('group', { name: 'Questions' }).textContent).toContain('2 of 2');
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue({ ok: true, value: { commentId: null, sessionId: 'lead', delivered: true } });
    fireEvent.click(screen.getByLabelText('Send reply for WEB-8'));
    await waitFor(() => expect(relay).toHaveBeenCalled());
    await waitFor(() => expect(readDraft('pc-coffee', 'answer:website/WEB-8')).toBe(''));
  });
});
