// @vitest-environment jsdom
// QA round 2 of the Workbench refine (r-qa-workbench-ui): one regression test per defect.
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentReview } from '../../dev/mock/scenarios/superagent-review';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { api, type Attachment } from '../../lib/board/api';
import { setBlobTransport } from '../../lib/blob';
import { boardUi, resetBoardUi } from '../../lib/board/ui';
import { boardStore, resetBoardStore } from '../../lib/board/store';
import { endQueue } from '../../lib/board/taskOrigin';
import { flushMoves, resetPendingMoves } from '../../lib/board/pendingMoves';
import { toasts } from '../../lib/toasts';
import type { DropZone } from '../../lib/localFiles';
import { StatusSchema, superagentApi } from './api';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
const zones: DropZone[] = [];
vi.mock('../../lib/localFiles', () => ({
  clipboardFiles: vi.fn(async () => []),
  describeLocal: vi.fn(async () => []),
  readLocalFiles: vi.fn(async () => []),
  registerDropZone: vi.fn((zone: DropZone) => { zones.push(zone); return () => { zones.splice(zones.indexOf(zone), 1); }; }),
  startNativeDrop: vi.fn(async () => {}),
}));

const spec = () => {
  const host = superagentReview.hosts![0]!;
  return { ...superagentReview, hosts: [{ ...host, board: structuredClone(host.board), fs: { ...(host.fs ?? {}), '~/.oino/uploads': { kind: 'dir' as const, children: {} } } }] };
};
let world: ReturnType<typeof createMockHost>;
function mount(phone = false, hash = '#/superagent?session=superagent-chat&host=pc-coffee') {
  world = createMockHost(spec());
  setHostClientFactory(world.client);
  setPlatformOverride(phone ? 'tauri-mobile' : 'tauri-desktop'); setDesktopOverride(!phone);
  localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'example-app' } }));
  location.hash = hash;
  render(() => <Root />);
}
const detail = () => screen.getByTestId('task-detail').getAttribute('aria-label');
const stored = (file: File): Attachment => ({
  name: `ab12cd34-${file.name}`, original: file.name, ref: `att:HP-3/ab12cd34-${file.name}`, path: `/x/${file.name}`, size: 4, mime: file.type, kind: file.type.startsWith('image/') ? 'image' : 'pdf',
  markdown: `![${file.name}](att:HP-3/ab12cd34-${file.name})`,
});
const clip = (...files: File[]) => ({ clipboardData: { types: [], getData: () => '', files } });
const png = (name = 'shot.png') => new File(['x'], name, { type: 'image/png' });
const dockNote = () => screen.getByLabelText('Add a note') as HTMLTextAreaElement;
async function openWorkbench(selector = 'Open review (7)') {
  fireEvent.click(await screen.findByLabelText(selector));
  await waitFor(() => expect(detail()).toBe('HP-3 details'));
  await screen.findByLabelText('Previous task');
  return screen.getByRole('dialog', { name: 'Workbench' });
}
const openReport = async () => { fireEvent.click(await screen.findByRole('button', { name: 'Read the full report' })); return screen.findByTestId('report-reader'); };

const WIDTH = window.innerWidth;
beforeEach(() => { zones.length = 0; });
afterEach(() => {
  cleanup(); resetPendingMoves(); resetBoardStore(); toasts().clear(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout();
  setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); boardUi().setReader(null); endQueue(); resetBoardUi(); location.hash = '';
  Object.defineProperty(window, 'innerWidth', { value: WIDTH, configurable: true });
});

