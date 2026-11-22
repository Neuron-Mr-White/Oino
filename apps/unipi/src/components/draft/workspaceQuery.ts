/**
 * Pure helpers behind the one workspace picker (FolderPicker.tsx): what a typed query means
 * (a name to search, or a path to browse), how `~`, `./` and `../` resolve, how Tab completes a
 * path, how zoxide's frecency hits, Oino's recent workspaces and the current folder's children
 * merge into one list, and what each key does. No UI, no host calls — unit-tested on their own.
 */
import { fuzzyRank } from '../../lib/fuzzy';
import { folderName, shortPath } from '../../lib/session/newSession';

/** A row of the list: a zoxide hit, a recent workspace, or a folder inside the browsed dir. */
export type QuickRow = {
  path: string;
  label: string;
  detail?: string;
  /** `zoxide`/`recent` rows OPEN as the workspace on Enter; `dir` (plain browsing) and `near` (a plain folder a search matched — here, or a parent of a frequent project) rows are ENTERED, like type-ahead in any folder dialog. */
  source?: 'zoxide' | 'recent' | 'near' | 'dir';
  /** Fuzzy-matched character positions inside `label` (marked in the list). */
  marks?: readonly number[];
};

export type ZoxideLike = { path: string; score: number };

/** The user's home among a host's `fs_roots` (it is listed first; a Unix or Windows home shape). */
export function homeOf(roots: readonly string[]): string | undefined {
  return roots.find((r) => /^\/home\/[^/]+\/?$|^\/Users\/[^/]+\/?$|^\/root\/?$|^[A-Za-z]:\\Users\\[^\\]+\\?$/.test(r));
}

/** `/abs/...`, `~`/`~/...`, `./...` or `../...` — the query names a place to browse, not a project to search for. */
export function isPathQuery(query: string): boolean {
  const q = query.trim();
  return q.startsWith('/') || q === '~' || q.startsWith('~/') || q === '.' || q === '..' || q.startsWith('./') || q.startsWith('../');
}

/** `/home/u/a/b` → `/home/u/a` (never above `/`). */
export const parentOf = (path: string): string => {
  const trimmed = path.replace(/\/+$/, '');
  const at = trimmed.lastIndexOf('/');
  return at <= 0 ? '/' : trimmed.slice(0, at);
};

/** Collapses `.`/`..`/`//` segments of an absolute path (never above `/`). */
export function normalizePath(abs: string): string {
  const out: string[] = [];
  for (const part of abs.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') out.pop();
    else out.push(part);
  }
  return `/${out.join('/')}`;
}

/** `~` and `~/x` → the home directory (unchanged when the home is unknown). */
export function expandHome(path: string, home: string | undefined): string {
  if (!home) return path;
  const base = home.replace(/\/+$/, '');
  if (path === '~') return base;
  if (path.startsWith('~/')) return `${base}${path.slice(1)}`;
  return path;
}

/**
 * A path query → the directory to list and the last-segment filter.
 * `~/p/` → dir `<home>/p`, prefix ''; `~/p/oi` → dir `<home>/p`, prefix `oi`; `~`, `.`, `..` → the folder itself.
 * `./x` and `../x` resolve against `cwd` (the folder being browsed).
 * `undefined` when the query is not (or cannot yet be resolved to) an absolute path.
 */
export function splitPathQuery(query: string, home: string | undefined, cwd?: string): { dir: string; prefix: string } | undefined {
  const q = query.trim();
  if (!isPathQuery(q)) return undefined;
  const relative = q === '.' || q === '..' || q.startsWith('./') || q.startsWith('../');
  const base = relative ? cwd : undefined;
  if (relative && !base) return undefined;
  const joined = relative ? `${base!.replace(/\/+$/, '')}/${q}` : expandHome(q, home);
  if (!joined.startsWith('/')) return undefined;
  const whole = q === '~' || q === '.' || q === '..' || q.endsWith('/') || q.endsWith('/.') || q.endsWith('/..');
  const abs = normalizePath(joined);
  if (whole) return { dir: abs, prefix: '' };
  const at = abs.lastIndexOf('/');
  return { dir: at <= 0 ? '/' : abs.slice(0, at), prefix: abs.slice(at + 1) };
}

/** What Tab writes into the field: the row's path (home collapsed to `~`) plus a trailing slash, ready to descend. */
export function completionFor(path: string, home: string | undefined): string {
  const short = home && (path === home || path.startsWith(`${home}/`)) ? `~${path.slice(home.length)}` : path;
  return short.endsWith('/') ? short : `${short}/`;
}

