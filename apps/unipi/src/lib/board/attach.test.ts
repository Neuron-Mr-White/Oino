import { describe, expect, it } from 'vitest';
import { insertAtCursor, namedFile, pendingMarkdown, resolvePending, stripPending } from './attach';

const png = (name = 'shot.png') => new File(['x'], name, { type: 'image/png' });

describe('namedFile', () => {
  it('gives a pasted screenshot a readable name and keeps a real one', () => {
    expect(namedFile(png('image.png'), new Date('2026-11-13T09:05:00Z')).name).toBe('screenshot-2026-11-13-0905.png');
    expect(namedFile(png('design.png')).name).toBe('design.png');
  });
});

describe('insertAtCursor', () => {
  it('puts the snippet on its own line at the cursor', () => {
    const area = document.createElement('textarea');
    area.value = 'first second';
    area.setSelectionRange(5, 5);
    expect(insertAtCursor(area, area.value, '![a](att:T-1/x-a.png)')).toBe('first\n![a](att:T-1/x-a.png)\n second');
  });
});

describe('pending placeholders (files picked before the task exists)', () => {
  it('writes an image chip for images and a link for other files', () => {
    expect(pendingMarkdown(png('a [1].png'), '3')).toBe('![a 1.png](pending:3)');
    expect(pendingMarkdown(new File(['x'], 'spec.pdf', { type: 'application/pdf' }), '4')).toBe('[spec.pdf](pending:4)');
  });

  it('swaps each placeholder for the stored markdown, keeping its place in the text', () => {
    const body = `before\n${pendingMarkdown(png('a.png'), '1')}\nmiddle\n${pendingMarkdown(png('a.png'), '2')}\nafter`;
    const out = resolvePending(body, [
      { id: '1', markdown: '![a.png](att:T-1/11111111-a.png)' },
      { id: '2', markdown: '![a.png](att:T-1/22222222-a.png)' },
    ]);
    expect(out).toBe('before\n![a.png](att:T-1/11111111-a.png)\nmiddle\n![a.png](att:T-1/22222222-a.png)\nafter');
  });

  it('is not fooled by `$` in a stored name', () => {
    expect(resolvePending('![a](pending:1)', [{ id: '1', markdown: '[$&-x](att:T-1/y)' }])).toBe('[$&-x](att:T-1/y)');
  });

  it('removes a failed upload together with its line; text without placeholders is untouched', () => {
    expect(resolvePending(`one\n${pendingMarkdown(png(), '1')}\ntwo`, [{ id: '1', markdown: null }])).toBe('one\ntwo');
    expect(stripPending(`${pendingMarkdown(png(), '1')}\n${pendingMarkdown(png('b.png'), '2')}`)).toBe('');
    expect(stripPending('plain\n\n\ntext')).toBe('plain\n\n\ntext');
  });
});
