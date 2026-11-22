// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentReview } from '../../dev/mock/scenarios/superagent-review';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { api, type Attachment } from '../../lib/board/api';
import { boardUi, resetBoardUi } from '../../lib/board/ui';
import { boardStore, resetBoardStore } from '../../lib/board/store';
import { endQueue } from '../../lib/board/taskOrigin';
import { flushMoves, isPending, resetPendingMoves, UNDO_MS } from '../../lib/board/pendingMoves';
import { toasts } from '../../lib/toasts';
import { StatusSchema, superagentApi } from './api';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));

const spec = () => { const host = superagentReview.hosts![0]!; return { ...superagentReview, hosts: [{ ...host, board: structuredClone(host.board) }] }; };
function mount(phone = false, hash = '#/superagent?session=superagent-chat&host=pc-coffee') {
  setHostClientFactory(createMockHost(spec()).client);
  setPlatformOverride(phone ? 'tauri-mobile' : 'tauri-desktop'); setDesktopOverride(!phone);
  localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'example-app' } }));
  location.hash = hash;
  render(() => <Root />);
}
const detail = () => screen.getByTestId('task-detail').getAttribute('aria-label');
const stored = (file: File): Attachment => ({
  name: `ab12cd34-${file.name}`, original: file.name, ref: `att:HP-3/ab12cd34-${file.name}`, path: `/x/${file.name}`, size: 4, mime: file.type, kind: file.type.startsWith('image/') ? 'image' : 'pdf',
  markdown: file.type.startsWith('image/') ? `![${file.name}](att:HP-3/ab12cd34-${file.name})` : `[${file.name}](att:HP-3/ab12cd34-${file.name})`,
});
async function openWorkbench() {
  fireEvent.click(await screen.findByLabelText('Open review (7)'));
  await waitFor(() => expect(detail()).toBe('HP-3 details')); // the first row opens by itself
  await screen.findByLabelText('Previous task');
  return screen.getByRole('dialog', { name: 'Workbench' });
}
const dockNote = () => screen.getByLabelText('Add a note') as HTMLTextAreaElement;

afterEach(() => { cleanup(); resetPendingMoves(); resetBoardStore(); toasts().clear(); vi.useRealTimers(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); boardUi().setReader(null); endQueue(); resetBoardUi(); location.hash = ''; });

