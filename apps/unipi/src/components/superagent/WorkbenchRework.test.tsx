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

describe('Workbench: Rework reply field', () => {
  it('Rework opens the inline reply field (focused, with a hint) and moves nothing yet', async () => {
    mount();
    const dialog = await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    expect(document.activeElement).toBe(dockNote());
    expect(within(dialog).getByTestId('rework-hint')).toBeTruthy();
    expect(screen.queryByTestId('comment-sheet')).toBeNull();
    expect(isPending('example-app', 'HP-3')).toBe(false);
    expect(move).not.toHaveBeenCalled();
  });

  it('submitting sends the typed text as the rework comment; the buttons use the shared .btn classes', async () => {
    mount();
    await openWorkbench();
    const rework = screen.getByRole('button', { name: /^Rework$/ });
    expect(rework.classList.contains('btn')).toBe(true);
    expect(screen.getByRole('button', { name: /Mark done/ }).classList.contains('btn-primary')).toBe(true);
    const move = vi.spyOn(api, 'move');
    fireEvent.click(rework);
    fireEvent.input(dockNote(), { target: { value: 'use the debounce helper' } });
    expect(screen.queryByTestId('rework-hint')).toBeNull();
    fireEvent.click(rework);
    await waitFor(() => expect(isPending('example-app', 'HP-3')).toBe(true));
    await flushMoves();
    expect(move).toHaveBeenCalledWith('pc-coffee', 'example-app', 'HP-3', 'todo', 'use the debounce helper');
  });

  it('Esc cancels the pending Rework: the hint goes, focus leaves the field, nothing is sent', async () => {
    mount();
    const dialog = await openWorkbench();
    const move = vi.spyOn(api, 'move');
    fireEvent.click(screen.getByRole('button', { name: /^Rework$/ }));
    expect(within(dialog).getByTestId('rework-hint')).toBeTruthy();
    fireEvent.keyDown(dockNote(), { key: 'Escape' });
    expect(within(dialog).queryByTestId('rework-hint')).toBeNull();
    expect(document.activeElement).not.toBe(dockNote());
    expect(screen.getByRole('dialog', { name: 'Workbench' })).toBeTruthy();
    expect(isPending('example-app', 'HP-3')).toBe(false);
    expect(move).not.toHaveBeenCalled();
  });
});