describe('P0 #1: every note field is a native drop zone and reads the clipboard through the core', () => {
  it('the escalation reply registers its field', async () => {
    const status = StatusSchema.parse({ enabled: true, leads: [], review: [], projects: [{ projectId: 'website', name: 'Website', autoStart: true }],
      needsYou: [{ projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain?' }] });
    vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: status });
    mount();
    fireEvent.click(await screen.findByLabelText('Open escalations (1)'));
    const reply = (await screen.findByLabelText('Reply to WEB-8')).closest('.note-field')!;
    expect(zones.some((z) => z.el() === reply)).toBe(true);
  });

  it('the Workbench dock and the move-note sheet register their fields', async () => {
    mount();
    await openWorkbench();
    const dock = dockNote().closest('.note-field')!;
    expect(dock.closest('.review-dock')).toBeTruthy();
    expect(zones.some((z) => z.el() === dock)).toBe(true);
    boardUi().setComment({ task: boardStore().taskById('HP-3')!, to: 'todo', hint: '', optional: true });
    const sheet = (await screen.findByLabelText('Note (optional)')).closest('.note-field')!;
    expect(sheet.closest('[data-testid=comment-sheet]')).toBeTruthy();
    expect(zones.some((z) => z.el() === sheet)).toBe(true);
    boardUi().setComment(null);
  });

  it('the Board composer (a ticket that is not in review) registers its field too', async () => {
    mount(false, '#/board');
    await waitFor(() => expect(boardStore().state.projectsLoaded).toBe(true));
    await api.move('pc-coffee', 'example-app', 'HP-5', 'todo');
    await boardStore().reload();
    boardUi().openTask('HP-5');
    await screen.findByTestId('task-detail');
    const field = (await screen.findByLabelText('Add a note')).closest('.note-field')!;
    expect(field.closest('.review-dock')).toBeNull();
    expect(zones.some((z) => z.el() === field)).toBe(true);
  });
});

describe('P1 #2: the side report pane follows the list', () => {
  it('closes on a tab change', async () => {
    mount();
    await openWorkbench();
    await openReport();
    fireEvent.click(screen.getByRole('radio', { name: /Escalate/ }));
    await waitFor(() => expect(screen.queryByTestId('report-reader')).toBeNull());
  });
  it('closes on Back to the list', async () => {
    mount();
    await openWorkbench();
    await openReport();
    fireEvent.click(screen.getByLabelText('Back to Superagent'));
    await waitFor(() => expect(screen.queryByTestId('report-reader')).toBeNull());
  });
  it('closes on a search with no match, which says "No match"', async () => {
    mount();
    const dialog = await openWorkbench();
    await openReport();
    fireEvent.input(screen.getByLabelText('Search tasks'), { target: { value: 'zzzz' } });
    await waitFor(() => expect(screen.queryByTestId('report-reader')).toBeNull());
    expect(within(dialog).getByText('No match')).toBeTruthy();
    expect(within(dialog).getByText(/No task matches/)).toBeTruthy();
    expect(within(dialog).queryByText('Nothing to review.')).toBeNull();
  });
  it('closes with the last ticket: "Queue done" has no report beside it and the list does not repeat itself', async () => {
    mount();
    const dialog = await openWorkbench();
    for (let i = 0; i < 7; i += 1) {
      await openReport();
      fireEvent.click(await screen.findByText('Mark done'));
      if (i < 6) await waitFor(() => expect(within(dialog).queryAllByRole('button', { name: /^Open / }).length).toBe(6 - i));
    }
    await screen.findByText('Queue done');
    expect(screen.queryByTestId('report-reader')).toBeNull();
    expect(dialog.querySelector('.sa-review-pane')?.hasAttribute('data-open')).toBe(false);
    expect(within(dialog).queryByText('Nothing to review.')).toBeNull();
  });
  it('the empty state is centred by CSS: the ticket mount does not hold the pane open', () => {
    const css = readFileSync(resolve(__dirname, 'superagent.css'), 'utf8');
    expect(css).toMatch(/\.sa-review-task-mount:not\(:has\(\.task-page\)\)\s*\{\s*display:none/);
    expect(css).toMatch(/\.sa-review-task-mount:not\(:has\(\.task-page\)\)\s*~\s*\.sa-empty\s*\{[^}]*margin:auto/);
  });
});

describe('P1 #3: Esc in a portalled sheet belongs to the sheet', () => {
  it('Esc in the Labels filter is not taken by the Workbench (no blur, no preventDefault) and the Workbench stays', async () => {
    mount();
    const dialog = await openWorkbench();
    fireEvent.click(screen.getByLabelText('Edit labels'));
    const filter = await screen.findByLabelText('Filter labels');
    filter.focus();
    const esc = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    filter.dispatchEvent(esc);
    expect(esc.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(filter);
    expect(dialog.isConnected).toBe(true);
  });
  it('Esc on the Done confirm sheet does not close the Workbench', async () => {
    mount();
    const dialog = await openWorkbench();
    fireEvent.click(screen.getByRole('button', { name: 'Status: In Review' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Done/ }));
    const cancel = await screen.findByRole('button', { name: 'Cancel' });
    cancel.focus();
    fireEvent.keyDown(cancel, { key: 'Escape' });
    await new Promise((r) => setTimeout(r, 30));
    expect(dialog.isConnected).toBe(true);
    expect(screen.getByRole('dialog', { name: 'Workbench' })).toBeTruthy();
  });
  it('Esc inside the Workbench itself still steps back: field → ticket → pane → close', async () => {
    mount();
    await openWorkbench();
    dockNote().focus();
    fireEvent.keyDown(dockNote(), { key: 'Escape' });
    expect(document.activeElement).toBe(screen.getByTestId('task-detail'));
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Workbench' })).toBeNull());
  });
});

describe('P1 #4: Back to Todo (rework) from the status menu asks for a note, with media', () => {
  it('opens the optional note sheet inside the Workbench; Move without note sends a plain move', async () => {
    mount();
    await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.click(screen.getByRole('button', { name: 'Status: In Review' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Back to Todo \(rework\)/ }));
    const sheet = await screen.findByTestId('comment-sheet');
    expect(move).not.toHaveBeenCalled();
    expect(within(sheet).getByText(/A note is optional/)).toBeTruthy();
    fireEvent.click(within(sheet).getByRole('button', { name: 'Move without note' }));
    await waitFor(() => expect(move).toHaveBeenCalledWith('pc-coffee', 'example-app', 'HP-3', 'todo', undefined, []));
  });
  it('the dock\'s own Rework and the dock Mark done are not asked again (confirmed)', async () => {
    mount();
    await openWorkbench();
    fireEvent.input(dockNote(), { target: { value: 'fix it' } });
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(screen.queryByTestId('comment-sheet')).toBeNull();
    await flushMoves();
  });
});

describe('P1 #5: stepping through the queue always leaves the right report in the pane (1440 px)', () => {
  it('after every ] and [ the pane shows the ticket on screen', async () => {
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    mount();
    await openWorkbench();
    const shows = async (id: string) => { await waitFor(() => expect(detail()).toBe(`${id} details`)); await waitFor(() => expect(within(screen.getByTestId('report-reader')).getByText(`${id} · Report`)).toBeTruthy()); };
    await shows('HP-3');
    for (const id of ['HP-5', 'HP-8', 'OIN-12', 'OIN-17', 'NOT-2', 'NOT-4']) {
      fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'j' });
      await shows(id);
    }
    for (const id of ['NOT-2', 'OIN-17', 'OIN-12', 'HP-8', 'HP-5', 'HP-3']) {
      fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'k' });
      await shows(id);
    }
  });
  it('rapid j/k presses without waiting end on the right report', async () => {
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    mount();
    await openWorkbench();
    await waitFor(() => expect(screen.getByTestId('report-reader')).toBeTruthy());
    for (const key of ['j', 'j', 'k', 'j', 'j', 'k', 'k', 'j']) fireEvent.keyDown(screen.getByTestId('task-detail'), { key });
    await new Promise((r) => setTimeout(r, 200));
    const id = detail()!.replace(' details', '');
    await waitFor(() => expect(within(screen.getByTestId('report-reader')).getByText(`${id} · Report`)).toBeTruthy());
  });
});