/** `~/Projects/oino` → `{ parent: '~/Projects/', name: 'oino' }` for the two-tone row. */
export function splitDisplay(path: string, home: string | undefined): { parent: string; name: string } {
  const short = shortPath(path, home);
  const trimmed = short.replace(/\/+$/, '');
  const at = trimmed.lastIndexOf('/');
  return at < 0 ? { parent: '', name: trimmed } : { parent: trimmed.slice(0, at + 1), name: trimmed.slice(at + 1) };
}

/** zoxide's answer in frecency order (best score first; ties keep zoxide's own order), de-duplicated. */
export function rankZoxide(hits: readonly ZoxideLike[]): ZoxideLike[] {
  const seen = new Set<string>();
  return hits
    .map((hit, index) => ({ hit, index }))
    .sort((a, b) => b.hit.score - a.hit.score || a.index - b.index)
    .map(({ hit }) => hit)
    .filter((hit) => (seen.has(hit.path) ? false : (seen.add(hit.path), true)));
}

/**
 * The name-search list: zoxide's hits FIRST (frecency order — they are what `z <query>` would
 * jump to), then this host's recent workspaces the query fuzzy-matches (best match first).
 * A recent matches on its folder name (its whole path only once the query has a `/`), so a
 * short query is not matched by letters scattered through `/home/you/…`.
 * An empty query lists the recents as they come (newest first).
 */
export function rankQuickRows(query: string, hits: readonly ZoxideLike[], recents: readonly QuickRow[], children: readonly QuickRow[] = [], ancestors: readonly QuickRow[] = []): QuickRow[] {
  const q = query.trim();
  const seen = new Set<string>();
  const out: QuickRow[] = [];
  for (const hit of rankZoxide(hits)) {
    seen.add(hit.path);
    const label = folderName(hit.path);
    const marks = q ? fuzzyRank(q, [label], (n) => [n])[0]?.positions : undefined;
    out.push({ path: hit.path, label, source: 'zoxide', ...(marks?.length ? { marks } : {}) });
  }
  const ranked = q ? fuzzyRank(q, recents, (r) => (q.includes('/') ? [r.label, r.path] : [r.label])) : recents.map((item) => ({ item, field: 0, positions: [] as number[] }));
  for (const { item, field, positions } of ranked) {
    if (seen.has(item.path)) continue;
    seen.add(item.path);
    out.push({ ...item, source: 'recent', ...(field === 0 && positions.length ? { marks: positions } : {}) });
  }
  // Folders right here that the query fuzzy-matches (zoxide may never have seen them).
  if (q) {
    for (const { item, positions } of fuzzyRank(q, children, (c) => [c.label]).slice(0, 5)) {
      if (seen.has(item.path)) continue;
      seen.add(item.path);
      out.push({ ...item, source: 'near', ...(positions.length ? { marks: positions } : {}) });
    }
    for (const row of ancestors) {
      if (seen.has(row.path)) continue;
      seen.add(row.path);
      out.push(row);
    }
  }
  return out;
}

/**
 * Parent / organisation folders a name search should find. zoxide only matches the LAST keyword against
 * the LAST path component, so `demoeditor` finds nothing when three frequent projects live in `…/demoeditor/`.
 * Here every folder above a known hit whose name (and, for several words, whose path in order) matches becomes
 * a row that is entered, not opened. Frecency order of the hits is kept; the hit folders themselves are skipped.
 */
export function ancestorRows(query: string, hits: readonly ZoxideLike[], home: string | undefined, limit = 5): QuickRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const last = words[words.length - 1]!;
  const out: QuickRow[] = [];
  const seen = new Set(hits.map((h) => h.path));
  for (const hit of rankZoxide(hits)) {
    const base = home && hit.path.startsWith(`${home.replace(/\/+$/, '')}/`) ? home.replace(/\/+$/, '') : '';
    const segs = hit.path.slice(base.length).split('/').filter(Boolean);
    for (let i = 0; i < segs.length - 1; i += 1) {
      const path = `${base}/${segs.slice(0, i + 1).join('/')}`;
      if (seen.has(path)) continue;
      const name = segs[i]!;
      const rel = segs.slice(0, i + 1).join('/').toLowerCase();
      const at = name.toLowerCase().indexOf(last);
      let from = 0;
      const inOrder = words.every((w) => {
        const found = rel.indexOf(w, from);
        from = found + w.length;
        return found >= 0;
      });
      if (at < 0 || !inOrder) continue;
      seen.add(path);
      out.push({ path, label: name, source: 'near', marks: Array.from({ length: last.length }, (_, k) => at + k) });
      if (out.length >= limit) return out;
    }
  }
  return out;
}

