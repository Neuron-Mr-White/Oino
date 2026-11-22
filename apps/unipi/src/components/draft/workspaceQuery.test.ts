/**
 * The pure rules behind the one workspace picker: what a query means (name / path), `~` `./` `../`
 * resolution, Tab completion, the order of the lists (zoxide's frecency first, deduplicated with
 * Oino's recents), and the keyboard map.
 */
import { describe, expect, it } from 'vitest';
import { ancestorRows, commonPrefix, completionFor, expandHome, isPathQuery, keyAction, mergeFrequent, normalizePath, parentOf, rankDirs, rankQuickRows, rankZoxide, splitDisplay, splitPathQuery, tabComplete, type QuickRow } from './workspaceQuery';

const HOME = '/home/you';
const RECENTS: QuickRow[] = [
  { path: '/home/you/p/unipi', label: 'unipi', detail: '~/p/unipi' },
  { path: '/home/you/p/oino', label: 'oino', detail: '~/p/oino' },
  { path: '/home/you/p/notes', label: 'notes', detail: '~/p/notes' },
];

describe('zoxide ranking', () => {
  it('lists zoxide hits first, best frecency first, whatever order they arrive in', () => {
    const rows = rankQuickRows('oi', [{ path: '/srv/oil', score: 2 }, { path: '/home/you/p/oino', score: 40 }, { path: '/home/you/old/oi', score: 9 }], RECENTS);
    expect(rows.map((r) => r.path)).toEqual(['/home/you/p/oino', '/home/you/old/oi', '/srv/oil']);
    expect(rows.every((r) => r.source === 'zoxide')).toBe(true);
  });

  it('puts fuzzy-matching recents after the zoxide hits, without repeating a folder', () => {
    const rows = rankQuickRows('uni', [{ path: '/home/you/code/universe', score: 5 }, { path: '/home/you/p/unipi', score: 3 }], RECENTS);
    expect(rows.map((r) => [r.path, r.source])).toEqual([
      ['/home/you/code/universe', 'zoxide'],
      ['/home/you/p/unipi', 'zoxide'],
    ]);
    const extra = rankQuickRows('no', [{ path: '/z/other', score: 1 }], RECENTS);
    expect(extra.map((r) => [r.path, r.source])).toEqual([
      ['/z/other', 'zoxide'],
      ['/home/you/p/notes', 'recent'],
      ['/home/you/p/oino', 'recent'],
    ]);
  });

  it('marks the matched letters of the folder name', () => {
    const [row] = rankQuickRows('oin', [{ path: '/home/you/p/oino', score: 3 }], []);
    expect(row!.marks).toEqual([0, 1, 2]);
  });

  it('ties keep zoxide\'s own order and duplicates collapse', () => {
    expect(rankZoxide([{ path: '/a', score: 1 }, { path: '/b', score: 1 }, { path: '/a', score: 1 }]).map((h) => h.path)).toEqual(['/a', '/b']);
  });
});

describe('empty query', () => {
  it('shows the recent workspaces as given (newest first), ignoring zoxide', () => {
    expect(rankQuickRows('', [], RECENTS).map((r) => r.path)).toEqual(RECENTS.map((r) => r.path));
    expect(rankQuickRows('   ', [], RECENTS).every((r) => r.source === 'recent')).toBe(true);
  });
});

