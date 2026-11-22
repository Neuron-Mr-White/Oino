// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Attachment } from '../../lib/board/api';
import { setPlatformOverride } from '../../lib/platform';
import { NoteField } from './NoteField';

afterEach(() => { cleanup(); setPlatformOverride(undefined); });

const stored = (name: string, kind: Attachment['kind'] = 'image'): Attachment => ({
  name: `ab12cd34-${name}`, original: name, ref: `att:HP-3/ab12cd34-${name}`, path: `/x/${name}`, size: 10, mime: kind === 'image' ? 'image/png' : 'application/pdf', kind,
  markdown: kind === 'image' ? `![${name}](att:HP-3/ab12cd34-${name})` : `[${name}](att:HP-3/ab12cd34-${name})`,
});

function setup() {
  setPlatformOverride('tauri-desktop');
  const upload = vi.fn(async (files: File[]) => files.map((f) => stored(f.name, f.type.startsWith('image/') ? 'image' : 'pdf')));
  const thumb = vi.fn(async () => 'blob:thumb');
  let value = () => '';
  function Harness() {
    const [text, setText] = createSignal('fix X');
    const [busy, setBusy] = createSignal(0);
    value = text;
    return <NoteField taskId="HP-3" value={text()} onInput={setText} uploading={busy()} setUploading={setBusy} upload={upload} thumb={thumb} placeholder="Note" label="Add a note" />;
  }
  render(() => <Harness />);
  return { upload, thumb, value: () => value(), field: screen.getByLabelText('Add a note') as HTMLTextAreaElement };
}

describe('NoteField (shared by the Board composer, the review dock, the move note and the escalation reply)', () => {
  it('pasting an image uploads it, inserts its att: link and shows a thumbnail', async () => {
    const { upload, value, field } = setup();
    const file = new File(['x'], 'image.png', { type: 'image/png' });
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [file] } });
    await waitFor(() => expect(value()).toContain('att:HP-3/ab12cd34-image.png'));
    expect(upload).toHaveBeenCalledWith([file]);
    // P3-1: the field shows the words only — the link is a chip under it, at the end of the saved text.
    expect(value()).toBe('fix X\n![image.png](att:HP-3/ab12cd34-image.png)');
    expect(field.value).toBe('fix X');
    const preview = await screen.findByLabelText('Attached files');
    await waitFor(() => expect(preview.querySelector('img')?.getAttribute('src')).toBe('blob:thumb'));
  });

  it('dropping a PDF uploads it and shows a file chip; the zone lights up while dragging over it', async () => {
    const { upload, value, field } = setup();
    const zone = field.closest('.note-field') as HTMLElement;
    const file = new File(['%PDF'], 'spec.pdf', { type: 'application/pdf' });
    fireEvent.dragOver(zone, { dataTransfer: { types: ['Files'], files: [file] } });
    expect(zone.classList.contains('drop-over')).toBe(true);
    fireEvent.drop(zone, { dataTransfer: { types: ['Files'], files: [file] } });
    expect(zone.classList.contains('drop-over')).toBe(false);
    await waitFor(() => expect(value()).toContain('[spec.pdf](att:HP-3/ab12cd34-spec.pdf)'));
    expect(upload).toHaveBeenCalledWith([file]);
    expect((await screen.findByLabelText('Attached files')).textContent).toContain('spec.pdf');
  });

  it('plain text paste and a drag without files are left alone; a removed chip takes its link out of the note', async () => {
    const { upload, value, field } = setup();
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [] } });
    const zone = field.closest('.note-field') as HTMLElement;
    fireEvent.drop(zone, { dataTransfer: { types: ['text/plain'], files: [] } });
    expect(upload).not.toHaveBeenCalled();
    fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [new File(['x'], 'a.png', { type: 'image/png' })] } });
    await screen.findByLabelText('Attached files');
    expect(field.value).toBe('fix X');
    fireEvent.click(screen.getByLabelText('Remove a.png'));
    await waitFor(() => expect(screen.queryByLabelText('Attached files')).toBeNull());
    expect(value()).toBe('fix X');
  });
});
