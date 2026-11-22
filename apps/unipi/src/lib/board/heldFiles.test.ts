import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Attachment } from './api';
import { commitHeld, heldSize, heldThumb, holdFiles, releaseHeld } from './heldFiles';

const KEY = 'demo/KBQ-2';
const png = (name: string) => new File(['x'.repeat(10)], name, { type: 'image/png' });
const stored = (file: File): Attachment => ({ name: `ab12-${file.name}`, original: file.name, ref: `att:KBQ-2/ab12-${file.name}`, path: '', size: 10, mime: 'image/png', kind: 'image', markdown: `![${file.name}](att:KBQ-2/ab12-${file.name})` });

afterEach(() => releaseHeld(KEY));

describe('files held for a reply that is not sent yet (P3-6r)', () => {
  it('holding stores nothing: the chips are pending links, with a size and a preview of the held file', () => {
    URL.createObjectURL = vi.fn(() => 'blob:held');
    const [ref] = holdFiles(KEY, [png('shot.png')]);
    expect(ref!.target).toMatch(/^pending:held-\d+$/);
    expect(ref!.image).toBe(true);
    expect(heldSize(KEY, ref!)).toBe(10);
    expect(heldThumb(KEY, ref!)).toBe('blob:held');
    expect(heldThumb('other/KEY', ref!)).toBeUndefined();
  });

  it('sending uploads only the files the text still names, in order, and swaps in the stored links', async () => {
    const [a, b, c] = holdFiles(KEY, [png('a.png'), png('b.png'), png('c.png')]);
    // b's chip was removed: its link is no longer in the text.
    const body = `fix X\n${a!.markdown}\n${c!.markdown}`;
    const upload = vi.fn(async (file: File) => stored(file));
    const out = await commitHeld(KEY, body, upload);
    expect(upload.mock.calls.map(([file]) => file.name)).toEqual(['a.png', 'c.png']);
    expect(out).toBe('fix X\n![a.png](att:KBQ-2/ab12-a.png)\n![c.png](att:KBQ-2/ab12-c.png)');
    expect(heldSize(KEY, b!)).toBeUndefined(); // released with everything else
  });

  it('a note whose chips were all removed uploads nothing', async () => {
    const [a] = holdFiles(KEY, [png('a.png')]);
    const upload = vi.fn(async (file: File) => stored(file));
    expect(await commitHeld(KEY, 'fix X', upload)).toBe('fix X');
    expect(upload).not.toHaveBeenCalled();
    expect(heldSize(KEY, a!)).toBeUndefined();
  });

  it('a file that fails to upload is dropped from the text, the rest still go', async () => {
    const [a, b] = holdFiles(KEY, [png('a.png'), png('b.png')]);
    const out = await commitHeld(KEY, `note\n${a!.markdown}\n${b!.markdown}`, async (file) => (file.name === 'a.png' ? undefined : stored(file)));
    expect(out).toBe('note\n![b.png](att:KBQ-2/ab12-b.png)');
  });
});