describe('~ and absolute paths', () => {
  it('knows a path query from a name', () => {
    expect(['~', '~/p', '/srv', ' /srv'].every(isPathQuery)).toBe(true);
    expect(['oino', '~oino', 'p/oino', ''].some(isPathQuery)).toBe(false);
  });

  it('expands ~ to the home', () => {
    expect(expandHome('~', HOME)).toBe(HOME);
    expect(expandHome('~/p/oino', HOME)).toBe('/home/you/p/oino');
    expect(expandHome('~/p', `${HOME}/`)).toBe('/home/you/p');
    expect(expandHome('/srv/x', HOME)).toBe('/srv/x');
    expect(expandHome('~/p', undefined)).toBe('~/p');
  });

  it('splits a path into the folder to list and the last-segment filter', () => {
    expect(splitPathQuery('~/p/', HOME)).toEqual({ dir: '/home/you/p', prefix: '' });
    expect(splitPathQuery('~/p/oi', HOME)).toEqual({ dir: '/home/you/p', prefix: 'oi' });
    expect(splitPathQuery('~', HOME)).toEqual({ dir: HOME, prefix: '' });
    expect(splitPathQuery('~/x', HOME)).toEqual({ dir: HOME, prefix: 'x' });
    expect(splitPathQuery('/', HOME)).toEqual({ dir: '/', prefix: '' });
    expect(splitPathQuery('/srv', HOME)).toEqual({ dir: '/', prefix: 'srv' });
    expect(splitPathQuery('/srv/a/', undefined)).toEqual({ dir: '/srv/a', prefix: '' });
    // `~` with an unknown home cannot be resolved yet.
    expect(splitPathQuery('~/p', undefined)).toBeUndefined();
    expect(splitPathQuery('oino', HOME)).toBeUndefined();
  });

  it('ranks the folders of a directory by the last segment, an exact name first', () => {
    const rows = rankDirs('/home/you/p', 'oino', ['oino-web', 'oino', 'other', 'notes']);
    expect(rows.map((r) => r.path)).toEqual(['/home/you/p/oino', '/home/you/p/oino-web']);
    expect(rankDirs('/', '', ['b', 'a']).map((r) => r.path)).toEqual(['/b', '/a']);
  });

  it('Tab writes the row back as ~/…/ (or an absolute path outside home)', () => {
    expect(completionFor('/home/you/p/oino', HOME)).toBe('~/p/oino/');
    expect(completionFor('/srv/app', HOME)).toBe('/srv/app/');
    expect(completionFor('/srv/app/', HOME)).toBe('/srv/app/');
  });

  it('shortens a row to a dim parent and the folder name', () => {
    expect(splitDisplay('/home/you/p/oino', HOME)).toEqual({ parent: '~/p/', name: 'oino' });
    expect(splitDisplay('/srv/app', HOME)).toEqual({ parent: '/srv/', name: 'app' });
  });
});

describe('relative paths', () => {
  it('knows ./ and ../ as paths, not ".git" or "..foo"', () => {
    expect(['.', '..', './x', '../x', ' ./x'].every(isPathQuery)).toBe(true);
    expect(['.git', '..foo', 'x/y', 'oino web'].some(isPathQuery)).toBe(false);
  });

  it('resolves ./ and ../ against the folder being browsed', () => {
    const cwd = '/home/you/p';
    expect(splitPathQuery('./', HOME, cwd)).toEqual({ dir: '/home/you/p', prefix: '' });
    expect(splitPathQuery('./oi', HOME, cwd)).toEqual({ dir: '/home/you/p', prefix: 'oi' });
    expect(splitPathQuery('../', HOME, cwd)).toEqual({ dir: '/home/you', prefix: '' });
    expect(splitPathQuery('..', HOME, cwd)).toEqual({ dir: '/home/you', prefix: '' });
    expect(splitPathQuery('../oino/', HOME, cwd)).toEqual({ dir: '/home/you/oino', prefix: '' });
    expect(splitPathQuery('../../../../x', HOME, cwd)).toEqual({ dir: '/', prefix: 'x' });
    // No folder to resolve against yet.
    expect(splitPathQuery('./x', HOME)).toBeUndefined();
  });

  it('normalises dot segments and never climbs above /', () => {
    expect(normalizePath('/a/./b/../c//d')).toBe('/a/c/d');
    expect(normalizePath('/../..')).toBe('/');
  });

  it('parentOf never goes above /', () => {
    expect(parentOf('/home/you/p')).toBe('/home/you');
    expect(parentOf('/home')).toBe('/');
    expect(parentOf('/')).toBe('/');
  });
});

