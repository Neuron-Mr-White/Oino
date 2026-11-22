import { describe, expect, it, beforeEach } from 'vitest';
import { basename, dirname, joinPath, formatSize, formatMtime, pushRecent, readRecents } from './fs';

describe('basename/dirname/joinPath', () => {
  it('basename returns the last path segment', () => {
    expect(basename('/home/u/proj/a.txt')).toBe('a.txt');
    expect(basename('/home/u/proj/')).toBe('proj');
    expect(basename('/')).toBe('/');
  });

  it('dirname returns the parent directory', () => {
    expect(dirname('/home/u/proj/a.txt')).toBe('/home/u/proj');
    expect(dirname('/home')).toBe('/');
    expect(dirname('/')).toBe('/');
  });

  it('joinPath joins a dir and a name', () => {
    expect(joinPath('/home/u', 'a.txt')).toBe('/home/u/a.txt');
    expect(joinPath('/', 'a.txt')).toBe('/a.txt');
  });
});

describe('formatSize', () => {
  it('formats bytes/KB/MB/GB', () => {
    expect(formatSize(500)).toBe('500 B');
    expect(formatSize(2048)).toBe('2.0 KB');
    expect(formatSize(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(formatSize(2 * 1024 * 1024 * 1024)).toBe('2.0 GB');
  });
});

describe('formatMtime', () => {
  it('formats recent times relatively', () => {
    const now = Date.now();
    expect(formatMtime(now - 1000)).toBe('just now');
    expect(formatMtime(now - 5 * 60_000)).toBe('5m ago');
    expect(formatMtime(now - 3 * 3_600_000)).toBe('3h ago');
    expect(formatMtime(now - 2 * 86_400_000)).toBe('2d ago');
  });
});

describe('recents', () => {
  beforeEach(() => localStorage.clear());

  it('pushRecent dedupes and orders newest-first', () => {
    pushRecent('h1', '/a.txt');
    pushRecent('h1', '/b.txt');
    pushRecent('h1', '/a.txt');
    const recents = readRecents();
    expect(recents.map((r) => r.path)).toEqual(['/a.txt', '/b.txt']);
  });

  it('readRecents tolerates garbage in storage', () => {
    localStorage.setItem('unipi.files.recent', 'not json');
    expect(readRecents()).toEqual([]);
  });

  it('caps the recents list', () => {
    for (let i = 0; i < 30; i++) pushRecent('h1', `/f${i}.txt`);
    expect(readRecents().length).toBeLessThanOrEqual(20);
  });
});