describe('Workbench refinement: rework inline', () => {
  it('Rework never opens a modal: the dock field takes focus and the ticket stays visible', async () => {
    mount();
    const dialog = await openWorkbench();
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    expect(screen.queryByTestId('comment-sheet')).toBeNull();
    expect(document.querySelector('[data-testid=comment-sheet]')).toBeNull();
    expect(document.activeElement).toBe(dockNote());
    expect(within(dialog).getByTestId('rework-hint').textContent).toContain('press Rework again');
    expect(detail()).toBe('HP-3 details');
  });

  it('a typed note is sent as the rework comment after the undo window, with the next ticket open at once', async () => {
    mount();
    await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.input(dockNote(), { target: { value: 'fix X' } });
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(isPending('example-app', 'HP-3')).toBe(true);
    expect(move).not.toHaveBeenCalled();
    expect(boardUi().toasts().some((t) => t.message === 'HP-3 sent back to Todo' && t.action?.label === 'Undo')).toBe(true);
    // The queue shows it gone while it waits.
    expect(screen.queryByLabelText('Open HP-3')).toBeNull();
    expect(screen.getByLabelText('Open review (6)')).toBeTruthy();
    await flushMoves();
    expect(move).toHaveBeenCalledWith('pc-coffee', 'example-app', 'HP-3', 'todo', 'fix X');
  });

  it('Mark done shows the next ticket within 300 ms, Undo reopens the first with its note', async () => {
    mount();
    await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.input(dockNote(), { target: { value: 'looks good, ship it' } });
    const started = performance.now();
    fireEvent.click(screen.getByRole('button', { name: /Mark done/ }));
    await waitFor(() => expect(detail()).toBe('HP-5 details'), { timeout: 300 });
    expect(performance.now() - started).toBeLessThan(300);
    const undo = boardUi().toasts().find((t) => t.action?.label === 'Undo')!;
    void undo.action!.run();
    await waitFor(() => expect(detail()).toBe('HP-3 details'));
    await waitFor(() => expect(dockNote().value).toBe('looks good, ship it'));
    await waitFor(() => expect(document.activeElement).toBe(dockNote()));
    expect(move).not.toHaveBeenCalled();
    expect(screen.getByLabelText('Open review (7)')).toBeTruthy();
  });

  it('closing the Workbench during the undo window saves the move', async () => {
    mount();
    await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.click(screen.getByRole('button', { name: /Mark done/ }));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    fireEvent.click(screen.getAllByLabelText('Close workbench')[0]!);
    await waitFor(() => expect(move).toHaveBeenCalledWith('pc-coffee', 'example-app', 'HP-3', 'done', undefined));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Workbench' })).toBeNull());
    fireEvent.click(await screen.findByLabelText(/Open review \(6\)/));
    await screen.findByLabelText('Open HP-5');
    expect(screen.queryByLabelText('Open HP-3')).toBeNull();
  });

  it('D marks done when no field has focus; in the field it types a letter', async () => {
    mount();
    const dialog = await openWorkbench();
    const move = vi.spyOn(api, 'move');
    dockNote().focus();
    fireEvent.keyDown(dockNote(), { key: 'd' });
    expect(isPending('example-app', 'HP-3')).toBe(false);
    dockNote().blur();
    screen.getByTestId('task-detail').focus();
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'd' });
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(isPending('example-app', 'HP-3')).toBe(true);
    await waitFor(() => expect(dialog.contains(document.activeElement) && document.activeElement === screen.getByTestId('task-detail')).toBe(true));
    expect(move).not.toHaveBeenCalled();
    await flushMoves();
  });

  it('R focuses the note field; Esc in the field leaves it and a second Esc closes the Workbench', async () => {
    mount();
    await openWorkbench();
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'r' });
    expect(document.activeElement).toBe(dockNote());
    fireEvent.keyDown(dockNote(), { key: 'Escape' });
    expect(document.activeElement).toBe(screen.getByTestId('task-detail'));
    expect(screen.getByRole('dialog', { name: 'Workbench' })).toBeTruthy();
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Workbench' })).toBeNull());
  });

  it('the note typed on one ticket survives ] then [', async () => {
    mount();
    await openWorkbench();
    fireEvent.input(dockNote(), { target: { value: 'half written' } });
    fireEvent.click(screen.getByLabelText('Next task'));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(dockNote().value).toBe('');
    fireEvent.click(screen.getByLabelText('Previous task'));
    await waitFor(() => expect(detail()).toBe('HP-3 details'));
    expect(dockNote().value).toBe('half written');
  });

  it('Ctrl+Enter comments and keeps the item; Ctrl+Shift+Enter reworks', async () => {
    mount();
    await openWorkbench();
    const note = vi.spyOn(api, 'note');
    fireEvent.input(dockNote(), { target: { value: 'a question' } });
    fireEvent.keyDown(dockNote(), { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(note).toHaveBeenCalledWith('pc-coffee', 'example-app', 'HP-3', 'a question'));
    await waitFor(() => expect(dockNote().value).toBe(''));
    expect(detail()).toBe('HP-3 details');
    expect(document.activeElement).toBe(dockNote());
    fireEvent.input(dockNote(), { target: { value: 'redo it' } });
    fireEvent.keyDown(dockNote(), { key: 'Enter', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(isPending('example-app', 'HP-3')).toBe(true);
    await flushMoves();
  });
});

describe('Workbench refinement: read the full report and reply together', () => {
  it('the full report opens in the side pane; description, attachments area and the reply field stay on screen', async () => {
    mount();
    const dialog = await openWorkbench();
    fireEvent.click(screen.getByRole('button', { name: 'Read the full report' }));
    const reader = await screen.findByTestId('report-reader');
    const pane = dialog.querySelector('.sa-review-pane')!;
    expect(pane.contains(reader)).toBe(true);
    expect(reader.textContent).toContain('Unit tests pass');
    // Nothing is mounted over the ticket: its page, the reply field and the pane are all in the document at once.
    expect(dialog.querySelector('.task-page .report-card')).toBeTruthy();
    expect(dockNote()).toBeTruthy();
    expect(dialog.querySelector('.task-page .reader-page')).toBeNull();
    // Typing while the pane shows keeps both.
    fireEvent.input(dockNote(), { target: { value: 'reply while reading' } });
    expect(screen.getByTestId('report-reader')).toBeTruthy();
    expect(dockNote().value).toBe('reply while reading');
  });

  it('O toggles the pane; clicking another ticket clears it', async () => {
    mount();
    const dialog = await openWorkbench();
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'o' });
    await screen.findByTestId('report-reader');
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'o' });
    expect(screen.queryByTestId('report-reader')).toBeNull();
    fireEvent.keyDown(screen.getByTestId('task-detail'), { key: 'o' });
    await screen.findByTestId('report-reader');
    fireEvent.click(within(dialog).getByLabelText('Open HP-5'));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    expect(screen.queryByTestId('report-reader')).toBeNull();
  });

  it('on the phone the report expands in place and the label becomes "Collapse report"', async () => {
    mount(true);
    fireEvent.click(await screen.findByLabelText('Open review (7)'));
    fireEvent.click(await screen.findByLabelText('Open HP-3'));
    const toggle = await screen.findByRole('button', { name: 'Read the full report' });
    fireEvent.click(toggle);
    expect(screen.queryByTestId('report-reader')).toBeNull();
    expect(toggle.textContent).toContain('Collapse report');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(screen.getByLabelText('Review report').hasAttribute('data-open')).toBe(true);
    expect(dockNote()).toBeTruthy();
  });
});