describe('P2 #10: closing within the undo window says it was saved', () => {
  it('Mark done then Close: the move is saved at once and a toast says so', async () => {
    mount();
    await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.click(screen.getByRole('button', { name: /Mark done/ }));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    fireEvent.click(screen.getAllByLabelText('Close workbench')[0]!);
    await waitFor(() => expect(move).toHaveBeenCalled());
    await waitFor(() => expect(boardUi().toasts().some((t) => t.message === 'HP-3 marked done — saved')).toBe(true));
    expect(boardUi().toasts().some((t) => t.action?.label === 'Undo' && !t.leaving)).toBe(false);
  });
});

describe('P2 #14: queue rows carry the ticket in their title (the 1100 px rail shows only a dot and a number)', () => {
  it('every row has project · id · title as its tooltip', async () => {
    mount();
    const dialog = await openWorkbench();
    const title = boardStore().taskById('HP-5')?.title ?? '';
    expect(title).not.toBe('');
    await waitFor(() => expect(within(dialog).getByLabelText('Open HP-5').getAttribute('title')).toBe(`ExampleApp · HP-5 · ${title}`));
    expect(within(dialog).getByLabelText('Open HP-5').textContent).toContain(title);
  });
});

describe('P2 #13: the phone Undo toast stays clear of the sheet title and Back', () => {
  it('while the Workbench sheet is open the stack is not pinned to the top', async () => {
    mount(true);
    fireEvent.click(await screen.findByLabelText('Open review (7)'));
    fireEvent.click(await screen.findByLabelText('Open HP-3'));
    fireEvent.click(await screen.findByRole('button', { name: /Mark done/ }));
    await waitFor(() => expect(screen.getAllByText('HP-3 marked done').length).toBeGreaterThan(0));
    const stack = screen.getByTestId('app-toasts');
    await waitFor(() => expect(stack.dataset.pos).toBeUndefined());
    expect(document.querySelector('.sa-review-sheet')).toBeTruthy();
    expect(document.querySelector('.sheet')).toBeTruthy(); // what made the stack jump to the top before
  });
});

