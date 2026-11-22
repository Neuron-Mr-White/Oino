import { describe, it, expect } from 'vitest';
import { classifyMedia, languageOf } from './previewer';

describe('classifyMedia', () => {
  it('routes by mime first', () => {
    expect(classifyMedia('image/png')).toBe('image');
    expect(classifyMedia('image/webp')).toBe('image');
    expect(classifyMedia('application/pdf')).toBe('pdf');
    expect(classifyMedia('audio/mpeg')).toBe('audio');
    expect(classifyMedia('video/mp4')).toBe('video');
    expect(classifyMedia('text/markdown')).toBe('markdown');
  });

  it('falls back to extension for generic/missing mime', () => {
    expect(classifyMedia('application/octet-stream', '/tmp/report.md')).toBe('markdown');
    expect(classifyMedia(undefined, '/tmp/build.log')).toBe('log');
    expect(classifyMedia('application/octet-stream', '/tmp/notes.txt')).toBe('text');
    expect(classifyMedia('application/octet-stream', '/tmp/main.rs')).toBe('code');
    expect(classifyMedia('application/octet-stream', '/tmp/doc.pdf')).toBe('pdf');
  });

  it('json/code mimes route to the code viewer', () => {
    expect(classifyMedia('application/json')).toBe('code');
    expect(classifyMedia('text/x-python')).toBe('code');
  });

  it('unknown binary with no extension is the no-preview card (UNI-169), never garbage text', () => {
    expect(classifyMedia('application/octet-stream')).toBe('other');
    expect(classifyMedia(undefined)).toBe('other');
  });

  it('unknown extension with a text-ish mime is plain text', () => {
    expect(classifyMedia('text/plain', '/tmp/weird.xyz')).toBe('text');
  });
});

describe('languageOf', () => {
  it('returns the lowercase extension', () => {
    expect(languageOf('/a/b/main.RS')).toBe('rs');
    expect(languageOf('script.py')).toBe('py');
  });
  it('undefined with no extension', () => {
    expect(languageOf('Makefile')).toBeUndefined();
  });
});

describe('classifyMedia open-with kinds (UNI-169)', () => {
  it('office documents, archives and packages are "other", not garbage text', () => {
    const paths = ['/a/report.docx', '/b/book.xlsx', '/c/deck.pptx', '/d/notes.odt', '/e/bundle.zip', '/f/app.apk', '/g/lib.so'];
    for (const path of paths) {
      expect([path, classifyMedia('application/octet-stream', path)]).toEqual([path, 'other']);
    }
  });
  it('opaque binary with no known extension is "other"', () => {
    expect(classifyMedia('application/octet-stream', '/x/data-2024')).toBe('other');
    expect(classifyMedia(undefined, '/x/data-2024')).toBe('other');
  });
  it('text and code still classify as before', () => {
    expect(classifyMedia('text/plain', '/a/readme.txt')).toBe('text');
    expect(classifyMedia('application/octet-stream', '/a/main.ts')).toBe('code');
    expect(classifyMedia('text/plain', '/a/notes.md')).toBe('markdown');
  });
});

describe('isDirectPreview: unknown extensions are sniffed, not routed to no-preview (owner5 #23)', () => {
  it.each(['/p/Cargo.lock', '/p/.env', '/p/x.foo', '/p/README'])('%s opens in the editor', async (path) => {
    const { isDirectPreview } = await import('./previewer');
    expect(isDirectPreview(path)).toBe(false);
  });
  it.each(['/p/a.png', '/p/a.zip', '/p/a.pdf', '/p/a.mp4'])('%s stays a direct preview', async (path) => {
    const { isDirectPreview } = await import('./previewer');
    expect(isDirectPreview(path)).toBe(true);
  });
});

import { classifyBytes, looksLikeText } from './previewer';
describe('byte sniff (dotfiles / unknown text types)', () => {
  const enc = (s: string) => new TextEncoder().encode(s);
  it('a .env with an octet-stream mime renders as text', () => {
    expect(classifyBytes('application/octet-stream', '/p/.env', enc('API_KEY=abc\nDEBUG=1\n'))).toBe('text');
    expect(classifyBytes(undefined, '/p/Makefile', enc('all:\n\techo hi\n'))).toBe('text');
  });
  it('real binaries stay no-preview', () => {
    expect(looksLikeText(new Uint8Array([0x7f, 0x45, 0x4c, 0x46, 0, 1, 2]))).toBe(false);
    expect(classifyBytes('application/octet-stream', '/p/blob', new Uint8Array([0, 1, 2, 3]))).toBe('other');
    expect(classifyBytes(undefined, '/p/a.zip', enc('PK text-looking'))).toBe('other');
  });
});