describe('Tab completion', () => {
  const rows = rankDirs('/home/you/p', '', ['oino', 'oino-web', 'oink', 'notes']);

  it('extends to the longest shared prefix first (shell style)', () => {
    expect(tabComplete('~/p/oi', rows, 0, 'oi')).toBe('~/p/oin');
    expect(tabComplete('/home/you/p/o', rows, -1, 'o')).toBe('/home/you/p/oin');
  });

  it('then completes to the highlighted match, with a trailing slash, keeping the typed style', () => {
    expect(tabComplete('~/p/oin', rows, 2, 'oin')).toBe('~/p/oink/');
    expect(tabComplete('~/p/oin', rows, -1, 'oin')).toBe('~/p/oino/');
    expect(tabComplete('~/p/n', rows, 3, 'n')).toBe('~/p/notes/');
  });

  it('with nothing typed after the slash it descends into the highlighted (else first) folder', () => {
    expect(tabComplete('~/p/', rows, 1, '')).toBe('~/p/oino-web/');
    expect(tabComplete('~/p/', rows, -1, '')).toBe('~/p/oino/');
  });

  it('~, ., .. gain their slash; ./ and ../ stay relative', () => {
    const home = rankDirs('/home/you', '', ['p', 'q']);
    expect(tabComplete('~', home, 0, '')).toBe('~/p/');
    expect(tabComplete('.', home, 1, '')).toBe('./q/');
    expect(tabComplete('../p', rankDirs('/home/you', 'p', ['p', 'pp']), 0, 'p')).toBe('../p/');
  });

  it('does nothing for a name query or an empty folder', () => {
    expect(tabComplete('oino', rows, 0, 'oino')).toBeUndefined();
    expect(tabComplete('~/p/', [], -1, '')).toBeUndefined();
    expect(commonPrefix(['abc', 'abd', 'ab'])).toBe('ab');
  });
});

describe('Frequent = zoxide frequency, merged with Oino recents', () => {
  it('zoxide order drives the list; recents it did not list follow, deduplicated', () => {
    const rows = mergeFrequent(
      [{ path: '/home/you/p/web', score: 3 }, { path: '/home/you/p/oino', score: 30 }],
      RECENTS,
    );
    expect(rows.map((r) => [r.path, r.source])).toEqual([
      ['/home/you/p/oino', 'zoxide'],
      ['/home/you/p/web', 'zoxide'],
      ['/home/you/p/unipi', 'recent'],
      ['/home/you/p/notes', 'recent'],
    ]);
  });

  it('without zoxide it is the recents as given', () => {
    expect(mergeFrequent([], RECENTS).map((r) => r.path)).toEqual(RECENTS.map((r) => r.path));
  });

  it('caps the list', () => {
    const many = Array.from({ length: 12 }, (_, i) => ({ path: `/z/${i}`, score: 100 - i }));
    expect(mergeFrequent(many, RECENTS, 5)).toHaveLength(5);
  });
});

describe('typing finds folders right here too', () => {
  it('zoxide first, then recents, then the current folder\'s matches (they open, like a search hit)', () => {
    const here: QuickRow[] = [
      { path: '/home/you/oinotes', label: 'oinotes', source: 'dir' },
      { path: '/home/you/other', label: 'other', source: 'dir' },
    ];
    const rows = rankQuickRows('oin', [{ path: '/home/you/p/oino', score: 9 }], RECENTS, here);
    expect(rows.map((r) => [r.path, r.source])).toEqual([
      ['/home/you/p/oino', 'zoxide'],
      ['/home/you/oinotes', 'near'],
    ]);
  });
});

