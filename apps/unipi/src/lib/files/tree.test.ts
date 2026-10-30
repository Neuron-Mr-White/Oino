import { describe, expect, it } from 'vitest';
import { emptyModel, flatten, flattenChanged, neededListings, rankByName, fuzzyScore, sortEntries, type TreeEntry, type TreeModel } from './tree';

const file = (name: string, extra: Partial<TreeEntry> = {}): TreeEntry => ({ name, kind: 'file', size: 10, mtime: 0, ...extra });
const dir = (name: string, extra: Partial<TreeEntry> = {}): TreeEntry => ({ name, kind: 'dir', size: 0, mtime: 0, ...extra });

const ROOT = '/home/you/p';

/** A model from `{ dirPath: entries }`. */
function model(listings: Record<string, TreeEntry[]>, expanded: string[] = [], roots: string[] = [ROOT]): TreeModel {
  return {
    roots,
    expanded: new Set(expanded),
    dirs: new Map(Object.entries(listings).map(([path, entries]) => [path, { status: 'ready', entries }])),
  };
}

describe('flatten', () => {
  it('shows the single root implicitly: its children at depth 0', () => {
    const rows = flatten(model({ [ROOT]: [dir('src'), file('README.md')] }));
    expect(rows.map((r) => [r.name, r.depth, r.kind])).toEqual([
      ['src', 0, 'dir'],
      ['README.md', 0, 'file'],
    ]);
  });

  it('sorts directories first, then case-insensitive names; dotfiles shown', () => {
    const rows = flatten(model({ [ROOT]: [file('b.ts'), dir('Zed'), file('.env'), dir('a')] }));
    expect(rows.map((r) => r.name)).toEqual(['a', 'Zed', '.env', 'b.ts']);
  });

  it('expands lazily: children only when the dir is expanded', () => {
    const rows = flatten(model({ [ROOT]: [dir('src')], [`${ROOT}/src`]: [file('math.ts')] }, [`${ROOT}/src`]));
    expect(rows.map((r) => [r.name, r.depth])).toEqual([
      ['src', 0],
      ['math.ts', 1],
    ]);
    expect(flatten(model({ [ROOT]: [dir('src')], [`${ROOT}/src`]: [file('math.ts')] }))).toHaveLength(1);  });

  it('compacts single-child dir chains into one row keyed by the chain end', () => {
    const listings = {
      [ROOT]: [dir('src')],
      [`${ROOT}/src`]: [dir('lib')],
      [`${ROOT}/src/lib`]: [dir('files')],
      [`${ROOT}/src/lib/files`]: [file('tree.ts')],
    };
    // Collapsed: one row for the whole chain.
    expect(flatten(model(listings)).map((r) => [r.name, r.path, r.depth])).toEqual([['src/lib/files', `${ROOT}/src/lib/files`, 0]]);
    // Expanded: the chain row, then the deepest dir's children.
    const rows = flatten(model(listings, [`${ROOT}/src/lib/files`]));
    expect(rows.map((r) => [r.name, r.depth])).toEqual([
      ['src/lib/files', 0],
      ['tree.ts', 1],
    ]);
  });

  it('does not compact when a dir also holds files or a second dir', () => {
    const listings = { [ROOT]: [dir('src')], [`${ROOT}/src`]: [dir('lib'), file('cli.ts')] };
    const rows = flatten(model(listings, [`${ROOT}/src`]));
    expect(rows.map((r) => r.name)).toEqual(['src', 'lib', 'cli.ts']);
  });

  it('carries git marks and ignored flags to rows; marks loading, empty and error dirs', () => {
    const rows = flatten(
      (() => {
        const m = model({ [ROOT]: [file('a.ts', { git: 'M' }), file('build.js', { ignored: true }), dir('new', { git: '?' }), dir('empty'), dir('gone'), dir('loading')] });
        const dirs = new Map(m.dirs);
        dirs.set(`${ROOT}/empty`, { status: 'ready', entries: [] });
        dirs.set(`${ROOT}/gone`, { status: 'error', error: 'No folder' });
        dirs.set(`${ROOT}/loading`, { status: 'loading' });
        return { ...m, dirs };
      })(),
    );
    const byName = Object.fromEntries(rows.map((r) => [r.name, r]));
    expect(byName['a.ts'].git).toBe('M');
    expect(byName['build.js'].ignored).toBe(true);
    expect(byName['new'].git).toBe('?');
    expect(byName.empty?.empty).toBe(true);
    expect(byName.gone?.error).toBe('No folder');
    expect(byName.loading?.loading).toBe(true);
  });

  it('lists several roots as expandable root rows', () => {
    const rows = flatten(model({ '/home/you': [dir('p')], '/srv': [dir('x')] }, [], ['/home/you', '/srv']));
    expect(rows.map((r) => [r.name, r.kind, r.depth])).toEqual([
      ['/home/you', 'root', 0],
      ['/srv', 'root', 0],
    ]);
  });
});