/** The folders of a browsed directory that the last typed segment matches (exact name first, then fuzzy rank). */
export function rankDirs(dir: string, prefix: string, names: readonly string[]): QuickRow[] {
  const base = dir.replace(/\/+$/, '');
  const row = (name: string, marks?: readonly number[]): QuickRow => ({
    path: `${base}/${name}`,
    label: name,
    source: 'dir',
    ...(marks?.length ? { marks } : {}),
  });
  if (!prefix) return names.map((n) => row(n));
  const ranked = fuzzyRank(prefix, names, (n) => [n]);
  const exact = ranked.findIndex((r) => r.item === prefix);
  if (exact > 0) ranked.unshift(...ranked.splice(exact, 1));
  return ranked.map((r) => row(r.item, r.positions));
}


/** The longest prefix every name shares (case-sensitive, from the first name's spelling). */
export function commonPrefix(names: readonly string[]): string {
  if (!names.length) return '';
  let prefix = names[0]!;
  for (const name of names.slice(1)) {
    let i = 0;
    while (i < prefix.length && i < name.length && prefix[i] === name[i]) i += 1;
    prefix = prefix.slice(0, i);
  }
  return prefix;
}

/**
 * Tab in path mode: extend the typed path to the longest prefix all matching folders share
 * (shell style); when it cannot grow, complete to the highlighted (else first) match with a
 * trailing `/`, ready to descend. The typed style (`~/`, `/abs`, `./`) is kept. `undefined` = nothing to complete.
 */
export function tabComplete(query: string, rows: readonly QuickRow[], selected: number, prefix: string): string | undefined {
  const q = query.trim();
  if (!isPathQuery(q) || !rows.length) return undefined;
  const slash = q.lastIndexOf('/');
  const dirPart = q === '~' ? '~/' : q === '.' ? './' : q === '..' ? '../' : slash >= 0 ? q.slice(0, slash + 1) : '';
  const typed = q.endsWith('/') || q === '~' || q === '.' || q === '..' ? '' : prefix;
  const lower = typed.toLowerCase();
  const matches = rows.filter((r) => r.label.toLowerCase().startsWith(lower));
  if (typed && matches.length > 1) {
    const shared = commonPrefix(matches.map((r) => r.label));
    if (shared.length > typed.length) return `${dirPart}${shared}`;
  }
  const row = (selected >= 0 && matches.includes(rows[selected]!) ? rows[selected] : undefined) ?? matches[0] ?? rows[selected] ?? rows[0];
  return row ? `${dirPart}${row.label}/` : undefined;
}

/** The Frequent list: zoxide's top folders in its order, then Oino's recent workspaces it did not list (deduplicated). */
export function mergeFrequent(hits: readonly ZoxideLike[], recents: readonly QuickRow[], limit = 8): QuickRow[] {
  const seen = new Set<string>();
  const out: QuickRow[] = [];
  for (const hit of rankZoxide(hits)) {
    if (out.length >= limit) break;
    seen.add(hit.path);
    out.push({ path: hit.path, label: folderName(hit.path), source: 'zoxide' });
  }
  for (const item of recents) {
    if (out.length >= limit) break;
    if (seen.has(item.path)) continue;
    seen.add(item.path);
    out.push({ ...item, source: 'recent' });
  }
  return out;
}

/** What one key does in the picker (the box keeps focus; the list is driven by `aria-activedescendant`). */
export type PickerAction = 'next' | 'prev' | 'enter' | 'use' | 'use-target' | 'complete' | 'step-in' | 'swallow' | 'up' | 'clear' | 'close' | 'none';

export function keyAction(
  e: { key: string; ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean; altKey?: boolean },
  s: { query: string; mode: 'browse' | 'search' | 'path'; rows: number; selected: number; source?: QuickRow['source'] },
): PickerAction {
  const mod = !!(e.ctrlKey || e.metaKey);
  switch (e.key) {
    case 'ArrowDown':
      return 'next';
    case 'ArrowUp':
      return e.altKey ? 'up' : 'prev';
    case 'Enter': {
      // Ctrl/Cmd+Enter: the highlighted folder, else the one being browsed — whatever the source.
      if (mod) return 'use-target';
      const row = s.selected >= 0 && s.selected < s.rows;
      if (!row) return s.mode === 'search' ? 'none' : 'use-target';
      // A folder found by search / Frequent opens as the workspace; a browsed folder is entered.
      return s.source === 'dir' || s.source === 'near' ? 'enter' : 'use';
    }
    case 'Tab':
      // Tab never leaves the box (it used to land on "No workspace", and Enter then picked it): it completes a path,
      // steps inside the highlighted folder, or does nothing. Shift+Tab is the user's own way out.
      if (e.shiftKey || mod || e.altKey) return 'none';
      if (s.mode === 'path') return 'complete';
      return s.selected >= 0 && s.selected < s.rows ? 'step-in' : 'swallow';
    case 'Backspace':
      return s.query === '' && !mod && !e.altKey ? 'up' : 'none';
    case 'Escape':
      return s.query ? 'clear' : 'close';
    default:
      return 'none';
  }
}