describe('keyboard map', () => {
  const state = (over: Partial<Parameters<typeof keyAction>[1]> = {}) => ({ query: '', mode: 'browse' as const, rows: 5, selected: 0, source: 'zoxide' as QuickRow['source'], ...over });
  const key = (k: string, mods: Record<string, boolean> = {}) => ({ key: k, ...mods });

  it('↑/↓ select, Alt+↑ goes to the parent', () => {
    expect(keyAction(key('ArrowDown'), state())).toBe('next');
    expect(keyAction(key('ArrowUp'), state())).toBe('prev');
    expect(keyAction(key('ArrowUp', { altKey: true }), state())).toBe('up');
  });

  it('Enter opens a Frequent/search row as the workspace, enters a browsed folder', () => {
    expect(keyAction(key('Enter'), state({ source: 'zoxide' }))).toBe('use');
    expect(keyAction(key('Enter'), state({ source: 'recent' }))).toBe('use');
    // A plain folder a search matched (not a zoxide/recent one) is entered, like type-ahead in any folder dialog.
    expect(keyAction(key('Enter'), state({ mode: 'search', source: 'near' }))).toBe('enter');
    expect(keyAction(key('Enter'), state({ source: 'dir' }))).toBe('enter');
    expect(keyAction(key('Enter'), state({ mode: 'path', source: 'dir' }))).toBe('enter');
  });

  it('Enter with nothing highlighted uses the current folder (browsing) and does nothing in an empty search', () => {
    expect(keyAction(key('Enter'), state({ selected: -1 }))).toBe('use-target');
    expect(keyAction(key('Enter'), state({ rows: 0 }))).toBe('use-target');
    expect(keyAction(key('Enter'), state({ mode: 'search', query: 'zzz', rows: 0, selected: -1 }))).toBe('none');
  });

  it('Ctrl/Cmd+Enter always uses the highlighted/current folder, even on a browsed folder', () => {
    expect(keyAction(key('Enter', { ctrlKey: true }), state({ source: 'dir' }))).toBe('use-target');
    expect(keyAction(key('Enter', { metaKey: true }), state({ source: 'dir' }))).toBe('use-target');
  });

  it('Tab completes a path, steps inside a highlighted folder, and never leaves the box (it used to reach "No workspace")', () => {
    expect(keyAction(key('Tab'), state({ mode: 'path', query: '~/p' }))).toBe('complete');
    expect(keyAction(key('Tab'), state({ mode: 'search', query: 'oi' }))).toBe('step-in');
    expect(keyAction(key('Tab'), state({ mode: 'browse', selected: 0 }))).toBe('step-in');
    expect(keyAction(key('Tab'), state({ mode: 'search', query: 'zz', rows: 0, selected: -1 }))).toBe('swallow');
    expect(keyAction(key('Tab'), state({ mode: 'browse', selected: -1 }))).toBe('swallow');
    // Shift+Tab is the user's own way out.
    expect(keyAction(key('Tab', { shiftKey: true }), state({ mode: 'path', query: '~/p' }))).toBe('none');
  });

  it('Backspace on an empty box goes up; with text it just edits', () => {
    expect(keyAction(key('Backspace'), state())).toBe('up');
    expect(keyAction(key('Backspace'), state({ query: 'o', mode: 'search' }))).toBe('none');
  });

  it('Esc clears the box first, then closes', () => {
    expect(keyAction(key('Escape'), state({ query: 'oi', mode: 'search' }))).toBe('clear');
    expect(keyAction(key('Escape'), state())).toBe('close');
  });
});

describe('parent / organisation folders (zoxide only matches the last component)', () => {
  const hits = [
    { path: `${HOME}/Projects/Work/demoeditor/api`, score: 50 },
    { path: `${HOME}/Projects/Work/demoeditor/web`, score: 30 },
    { path: `${HOME}/Projects/oino`, score: 20 },
  ];

  it('"de" finds the demoeditor folder above three frequent projects, once', () => {
    const rows = ancestorRows('de', hits, HOME);
    expect(rows.map((r) => [r.path, r.source])).toEqual([[`${HOME}/Projects/Work/demoeditor`, 'near']]);
    expect(rows[0]!.marks).toEqual([0, 1]);
  });

  it('needs every word in order and the last in the folder name; skips hit folders themselves', () => {
    expect(ancestorRows('work de', hits, HOME).map((r) => r.path)).toEqual([`${HOME}/Projects/Work/demoeditor`]);
    expect(ancestorRows('de work', hits, HOME)).toEqual([]);
    expect(ancestorRows('api', hits, HOME)).toEqual([]);
    expect(ancestorRows('', hits, HOME)).toEqual([]);
  });

  it('joins the search list after zoxide hits, recents and nearby folders', () => {
    const rows = rankQuickRows('de', [], [], [], ancestorRows('de', hits, HOME));
    expect(rows.map((r) => r.path)).toEqual([`${HOME}/Projects/Work/demoeditor`]);
  });
});