describe('flattenChanged (git filter)', () => {
  it('lists loaded entries with a git mark as root-relative paths, keeping untracked dirs expandable', () => {
    const rows = flattenChanged(
      model(
        {
          [ROOT]: [file('a.ts', { git: 'M' }), file('b.ts'), dir('new', { git: '?' })],
          [`${ROOT}/new`]: [file('c.rs', { git: 'A' })],
        },
        [`${ROOT}/new`],
      ),
    );
    expect(rows.map((r) => [r.name, r.git])).toEqual([
      ['a.ts', 'M'],
      ['new', '?'],
      ['new/c.rs', 'A'],
    ]);
  });
});

describe('neededListings', () => {
  it('asks for the root, expanded dirs that never loaded, and failed ones', () => {
    const m: TreeModel = {
      roots: [ROOT],
      expanded: new Set([`${ROOT}/src`, `${ROOT}/gone`]),
      dirs: new Map([
        [ROOT, { status: 'ready', entries: [dir('src'), dir('gone'), dir('fresh')] }],
        [`${ROOT}/src`, { status: 'ready', entries: [file('a.ts')] }],
        [`${ROOT}/gone`, { status: 'error', error: 'x' }],
      ]),
    };
    expect(neededListings(m)).toEqual([`${ROOT}/gone`]);
    // An unloaded root is wanted immediately.
    expect(neededListings(emptyModel(['/srv/data']))).toEqual(['/srv/data']);
  });
});

describe('fuzzy + rankByName', () => {
  it('scores subsequence matches with bonuses, 0 for misses', () => {
    expect(fuzzyScore('mt', 'math.ts')).toBeGreaterThan(0);
    expect(fuzzyScore('mth', 'math.ts')).toBeGreaterThan(fuzzyScore('mhh', 'math.ts') || 0);
    expect(fuzzyScore('zqx', 'math.ts')).toBe(0);
    expect(fuzzyScore('readme', 'README.md')).toBeGreaterThan(0);
  });

  it('ranks basename hits first, shorter paths win ties', () => {
    const ranked = rankByName('math', [`${ROOT}/docs/math-old.md`, `${ROOT}/src/math.ts`, `${ROOT}/src/graphics.ts`]).map((h) => h.path);
    expect(ranked[0]).toBe(`${ROOT}/src/math.ts`);
    expect(ranked).not.toContain(`${ROOT}/src/graphics.ts`);
  });
});

describe('performance (acceptance: 50k entries flatten for a virtual list frame)', () => {
  it('flattens a 50k-entry expanded tree well inside a 16 ms frame budget', () => {
    // 20 dirs × 2 500 files = 50 000 loaded entries, every dir expanded.
    const listings: Record<string, TreeEntry[]> = {};
    const rootEntries: TreeEntry[] = [];
    const expanded: string[] = [];
    for (let d = 0; d < 20; d++) {
      const name = `dir${String(d).padStart(2, '0')}`;
      rootEntries.push(dir(name));
      const path = `${ROOT}/${name}`;
      expanded.push(path);
      listings[path] = Array.from({ length: 2500 }, (_, i) => file(`file${String(i).padStart(4, '0')}.ts`));
    }
    listings[ROOT] = rootEntries;
    const m = model(listings, expanded);

    const rows = flatten(m);
    expect(rows).toHaveLength(50_020);

    // Warm up (JIT), then measure a flatten + a viewport slice, like the
    // virtualizer does per frame. The **best** run is measured: the gate
    // guards the complexity class (a per-row string/path rebuild is ~10×
    // slower), not a scheduler hiccup while other test files run in
    // parallel on the same CPU.
    flatten(m);
    const runs = 7;
    let best = Infinity;
    for (let i = 0; i < runs; i++) {
      const t0 = performance.now();
      const all = flatten(m);
      const start = 10_000 + i * 100;
      void all.slice(start, start + 40).map((r) => r.name);
      best = Math.min(best, performance.now() - t0);
    }
    // The budget is 16.6 ms (60 fps).
    expect(best).toBeLessThan(16);
  });
});

describe('sortEntries', () => {
  it('is stable input for the view (dirs first, case-insensitive)', () => {
    expect(sortEntries([file('b'), dir('c'), file('A'), dir('a')]).map((e) => e.name)).toEqual(['a', 'c', 'A', 'b']);
  });
});
