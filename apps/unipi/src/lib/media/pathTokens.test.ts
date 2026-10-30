import { describe, it, expect } from 'vitest';
import { looksLikePath, resolvePathToken } from './pathTokens';

describe('looksLikePath', () => {
  it('accepts absolute paths', () => {
    expect(looksLikePath('/home/u/proj/src/a.ts')).toBe('/home/u/proj/src/a.ts');
    expect(looksLikePath('/etc/hosts')).toBe('/etc/hosts');
  });

  it('accepts @-prefixed paths', () => {
    expect(looksLikePath('@src/lib/foo.ts')).toBe('src/lib/foo.ts');
    expect(looksLikePath('@/abs/path.py')).toBe('/abs/path.py');
  });

  it('accepts explicit relative paths', () => {
    expect(looksLikePath('./src/a.ts')).toBe('./src/a.ts');
    expect(looksLikePath('../sibling/b.rs')).toBe('../sibling/b.rs');
  });

  it('accepts a relative path with a slash and an extension', () => {
    expect(looksLikePath('src/lib/foo.ts')).toBe('src/lib/foo.ts');
  });

  it('rejects bare words, prose and URLs', () => {
    expect(looksLikePath('hello')).toBeUndefined();
    expect(looksLikePath('foo.bar')).toBeUndefined(); // no slash, ambiguous
    expect(looksLikePath('npm install')).toBeUndefined(); // has whitespace
    expect(looksLikePath('https://example.com/a.ts')).toBeUndefined();
    expect(looksLikePath('')).toBeUndefined();
  });

  it('rejects an absurdly long token', () => {
    expect(looksLikePath(`/${'a'.repeat(400)}.ts`)).toBeUndefined();
  });
});

describe('resolvePathToken', () => {
  it('leaves an absolute path unchanged', () => {
    expect(resolvePathToken('/a/b.ts', '/home/u/proj')).toBe('/a/b.ts');
  });
  it('joins a relative path onto cwd', () => {
    expect(resolvePathToken('src/a.ts', '/home/u/proj')).toBe('/home/u/proj/src/a.ts');
    expect(resolvePathToken('./src/a.ts', '/home/u/proj')).toBe('/home/u/proj/src/a.ts');
  });
  it('handles a cwd with a trailing slash', () => {
    expect(resolvePathToken('a.ts', '/home/u/proj/')).toBe('/home/u/proj/a.ts');
  });

  it('passes ~-prefixed paths through for the host to expand (UNI-178)', () => {
    expect(resolvePathToken('~/notes/a.md', '/home/u/proj')).toBe('~/notes/a.md');
    expect(resolvePathToken('~', '/home/u/proj')).toBe('~');
    expect(resolvePathToken('~', undefined)).toBe('~');
  });

  it('falls back to the raw path when relative with no cwd', () => {
    expect(resolvePathToken('src/a.ts', undefined)).toBe('src/a.ts');
  });
  it('without a cwd, returns the relative path as-is', () => {
    expect(resolvePathToken('src/a.ts', undefined)).toBe('src/a.ts');
  });
});
