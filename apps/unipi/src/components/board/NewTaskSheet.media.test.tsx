// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { NewTaskSheet } from './NewTaskSheet';
import { api, type Attachment } from '../../lib/board/api';
import { setBoardTransport } from '../../lib/board/transport';
import { fakeBoard, task } from '../../lib/board/testing';
import { boardStore, resetBoardStore } from '../../lib/board/store';
import { boardUi, resetBoardUi } from '../../lib/board/ui';
import { setPlatformOverride } from '../../lib/platform';

const stored = (taskId: string, file: File, n: number): Attachment => ({
  name: `0000000${n}-${file.name}`, original: file.name, ref: `att:${taskId}/0000000${n}-${file.name}`, path: `/x/${file.name}`, size: 1, mime: file.type,
  kind: file.type.startsWith('image/') ? 'image' : 'pdf',
  markdown: `${file.type.startsWith('image/') ? '!' : ''}[${file.name}](att:${taskId}/0000000${n}-${file.name})`,
});
const clip = (...files: File[]) => ({ clipboardData: { types: [], getData: () => '', files } });
const png = (name: string) => new File(['x'], name, { type: 'image/png' });

async function open() {
  setPlatformOverride('tauri-desktop');
  setBoardTransport(fakeBoard({ handlers: { 'list --all': () => ({ tasks: [], problems: [] }) } }).transport);
  await boardStore().open('pc', 'demo');
  render(() => <NewTaskSheet />);
  boardUi().openNewTask({ lane: 'backlog' });
  return (await screen.findByLabelText('Description')) as HTMLTextAreaElement;
}
const type = (field: HTMLTextAreaElement, value: string, caret = value.length) => {
  fireEvent.input(field, { target: { value } });
  field.setSelectionRange(caret, caret);
};

beforeEach(() => { vi.spyOn(console, 'warn').mockImplementation(() => {}); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); setBoardTransport(undefined); setPlatformOverride(undefined); resetBoardStore(); resetBoardUi(); });

describe('new task sheet: paste and drop media into the description (OIN-95)', () => {
  it('a pasted image becomes a chip, not markdown in the field; plain paste is left to the field', async () => {
    const field = await open();
    type(field, 'before after', 6);
    fireEvent.paste(field, { clipboardData: { types: ['text/plain'], getData: () => 'just text', files: [] } });
    expect(field.value).toBe('before after');
    fireEvent.paste(field, clip(png('image.png')));
    // D2: the image is a chip under the description (the chat composer's attachment chip); the field keeps the words only.
    await waitFor(() => expect(screen.getByLabelText(/^Remove screenshot-/)).toBeTruthy());
    expect(field.value).toBe('before after');
    expect(screen.getByTestId('draft-chip').dataset.kind).toBe('image');
  });

  it('a dropped file is a chip too, and removing it takes its reference out of the text', async () => {
    const field = await open();
    type(field, 'notes');
    fireEvent.drop(field, { dataTransfer: { types: ['Files'], files: [new File(['%PDF'], 'spec.pdf', { type: 'application/pdf' })] } });
    await waitFor(() => expect(screen.getByLabelText('Remove spec.pdf')).toBeTruthy());
    expect(field.value).toBe('notes');
    fireEvent.click(screen.getByLabelText('Remove spec.pdf'));
    expect(field.value).toBe('notes');
    expect(screen.queryByLabelText('Remove spec.pdf')).toBeNull();
  });

  it('Create uploads each file to the new task and swaps its placeholder for the stored markdown', async () => {
    const field = await open();
    const created = task({ id: 'N-1', body: 'ignored' });
    const create = vi.spyOn(api, 'create').mockResolvedValue(created);
    const upload = vi.spyOn(api, 'uploadFile').mockImplementation(async (_h, _s, id, file) => stored(id, file, upload.mock.calls.length));
    const edit = vi.spyOn(api, 'edit').mockImplementation(async (_h, _s, _id, patch) => ({ ...created, body: patch.body }));
    type(field, 'see');
    fireEvent.paste(field, clip(png('a.png')));
    await waitFor(() => expect(screen.getByLabelText('Remove a.png')).toBeTruthy());
    type(field, `${field.value}\nthen`);
    fireEvent.paste(field, clip(png('b.png')));
    await waitFor(() => expect(screen.getByLabelText('Remove b.png')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));
    await waitFor(() => expect(edit).toHaveBeenCalled());
    expect(create.mock.calls[0]![2].body).toBe('see\nthen');
    expect(upload.mock.calls.map((call) => (call[3] as File).name)).toEqual(['a.png', 'b.png']);
    expect(edit.mock.calls[0]![3].body).toBe('see\nthen\n![a.png](att:N-1/00000001-a.png)\n![b.png](att:N-1/00000002-b.png)');
  });

  it('an image-only description is not a task yet (a title or text is still needed)', async () => {
    const field = await open();
    const create = vi.spyOn(api, 'create');
    fireEvent.paste(field, clip(png('only.png')));
    await waitFor(() => expect(screen.getByLabelText('Remove only.png')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Create task' }));
    expect(create).not.toHaveBeenCalled();
  });
});
