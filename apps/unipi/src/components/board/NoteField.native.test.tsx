// @vitest-environment jsdom
// UNI-256 on the note field: WebKitGTK (the installed Linux app) gives the page no File for an OS drop and an
// empty paste event for a clipboard image, so the core reads them (lib/localFiles) — as the chat composer does.
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Attachment } from '../../lib/board/api';
import { setPlatformOverride } from '../../lib/platform';
import type { DropZone } from '../../lib/localFiles';
import { NoteField } from './NoteField';

const zones: DropZone[] = [];
const off = vi.fn();
const local = { clipboard: [] as Array<{ path: string; name: string; size: number; isDir: boolean }> };
vi.mock('../../lib/localFiles', () => ({
  clipboardFiles: vi.fn(async () => local.clipboard),
  describeLocal: vi.fn(async (paths: string[]) => paths.map((path) => ({ path, name: path.split('/').pop()!, size: 4, isDir: path.endsWith('/dir') }))),
  readLocalFiles: vi.fn(async (files: Array<{ name: string }>) => files.map((f) => new File(['x'], f.name, { type: f.name.endsWith('.png') ? 'image/png' : 'application/pdf' }))),
  registerDropZone: vi.fn((zone: DropZone) => { zones.push(zone); return off; }),
  startNativeDrop: vi.fn(async () => {}),
}));

beforeEach(() => { zones.length = 0; off.mockClear(); local.clipboard = []; setPlatformOverride('tauri-desktop'); });
afterEach(() => { cleanup(); setPlatformOverride(undefined); });

const stored = (name: string): Attachment => ({
  name: `ab12cd34-${name}`, original: name, ref: `att:HP-3/ab12cd34-${name}`, path: `/x/${name}`, size: 10, mime: 'image/png', kind: name.endsWith('.png') ? 'image' : 'pdf',
  markdown: `[${name}](att:HP-3/ab12cd34-${name})`,
});

function setup() {
  const upload = vi.fn(async (files: File[]) => files.map((f) => stored(f.name)));
  let value = () => '';
  function Harness() {
    const [text, setText] = createSignal('');
    const [busy, setBusy] = createSignal(0);
    value = text;
    return <NoteField taskId="HP-3" value={text()} onInput={setText} uploading={busy()} setUploading={setBusy} upload={upload} thumb={async () => undefined} placeholder="Note" label="Add a note" />;
  }
  render(() => <Harness />);
  return { upload, value: () => value(), field: screen.getByLabelText('Add a note') as HTMLTextAreaElement };
}

/** A paste event the way WebKitGTK shows a screenshot on the clipboard: nothing in it. */
const emptyPaste = (field: HTMLElement, text = '') => fireEvent.paste(field, { clipboardData: { types: text ? ['text/plain'] : [], getData: (t: string) => (t === 'text/plain' ? text : ''), files: [] } });

describe('NoteField in the desktop app (native clipboard and drop)', () => {
  it('an empty paste event asks the core for the clipboard image and uploads it', async () => {
    const { upload, value, field } = setup();
    local.clipboard = [{ path: '/tmp/screenshot.png', name: 'screenshot.png', size: 9, isDir: false }];
    emptyPaste(field);
    await waitFor(() => expect(value()).toContain('att:HP-3/ab12cd34-screenshot.png'));
    expect(upload.mock.calls[0]![0].map((f) => f.name)).toEqual(['screenshot.png']);
  });

  it('copied files (a uri-list the page cannot read) are fetched natively too', async () => {
    const { upload, field } = setup();
    local.clipboard = [{ path: '/home/u/spec.pdf', name: 'spec.pdf', size: 9, isDir: false }];
    fireEvent.paste(field, { clipboardData: { types: ['text/uri-list'], getData: () => '', files: [] } });
    await waitFor(() => expect(upload).toHaveBeenCalled());
    expect(upload.mock.calls[0]![0][0]!.name).toBe('spec.pdf');
  });

  it('an empty clipboard keeps the pasted text; a text paste is left to the browser', async () => {
    const { upload, value, field } = setup();
    fireEvent.paste(field, { clipboardData: { types: ['text/plain'], getData: () => 'plain words', files: [] } });
    expect(upload).not.toHaveBeenCalled();
    local.clipboard = [];
    // Copied-files event that yields nothing: the text it carried is pasted instead.
    fireEvent.paste(field, { clipboardData: { types: ['text/uri-list', 'text/plain'], getData: (t: string) => (t === 'text/plain' ? 'file:///gone' : ''), files: [] } });
    await waitFor(() => expect(value()).toContain('file:///gone'));
    expect(upload).not.toHaveBeenCalled();
  });

  it('registers the field as a native drop zone; a drop of paths reads the files and uploads them', async () => {
    const { upload, value, field } = setup();
    const wrap = field.closest('.note-field') as HTMLElement;
    expect(zones).toHaveLength(1);
    expect(zones[0]!.el()).toBe(wrap);
    zones[0]!.hover(true);
    expect(wrap.classList.contains('drop-over')).toBe(true);
    zones[0]!.hover(false);
    expect(wrap.classList.contains('drop-over')).toBe(false);
    zones[0]!.drop(['/home/u/shot.png', '/home/u/dir']);
    await waitFor(() => expect(value()).toContain('att:HP-3/ab12cd34-shot.png'));
    // The folder is skipped, not uploaded.
    expect(upload.mock.calls[0]![0].map((f) => f.name)).toEqual(['shot.png']);
  });

  it('the zone is enabled while the field is, and goes with it', () => {
    setup();
    expect(zones[0]!.enabled?.()).toBe(true);
    cleanup();
    expect(off).toHaveBeenCalled();
  });

  it('the web build registers nothing and pastes natively only where the core exists', async () => {
    cleanup(); zones.length = 0;
    setPlatformOverride('web');
    const { upload, field } = setup();
    expect(zones).toHaveLength(0);
    emptyPaste(field);
    await new Promise((r) => setTimeout(r, 20));
    expect(upload).not.toHaveBeenCalled();
  });
});
