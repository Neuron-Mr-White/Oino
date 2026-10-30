/**
 * Files tree model (app spec WP-A20, Z-101): a lazy, virtualized tree over
 * `fs_list`. Pure data + pure functions so the shape is testable without a
 * host; `FileTree.tsx` owns the reactive store that feeds it.
 *
 * Behaviours (ZCode `docs/ui/workspace-file-tree.md` via the audit):
 * lazy `fs_list` per expanded dir, **compact folders** (a chain of
 * single-child directories renders as one `a/b` row), dotfiles shown,
 * git-changed filter + badges, gitignored rows dimmed, fuzzy name search
 * ranking. Rows are flattened (depth kept per row) so the view is one
 * virtual list — a 50 k-entry tree flattens in a single pass.
 */
import { joinPath } from '../fs';

export type GitMark = 'M' | 'A' | 'D' | '?';

/** One `fs_list` entry (proto `FsEntry`). */
export type TreeEntry = {
  name: string;
  kind: 'file' | 'dir' | 'symlink';
  size: number;
  mtime: number;
  git?: GitMark;
  ignored?: boolean;
};

export type DirState = { status: 'idle' | 'loading' | 'ready' | 'error'; entries?: TreeEntry[]; error?: string; more?: boolean };

export type TreeModel = {
  /** Absolute root paths (the project folder, or the host roots for `~`). */
  roots: string[];
  /** Expanded dir paths. A compact `a/b` row is keyed by the chain's end. */
  expanded: ReadonlySet<string>;
  /** Listing state per dir path. */
  dirs: ReadonlyMap<string, DirState>;
};

export const emptyModel = (roots: string[] = []): TreeModel => ({ roots, expanded: new Set(), dirs: new Map() });

export type TreeRow = {
  /** Full path (a dir row: the compact chain's deepest dir). */
  path: string;
  /** Display name (a compact chain joins with `/`). */
  name: string;
  depth: number;
  kind: 'root' | 'dir' | 'file' | 'symlink';
  git?: GitMark;
  ignored?: boolean;
  size?: number;
  /** Dir rows: expanded / children still loading / listing failed / empty. */
  expanded?: boolean;
  loading?: boolean;
  error?: string;
  empty?: boolean;
  /** `fs_list` hit its cap: the folder has more children than listed. */
  more?: boolean;
};

/** `fs_list` row order: directories first, then case-insensitive by name. */
export function sortEntries(entries: readonly TreeEntry[]): TreeEntry[] {
  return [...entries].sort((a, b) => {
    const aDir = a.kind === 'dir';
    const bDir = b.kind === 'dir';
    if (aDir !== bDir) return aDir ? -1 : 1;
    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
  });
}

/** True when `dir` is listed and has exactly one child, a directory (compactible). */
function compactible(model: TreeModel, path: string): string | undefined {
  const state = model.dirs.get(path);
  if (!state || state.status !== 'ready' || !state.entries) return undefined;
  const entries = sortEntries(state.entries);
  if (entries.length !== 1 || entries[0]!.kind !== 'dir') return undefined;
  return joinPath(path, entries[0]!.name);
}

/**
 * Pure: the visible rows of the tree, depth-first, compact folders merged.
 * A single root is implicit (its children at depth 0); several roots (the
 * `~` scope) each get a root row.
 */
export function flatten(model: TreeModel): TreeRow[] {
  const rows: TreeRow[] = [];
  const single = model.roots.length === 1 ? model.roots[0] : undefined;
  if (single !== undefined) {
    children(model, single, 0, rows);
    return rows;
  }
  for (const root of model.roots) {
    rows.push(dirRow(model, root, root, 0, true));
    if (model.expanded.has(root)) children(model, root, 1, rows);
  }
  return rows;
}

function dirRow(model: TreeModel, path: string, name: string, depth: number, isRoot: boolean): TreeRow {
  const state = model.dirs.get(path);
  const row: TreeRow = { path, name, depth, kind: isRoot ? 'root' : 'dir', expanded: model.expanded.has(path) };
  if (state?.status === 'loading') row.loading = true;
  else if (state?.status === 'error') row.error = state.error;
  else if (state?.status === 'ready') {
    if (!state.entries?.length) row.empty = true;
    if (state.more) row.more = true;
  }
  return row;
}

function children(model: TreeModel, dir: string, depth: number, rows: TreeRow[]) {
  const state = model.dirs.get(dir);
  if (!state || state.status !== 'ready' || !state.entries) return;
  for (const entry of sortEntries(state.entries)) {
    const path = joinPath(dir, entry.name);
    if (entry.kind === 'dir') {
      // Compact folders: absorb a chain of single-dir children into one row.
      let end = path;
      let name = entry.name;
      for (let guard = 0; guard < 64; guard++) {
        const next = compactible(model, end);
        if (next === undefined) break;
        name += `/${next.slice(next.lastIndexOf('/') + 1)}`;
        end = next;
      }
      const row = dirRow(model, end, name, depth, false);
      row.git = entry.git;
      row.ignored = entry.ignored;
      rows.push(row);
      if (model.expanded.has(end)) children(model, end, depth + 1, rows);
    } else {
      rows.push({ path, name: entry.name, depth, kind: entry.kind, git: entry.git, ignored: entry.ignored, size: entry.size });
    }
  }
}

