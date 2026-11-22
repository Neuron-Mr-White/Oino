// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Attachment } from '../../lib/board/api';
import { setPlatformOverride } from '../../lib/platform';
import { task } from '../../lib/board/testing';
import { releaseHeld } from '../../lib/board/heldFiles';

const actions = vi.hoisted(() => ({
  addNote: vi.fn(async () => true),
  moveTask: vi.fn(async () => true),
  uploadToTask: vi.fn(async (_task: unknown, files: File[]) => files.map((f) => ({
    name: `ab12-${f.name}`, original: f.name, ref: `att:KBQ-2/ab12-${f.name}`, path: '', size: 10, mime: 'image/png', kind: 'image',
    markdown: `![${f.name}](att:KBQ-2/ab12-${f.name})`,
  }) as Attachment)),
  boardCtx: vi.fn(() => undefined),
}));
vi.mock('../../lib/board/taskActions', () => actions);

import { ReviewDock } from './ReviewDock';

const review = task({ id: 'KBQ-2', status: 'in_review', allowedMoves: ['todo', 'done'] });

function setup(initial = '') {
  setPlatformOverride('tauri-desktop');
  let note = () => '';
  function Harness() {
    const [text, setText] = createSignal(initial);
    const [busy, setBusy] = createSignal(0);
    note = text;
    return <ReviewDock task={review} kind="review" note={text()} onNote={setText} uploading={busy()} setUploading={setBusy} />;
  }
  render(() => <Harness />);
  const field = screen.getByLabelText('Add a note') as HTMLTextAreaElement;
  const paste = (name: string) => fireEvent.paste(field, { clipboardData: { types: [], getData: () => '', files: [new File(['px'], name, { type: 'image/png' })] } });
  return { field, note: () => note(), paste };
}

beforeEach(() => { URL.createObjectURL = vi.fn(() => 'blob:held'); });
afterEach(() => {
  cleanup(); setPlatformOverride(undefined); releaseHeld('/KBQ-2');
  for (const fn of Object.values(actions)) fn.mockClear();
});

describe('the review dock stores a pasted image only when the reply is sent (P3-6r)', () => {
  it('pasting shows a chip and uploads nothing', async () => {
    const { paste, note } = setup();
    paste('shot.png');
    const chips = await screen.findByLabelText('Attached files');
    expect(chips.textContent).toContain('shot.png');
    expect(note()).toMatch(/\(pending:held-\d+\)/);
    expect(actions.uploadToTask).not.toHaveBeenCalled();
  });

  it('removing the chip leaves nothing stored, and a later send does not upload it', async () => {
    const { field, paste, note } = setup();
    paste('shot.png');
    await screen.findByLabelText('Attached files');
    fireEvent.click(screen.getByLabelText('Remove shot.png'));
    await waitFor(() => expect(screen.queryByLabelText('Attached files')).toBeNull());
    expect(note()).toBe('');
    fireEvent.input(field, { target: { value: 'just words' } });
    fireEvent.click(screen.getByRole('button', { name: /Comment/ }));
    await waitFor(() => expect(actions.addNote).toHaveBeenCalledWith(review, 'just words'));
    expect(actions.uploadToTask).not.toHaveBeenCalled();
  });

  it('Comment stores the file, then sends the note with its real link', async () => {
    const { field, paste } = setup();
    paste('shot.png');
    await screen.findByLabelText('Attached files');
    fireEvent.input(field, { target: { value: 'see this' } });
    fireEvent.click(screen.getByRole('button', { name: /Comment/ }));
    await waitFor(() => expect(actions.addNote).toHaveBeenCalledTimes(1));
    expect(actions.uploadToTask).toHaveBeenCalledTimes(1);
    expect((actions.uploadToTask.mock.calls[0]![1] as File[])[0]!.name).toBe('shot.png');
    expect(actions.addNote).toHaveBeenCalledWith(review, 'see this\n![shot.png](att:KBQ-2/ab12-shot.png)');
  });

  it('Rework stores the file and sends the note with its real link as the move\'s comment', async () => {
    const { paste } = setup();
    paste('shot.png');
    await screen.findByLabelText('Attached files');
    fireEvent.click(screen.getByRole('button', { name: /Rework/ }));
    await waitFor(() => expect(actions.moveTask).toHaveBeenCalledTimes(1));
    expect(actions.uploadToTask).toHaveBeenCalledTimes(1);
    expect(actions.moveTask).toHaveBeenCalledWith(review, 'todo', '![shot.png](att:KBQ-2/ab12-shot.png)', { confirmed: true });
  });
});
