import { describe, expect, it } from 'vitest';
import { classifyPaste, dropPoint, isInlineImage, mimeForName, planLocalFiles, type PasteData } from './paste';

function dt(data: Record<string, string>, files: File[] = []): PasteData {
  return { types: Object.keys(data), getData: (t: string) => data[t] ?? '', files };
}

describe('classifyPaste (UNI-256)', () => {
  const png = new File([new Uint8Array(4)], 'shot.png', { type: 'image/png' });

  it('files on the event are attached (Chromium / WebView2 screenshot)', () => {
    expect(classifyPaste(dt({ 'text/plain': 'x' }, [png]), true)).toEqual({ kind: 'files', files: [png] });
    expect(classifyPaste(dt({}, [png]), false)).toEqual({ kind: 'files', files: [png] });
  });

  it('plain text is pasted as text (CRLF normalised)', () => {
    expect(classifyPaste(dt({ 'text/plain': 'a\r\nb\rc' }), true)).toEqual({ kind: 'text', text: 'a\nb\nc' });
    expect(classifyPaste(dt({ 'text/plain': 'hi' }), false)).toEqual({ kind: 'text', text: 'hi' });
  });

  it('an EMPTY event (WebKitGTK with a clipboard image) asks the core in the desktop app', () => {
    expect(classifyPaste(dt({}), true)).toEqual({ kind: 'native', text: '' });
    expect(classifyPaste(null, true)).toEqual({ kind: 'native', text: '' });
    // Web / phone: nothing to do.
    expect(classifyPaste(dt({}), false)).toEqual({ kind: 'none' });
    expect(classifyPaste(undefined, false)).toEqual({ kind: 'none' });
  });

  it('files copied in the file manager (uri list, unreadable) go native, keeping any text as the fallback', () => {
    expect(classifyPaste(dt({ 'text/uri-list': '' }), true)).toEqual({ kind: 'native', text: '' });
    expect(classifyPaste(dt({ 'x-special/gnome-copied-files': '', 'text/plain': '/tmp/a.png' }), true)).toEqual({ kind: 'native', text: '/tmp/a.png' });
    // Without the core the path text is still pasted (browser).
    expect(classifyPaste(dt({ 'text/uri-list': 'file:///tmp/a.png', 'text/plain': '/tmp/a.png' }), false)).toEqual({ kind: 'text', text: '/tmp/a.png' });
  });
});

describe('local files (drop / copied files)', () => {
  it('mime by extension', () => {
    expect(mimeForName('Screenshot From 2026.PNG')).toBe('image/png');
    expect(mimeForName('a.jpeg')).toBe('image/jpeg');
    expect(mimeForName('notes.md')).toBe('text/markdown');
    expect(mimeForName('Makefile')).toBe('application/octet-stream');
  });

  it('raster images go inline; svg/heic go as files', () => {
    expect(isInlineImage('a.png')).toBe(true);
    expect(isInlineImage('a.webp')).toBe(true);
    expect(isInlineImage('a.svg')).toBe(false);
    expect(isInlineImage('a.heic')).toBe(false);
    expect(isInlineImage('a.pdf')).toBe(false);
  });

  it('folders and too-big non-images are refused with a reason; big images still go (they are shrunk)', () => {
    const f = (name: string, size: number, isDir = false) => ({ path: `/x/${name}`, name, size, isDir });
    const plan = planLocalFiles([f('a.png', 50_000_000), f('b.txt', 10), f('dir', 0, true), f('big.zip', 20_000_000)], 10 * 1024 * 1024);
    expect(plan.read.map((x) => x.name)).toEqual(['a.png', 'b.txt']);
    expect(plan.refused.map((x) => x.file.name)).toEqual(['dir', 'big.zip']);
    expect(plan.refused[0]!.reason).toMatch(/Folders/);
    expect(plan.refused[1]!.reason).toBe('big.zip is larger than 10 MB');
  });

  it('drop position: physical px → CSS px', () => {
    expect(dropPoint({ x: 200, y: 100 }, 2)).toEqual({ x: 100, y: 50 });
    expect(dropPoint({ x: 200, y: 100 }, 0)).toEqual({ x: 200, y: 100 });
  });
});
