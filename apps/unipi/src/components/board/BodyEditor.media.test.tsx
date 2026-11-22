// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { BodyEditor } from './BodyEditor';
import { api, type Attachment } from '../../lib/board/api';
import { setBoardTransport } from '../../lib/board/transport';
import { fakeBoard, task } from '../../lib/board/testing';
import { boardStore, resetBoardStore } from '../../lib/board/store';
import { setPlatformOverride } from '../../lib/platform';

afterEach(() => { cleanup(); vi.restoreAllMocks(); setBoardTransport(undefined); setPlatformOverride(undefined); resetBoardStore(); });

const stored = (file: File): Attachment => ({
  name: `ab12cd34-${file.name}`, original: file.name, ref: `att:A-1/ab12cd34-${file.name}`, path: `/x/${file.name}`, size: 1, mime: file.type,
  kind: file.type.startsWith('image/') ? 'image' : 'pdf',
  markdown: `${file.type.startsWith('image/') ? '!' : ''}[${file.name}](att:A-1/ab12cd34-${file.name})`,
});

async function mount() {
  setPlatformOverride('tauri-desktop');
  setBoardTransport(fakeBoard({ handlers: { 'list --all': () => ({ tasks: [], problems: [] }) } }).transport);
  await boardStore().open('pc', 'demo');
  const upload = vi.spyOn(api, 'uploadFile').mockImplementation(async (_h, _s, _id, file) => stored(file));
  const saved: string[] = [];
  let value = () => '';
  function Harness() {
    const [text, setText] = createSignal('intro outro');
    const [busy, setBusy] = createSignal(0);
    value = text;
    return <BodyEditor task={task({ id: 'A-1' })} value={text()} onInput={setText} dirty busy={false} onSave={() => saved.push(text())} onCancel={() => {}} uploading={busy()} setUploading={setBusy} />;
  }
  render(() => <Harness />);
  const field = screen.getByLabelText('Description (markdown)') as HTMLTextAreaElement;
  return { field, upload, saved, value: () => value() };
}

describe('description editor: paste and drop media (OIN-95)', () => {
  it('a pasted image is a chip under the field, not markdown in it; nothing is stored until Save (P3-6)', async () => {
    const { field, upload, value } = await mount();
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [new File(['x'], 'shot.png', { type: 'image/png' })] } });
    await waitFor(() => expect(screen.getByLabelText('Remove shot.png')).toBeTruthy());
    expect(field.value).toBe('intro outro');
    expect(value()).toBe('intro outro\n![shot.png](pending:1)');
    expect(upload).not.toHaveBeenCalled();
  });

  it('Save stores the held file on the task first and saves the text with its link', async () => {
    const { field, upload, saved } = await mount();
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [new File(['x'], 'shot.png', { type: 'image/png' })] } });
    await waitFor(() => expect(screen.getByLabelText('Remove shot.png')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(upload).toHaveBeenCalledTimes(1);
    expect(saved[0]).toBe('intro outro\n![shot.png](att:A-1/ab12cd34-shot.png)');
  });

  it('removing the chip takes the link out and Save then uploads nothing', async () => {
    const { field, upload, saved, value } = await mount();
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [new File(['x'], 'shot.png', { type: 'image/png' })] } });
    fireEvent.click(await screen.findByLabelText('Remove shot.png'));
    expect(value()).toBe('intro outro');
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(saved).toEqual(['intro outro']));
    expect(upload).not.toHaveBeenCalled();
  });

  it('a dropped PDF is held as a file chip; the field lights up while dragging over it', async () => {
    const { field, upload, value } = await mount();
    const files = [new File(['%PDF'], 'spec.pdf', { type: 'application/pdf' })];
    fireEvent.dragOver(field, { dataTransfer: { types: ['Files'], files } });
    expect(field.classList.contains('drop-over')).toBe(true);
    fireEvent.drop(field, { dataTransfer: { types: ['Files'], files } });
    await waitFor(() => expect(value()).toContain('[spec.pdf](pending:1)'));
    expect(screen.getByLabelText('Remove spec.pdf')).toBeTruthy();
    expect(field.classList.contains('drop-over')).toBe(false);
    expect(upload).not.toHaveBeenCalled();
  });

  it('links already closing a saved description show as chips, not text', async () => {
    setPlatformOverride('tauri-desktop');
    setBoardTransport(fakeBoard({ handlers: { 'list --all': () => ({ tasks: [], problems: [] }) } }).transport);
    await boardStore().open('pc', 'demo');
    render(() => <BodyEditor task={task({ id: 'A-1' })} value={'notes\n![a.png](att:A-1/ab12cd34-a.png)'} onInput={() => {}} dirty={false} busy={false} onSave={() => {}} onCancel={() => {}} uploading={0} setUploading={() => {}} />);
    expect((screen.getByLabelText('Description (markdown)') as HTMLTextAreaElement).value).toBe('notes');
    expect(screen.getByLabelText('Remove a.png')).toBeTruthy();
  });

  it('plain text paste and a text drag are not taken', async () => {
    const { field, upload } = await mount();
    fireEvent.paste(field, { clipboardData: { types: ['text/plain'], getData: () => 'hello', files: [] } });
    fireEvent.drop(field, { dataTransfer: { types: ['text/plain'], files: [] } });
    expect(upload).not.toHaveBeenCalled();
  });
});
