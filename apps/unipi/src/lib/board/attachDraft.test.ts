import { describe, expect, it } from 'vitest';
import { attTarget, joinRefs, parseRef, pendingId, splitRefs } from './attachDraft';

describe('attachment links at the end of a body (D2)', () => {
  it('splits the closing links from the words and joins them back unchanged', () => {
    const body = 'Login bug\n\n![a.png](att:T-1/0001-a.png)\n[log.txt](att:T-1/0002-log.txt)';
    const { text, refs } = splitRefs(body);
    expect(text).toBe('Login bug\n');
    expect(refs.map((r) => [r.name, r.image, r.target])).toEqual([['a.png', true, 'att:T-1/0001-a.png'], ['log.txt', false, 'att:T-1/0002-log.txt']]);
    expect(joinRefs(text, refs)).toBe(body);
  });

  it('keeps a trailing newline the author typed, and a body of links alone', () => {
    expect(splitRefs('abc\n\n![x](pending:1)').text).toBe('abc\n');
    expect(joinRefs('abc\n', splitRefs('abc\n\n![x](pending:1)').refs)).toBe('abc\n\n![x](pending:1)');
    expect(splitRefs('![x](pending:1)')).toEqual({ text: '', refs: [parseRef('![x](pending:1)')] });
    expect(joinRefs('', [parseRef('![x](pending:1)')!])).toBe('![x](pending:1)');
  });

  it('leaves links in the middle of the text, other schemes and plain text alone', () => {
    const middle = 'see ![a](att:T-1/x.png) here';
    expect(splitRefs(middle)).toEqual({ text: middle, refs: [] });
    expect(splitRefs('docs\n[site](https://example.com)').refs).toEqual([]);
    expect(splitRefs('![a](att:T-1/x.png)\nmore words').refs).toEqual([]);
    expect(joinRefs('words', [])).toBe('words');
  });

  it('names a link target', () => {
    expect(attTarget('att:T-1/ab-shot.png')).toEqual({ task: 'T-1', name: 'ab-shot.png' });
    expect(attTarget('pending:3')).toBeUndefined();
    expect(pendingId(parseRef('![x](pending:3)')!)).toBe('3');
    expect(pendingId(parseRef('![x](att:T-1/a.png)')!)).toBeUndefined();
  });
});