describe('P2 #15: cancelling the move note stores nothing', () => {
  it('a pasted image is held (not uploaded); Cancel leaves the ticket untouched', async () => {
    mount();
    await openWorkbench();
    const upload = vi.spyOn(api, 'uploadFile');
    const move = vi.spyOn(api, 'move');
    boardUi().setComment({ task: boardStore().taskById('HP-3')!, to: 'todo', hint: '', optional: true });
    const field = await screen.findByLabelText('Note (optional)') as HTMLTextAreaElement;
    fireEvent.paste(field, clip(png()));
    const preview = await screen.findByLabelText('Attached to this note');
    expect(preview.querySelector('[data-held]')).toBeTruthy();
    expect(upload).not.toHaveBeenCalled();
    fireEvent.click(within(screen.getByTestId('comment-sheet')).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(boardUi().comment()).toBeNull());
    expect(upload).not.toHaveBeenCalled();
    expect(move).not.toHaveBeenCalled();
    expect(world.log.filter((l) => l.op === 'blob_put_bytes' || l.op === 'fs_import')).toHaveLength(0);
  });
  it('a held file can be removed; Move sends the rest with the move', async () => {
    mount();
    await openWorkbench();
    const move = vi.spyOn(api, 'move');
    boardUi().setComment({ task: boardStore().taskById('HP-3')!, to: 'todo', hint: '', optional: true });
    const field = await screen.findByLabelText('Note (optional)') as HTMLTextAreaElement;
    fireEvent.paste(field, clip(png('a.png')));
    fireEvent.paste(field, clip(png('b.png')));
    await screen.findByLabelText('Remove b.png');
    fireEvent.click(screen.getByLabelText('Remove a.png'));
    fireEvent.input(field, { target: { value: 'see b' } });
    fireEvent.click(within(screen.getByTestId('comment-sheet')).getByRole('button', { name: /^Move$|^Back to Todo$/ }));
    await waitFor(() => expect(move).toHaveBeenCalled());
    const call = move.mock.calls[0]!;
    expect(call.slice(0, 5)).toEqual(['pc-coffee', 'example-app', 'HP-3', 'todo', 'see b']);
    expect((call[5] as File[]).map((f) => f.name)).toEqual(['b.png']);
  });
  it('api.move stages each file on the host and passes it as --attach', async () => {
    mount();
    await waitFor(() => expect(world.client).toBeTruthy());
    // The app's `blob_put_bytes` needs the transport's progress channel (OIN-93): staging goes through the blob transport.
    const putBytes = vi.fn((hostId: string, bytes: Uint8Array, mime: string, name?: string) => world.client(hostId).request('blob_put_bytes', { bytes: Array.from(bytes), mime, name }) as Promise<never>);
    setBlobTransport({ putBytes } as never);
    try {
      await api.move('pc-coffee', 'example-app', 'HP-3', 'todo', 'note', [png('c.png')]);
    } finally {
      setBlobTransport(undefined);
    }
    expect(putBytes).toHaveBeenCalledWith('pc-coffee', expect.any(Uint8Array), 'application/octet-stream', 'c.png');
    const call = world.log.filter((l) => l.op === 'board_cli').map((l) => (l.args as { args: string[] }).args).find((a) => a[0] === 'move')!;
    expect(call.slice(0, 5)).toEqual(['move', 'HP-3', 'todo', '--comment', 'note']);
    expect(call[5]).toBe('--attach');
    expect(call[6]).toMatch(/^~\/\.oino\/uploads\/board-.*-c\.png$/);
  });
});
