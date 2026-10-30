import { describe, expect, it } from 'vitest';
import { isFenceClosed } from './fence';

describe('isFenceClosed', () => {
  it('true for a closed fence', () => {
    expect(isFenceClosed('```js\nconst a = 1;\n```')).toBe(true);
    expect(isFenceClosed('~~~\nx\n~~~')).toBe(true);
  });
  it('false for an open (streaming) fence', () => {
    expect(isFenceClosed('```js\nconst a = 1')).toBe(false);
    expect(isFenceClosed('```js\nconst a = 1\n``')).toBe(false);
  });
  it('true for non-fenced (indented) code', () => {
    expect(isFenceClosed('    const a = 1;')).toBe(true);
  });
  it('a longer closing fence of the same character closes a shorter opener', () => {
    expect(isFenceClosed('```js\nconst a = 1;\n````')).toBe(true);
  });
  it('a mismatched fence character does not close', () => {
    expect(isFenceClosed('```js\nconst a = 1;\n~~~')).toBe(false);
  });
});