/**
 * Pure: the changed-only view (Z-101 git-changed filter): every *loaded*
 * entry with a git mark, as a flat list of paths relative to their root —
 * like unipi's v1 filter, over the lazy tree. Untracked (`?`) dirs stay
 * expandable rows so their contents can load.
 */
export function flattenChanged(model: TreeModel): TreeRow[] {
  const out: TreeRow[] = [];
  const rootOf = (path: string): string | undefined => model.roots.find((r) => path === r || path.startsWith(`${r}/`));
  const rel = (path: string): string => {
    const root = rootOf(path);
    if (root === undefined) return path;
    const rest = path.slice(root.length);
    return rest.startsWith('/') ? rest.slice(1) : rest;
  };
  const walk = (dir: string) => {
    const state = model.dirs.get(dir);
    if (!state || state.status !== 'ready' || !state.entries) return;
    for (const entry of sortEntries(state.entries)) {
      const path = joinPath(dir, entry.name);
      if (entry.kind === 'dir') {
        // Descend every loaded dir — a change deep under an unmarked
        // folder must still surface once its dir is expanded.
        if (entry.git) out.push({ ...dirRow(model, path, rel(path), 0, false), git: entry.git, ignored: entry.ignored, expanded: model.expanded.has(path) });
        walk(path);
      } else if (entry.git) {
        out.push({ path, name: rel(path), depth: 0, kind: entry.kind, git: entry.git, ignored: entry.ignored, size: entry.size });
      }
    }
  };
  for (const root of model.roots) walk(root);
  out.sort((a, b) => a.path.toLowerCase().localeCompare(b.path.toLowerCase()));
  return out;
}

/**
 * Fuzzy subsequence score for name search (ZCode-style): 0 = no match;
 * higher = better. Bonuses for word starts, path separators and contiguous
 * runs; earlier matches win.
 */
export function fuzzyScore(query: string, target: string): number {
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  if (!q) return 1;
  if (q === t) return 1e6;
  let score = 0;
  let ti = 0;
  let run = 0;
  for (let qi = 0; qi < q.length; qi++) {
    const ch = q[qi]!;
    if (ch === ' ') continue;
    let at = t.indexOf(ch, ti);
    if (at < 0) return 0;
    if (at === ti && qi > 0) score += 8 + run * 2; // contiguous
    else score += 1;
    const prev = at > 0 ? t[at - 1]! : '/';
    if (prev === '/' || prev === '.' || prev === '_' || prev === '-' || prev === ' ') score += 6; // word start
    if (at === 0) score += 4;
    run = at === ti ? run + 1 : 0;
    ti = at + 1;
  }
  score += Math.max(0, 24 - (t.length - q.length)); // shorter targets win
  return score;
}

export type NameHit = { path: string; score: number };

/** The dirs whose listing the view must fetch: the roots plus every expanded dir that never loaded (or failed). */
export function neededListings(model: TreeModel): string[] {
  const out: string[] = [];
  const consider = (path: string) => {
    const state = model.dirs.get(path);
    if (!state || state.status === 'idle' || state.status === 'error') out.push(path);
  };
  const walkExpanded = (dir: string) => {
    consider(dir);
    const state = model.dirs.get(dir);
    if (state?.status === 'ready' && state.entries) {
      for (const entry of state.entries) {
        if (entry.kind !== 'dir') continue;
        const path = joinPath(dir, entry.name);
        // Compact chains: the row's path is the chain end; intermediate dirs never expand.
        let end = path;
        for (let guard = 0; guard < 64; guard++) {
          const next = compactible(model, end);
          if (next === undefined) break;
          end = next;
        }
        if (model.expanded.has(end)) walkExpanded(end);
      }
    }
  };
  for (const root of model.roots) {
    if (model.roots.length === 1) walkExpanded(root);
    else {
      consider(root);
      if (model.expanded.has(root)) walkExpanded(root);
    }
  }
  return [...new Set(out)];
}

/** Pure: fuzzy-rank `paths` by their basename for `query` (best first). */
export function rankByName(query: string, paths: readonly string[]): NameHit[] {
  return paths
    .map((path) => ({ path, score: fuzzyScore(query, path.slice(path.lastIndexOf('/') + 1)) + fuzzyScore(query, path) / 4 }))
    .filter((hit) => hit.score > 0)
    .sort((a, b) => b.score - a.score || a.path.length - b.path.length);
}