describe('Workbench refinement: empty Rework guard', () => {
  it('the first press asks, the second within 5 s moves the ticket', async () => {
    mount();
    await openWorkbench();
    const rework = () => screen.getByRole('button', { name: /^Rework$/ });
    fireEvent.click(rework());
    expect(screen.getByTestId('rework-hint').textContent).toBe('Add what should change, or press Rework again to send it back without a note.');
    expect(isPending('example-app', 'HP-3')).toBe(false);
    fireEvent.click(rework());
    await waitFor(() => expect(isPending('example-app', 'HP-3')).toBe(true));
    expect(detail()).toBe('HP-5 details');
    await flushMoves();
    expect(boardStore().taskById('HP-3')?.status ?? 'todo').toBe('todo');
  });
});

describe('Workbench refinement: media in rework notes', () => {
  it('pasting an image and dropping a PDF on the dock field make chips and store nothing until the reply is sent (P3-6r)', async () => {
    mount();
    await openWorkbench();
    const upload = vi.spyOn(api, 'uploadFile').mockImplementation(async (_h, _s, _t, file) => stored(file));
    const image = new File(['x'], 'shot.png', { type: 'image/png' });
    fireEvent.paste(dockNote(), { clipboardData: { types: [], getData: () => '', files: [image] } });
    // P3-1: the file is a chip; the field keeps the words only.
    await waitFor(() => expect(screen.getByLabelText('Remove shot.png')).toBeTruthy());
    expect(dockNote().value).toBe('');
    const pdf = new File(['%PDF'], 'spec.pdf', { type: 'application/pdf' });
    fireEvent.drop(dockNote().closest('.note-field')!, { dataTransfer: { types: ['Files'], files: [pdf] } });
    await waitFor(() => expect(screen.getByLabelText('Remove spec.pdf')).toBeTruthy());
    expect(upload).not.toHaveBeenCalled();
    expect(await screen.findByLabelText('Attached files')).toBeTruthy();
  });

  it('Rework with "fix X" + one image sends both as the move note', async () => {
    mount();
    await openWorkbench();
    vi.spyOn(api, 'uploadFile').mockImplementation(async (_h, _s, _t, file) => stored(file));
    const move = vi.spyOn(api, 'move');
    fireEvent.input(dockNote(), { target: { value: 'fix X' } });
    fireEvent.paste(dockNote(), { clipboardData: { types: [], getData: () => '', files: [new File(['x'], 'shot.png', { type: 'image/png' })] } });
    await waitFor(() => expect(screen.getByLabelText('Remove shot.png')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    await waitFor(() => expect(detail()).toBe('HP-5 details'));
    await flushMoves();
    const note = move.mock.calls.at(-1)![4];
    expect(move.mock.calls.at(-1)!.slice(2, 4)).toEqual(['HP-3', 'todo']);
    expect(note).toContain('fix X');
    expect(note).toContain('![shot.png](att:HP-3/ab12cd34-shot.png)');
  });
});

describe('Workbench refinement: media on the Board ticket page and in an escalation reply', () => {
  it('a Board ticket\'s composer takes a pasted image the same way', async () => {
    mount(false, '#/board');
    await waitFor(() => expect(boardStore().state.loaded).toBe(true), { timeout: 4000 });
    boardUi().openTask('HP-5');
    await screen.findByTestId('task-detail');
    vi.spyOn(api, 'uploadFile').mockImplementation(async (_h, _s, _t, file) => stored(file));
    const field = screen.getByLabelText('Add a note') as HTMLTextAreaElement;
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [new File(['x'], 'board.png', { type: 'image/png' })] } });
    await waitFor(() => expect(screen.getByLabelText('Remove board.png')).toBeTruthy());
    expect(await screen.findByLabelText('Attached files')).toBeTruthy();
  });

  it('an escalation reply takes a dropped PDF (uploaded to that ticket) and Ctrl+Enter sends it', async () => {
    const status = StatusSchema.parse({ enabled: true, leads: [], review: [], projects: [{ projectId: 'website', name: 'Website', autoStart: true }],
      needsYou: [{ projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain?' }] });
    vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: status });
    const relay = vi.spyOn(superagentApi, 'relay').mockResolvedValue({ ok: true, value: { commentId: null, sessionId: 'lead', delivered: true } });
    const upload = vi.spyOn(api, 'uploadFile').mockImplementation(async (_h, _s, _t, file) => stored(file));
    mount();
    fireEvent.click(await screen.findByLabelText('Open escalations (1)'));
    const box = await screen.findByLabelText('Reply to WEB-8') as HTMLTextAreaElement;
    const pdf = new File(['%PDF'], 'spec.pdf', { type: 'application/pdf' });
    fireEvent.drop(box.closest('.note-field')!, { dataTransfer: { types: ['Files'], files: [pdf] } });
    await waitFor(() => expect(screen.getByLabelText('Remove spec.pdf')).toBeTruthy());
    expect(upload.mock.calls[0]!.slice(1, 3)).toEqual(['website', 'WEB-8']);
    fireEvent.keyDown(box, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(relay).toHaveBeenCalled());
    expect(relay.mock.calls[0]![1].text).toContain('spec.pdf');
  });
});

describe('Workbench refinement: the Rework bug (sheets opened behind the Workbench)', () => {
  const css = readFileSync(resolve(__dirname, 'superagent.css'), 'utf8');
  const z = (selector: string) => Number(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{\\s*z-index:\\s*(\\d+)`).exec(css)?.[1]);
  const modal = Number(/\.sm-overlay\s*\{[^}]*z-index:\s*var\(--z-scrim,\s*(\d+)\)/.exec(readFileSync(resolve(__dirname, '../session/session-modal.css'), 'utf8'))?.[1]);

  it('while the Workbench is open the board sheets and the previewer are layered above its overlay', () => {
    expect(modal).toBe(60);
    expect(z('body:has(.sa-review-overlay) .sheet-overlay')).toBeGreaterThan(modal);
    expect(z('body:has(.sa-review-overlay) .sheet')).toBeGreaterThan(z('body:has(.sa-review-overlay) .sheet-overlay'));
    expect(z('body:has(.sa-review-overlay) :is(.image-viewer, .media-page, .media-pager)')).toBeGreaterThan(z('body:has(.sa-review-overlay) .sheet'));
  });

  it('Status → Done opens its confirm sheet from inside the Workbench (it mounts on <body>, outside the dialog)', async () => {
    mount();
    const dialog = await openWorkbench();
    fireEvent.click(screen.getByRole('button', { name: 'Status: In Review' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Done/ }));
    await waitFor(() => expect(document.querySelector('[data-testid=confirm-sheet]')).toBeTruthy());
    const sheet = document.querySelector('[data-testid=confirm-sheet]');
    expect(dialog.contains(sheet)).toBe(false); // a portal on <body>: only the z-index rule above puts it in front
    boardUi().setConfirm(null); boardUi().setComment(null);
  });
});

describe('Workbench refinement: header row', () => {
  it('has no native select; tabs are chips with counts; the project menu is searchable and newest activity first', async () => {
    const real = api.task.bind(api);
    const at: Record<string, string> = { 'example-app': '2026-11-01T10:00:00Z', oino: '2026-11-10T10:00:00Z', notes: '2026-11-05T10:00:00Z' };
    vi.spyOn(api, 'task').mockImplementation(async (h, slug, id) => ({ ...(await real(h, slug, id)), updated: at[slug]! }));
    mount();
    const dialog = await openWorkbench();
    expect(dialog.querySelector('select')).toBeNull();
    const tabs = within(dialog).getByRole('radiogroup', { name: 'Workbench sections' });
    expect([...tabs.querySelectorAll('.ph-chip')].map((c) => c.textContent)).toEqual(['Approve3', 'Review7', 'Escalate2']);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Project: All projects' }));
    const menu = await screen.findByRole('menu', { name: 'Filter by project' });
    await waitFor(() => expect([...menu.querySelectorAll('[role=menuitem]')].map((r) => r.querySelector('.truncate')?.textContent)).toEqual(['All projects', 'Oino', 'Field notes', 'ExampleApp']));
    expect(within(menu).getAllByRole('menuitem')[0]!.textContent).toContain('7');
    fireEvent.input(within(menu).getByLabelText('Search projects'), { target: { value: 'fie' } });
    expect([...menu.querySelectorAll('[role=menuitem]')].map((r) => r.querySelector('.truncate')?.textContent)).toEqual(['Field notes']);
    fireEvent.click(within(menu).getByRole('menuitem'));
    await waitFor(() => expect(within(dialog).getByRole('button', { name: 'Project: Field notes' })).toBeTruthy());
    await waitFor(() => expect(detail()).toBe('NOT-2 details'));
    expect(within(dialog).queryByLabelText('Open HP-3')).toBeNull();
    expect(within(dialog).getByLabelText('Search tasks').closest('.ph-search')).toBeTruthy();
  });
});
