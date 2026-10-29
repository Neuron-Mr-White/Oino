/**
 * Files navigation memory (UNI-167): the last browsed folder per host,
 * so returning to Files (after the editor/previewer round-trip, or an
 * app restart) reopens where the user left off instead of the roots.
 * Plus the pure Back-stack decision for the Files screen: parent folder
 * first, then the chat Files was opened from, then the app home.
 */
import { dirname } from './fs';

const LAST_FOLDER_KEY = 'unipi.files.lastFolder';

type FolderMap = { [hostId: string]: string };

function readMap(): FolderMap {
  try {
    const raw = localStorage.getItem(LAST_FOLDER_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (parsed && typeof parsed === 'object') return parsed as FolderMap;
    return {};
  } catch {
    // localStorage unavailable or corrupt: best-effort, start empty.
    return {};
  }
}

/** Remembers the folder the user is browsing on a host (best-effort). */
export function rememberFolder(hostId: string, path: string): void {
  try {
    const map = readMap();
    map[hostId] = path;
    localStorage.setItem(LAST_FOLDER_KEY, JSON.stringify(map));
  } catch {
    // Best-effort only (private mode, quota).
  }
}

/** The last browsed folder on a host, if any. */
export function lastFolder(hostId: string): string | undefined {
  return readMap()[hostId];
}

/**
 * Where Files opens on a host: the remembered folder when it is still under
 * one of the host's roots, else the first root. A folder remembered from an
 * older host/scope (e.g. `/home/me` after the host's HOME changed) would only
 * show the host's raw "outside the allowed scope" error (UNI-218 F63).
 */
export function startFolder(hostId: string, roots: readonly string[]): string | undefined {
  const last = lastFolder(hostId);
  return last && activeRoot(last, roots) ? last : roots[0];
}

export type FilesBack =
  | { action: 'parent'; to: string }
  | { action: 'chat' }
  | { action: 'home' };

/**
 * What Android/header Back does on the Files screen:
 * - inside a host tree (not at a root): up one folder;
 * - at a root (or the roots list): back to the chat Files was opened
 *   from, when there is one; otherwise the app home.
 */
export function nextFilesBack(state: { path: string | undefined; roots: string[]; hasReturnChat: boolean }): FilesBack {
  const { path, roots, hasReturnChat } = state;
  if (path === undefined || path === '/' || roots.includes(path)) {
    return hasReturnChat ? { action: 'chat' } : { action: 'home' };
  }
  return { action: 'parent', to: dirname(path) };
}

/** The root `path` is browsing (the longest root containing it), if any. */
export function activeRoot(path: string | undefined, roots: readonly string[]): string | undefined {
  if (!path) return undefined;
  let best: string | undefined;
  for (const root of roots) {
    const within = path === root || path.startsWith(root === '/' ? '/' : `${root}/`);
    if (within && (!best || root.length > best.length)) best = root;
  }
  return best;
}

/**
 * Label for a root pill (UNI-220). The host lists `$HOME` first, then its
 * extra safe roots (`/tmp`, `/mnt`, …), then live session folders:
 * home is `~`, a folder under home its name, anything else its full path
 * (so `/tmp` and `/var/tmp` read as what they are).
 */
export function rootLabel(root: string, home: string | undefined): string {
  if (home && root === home) return '~';
  if (home && root.startsWith(`${home}/`)) return root.slice(root.lastIndexOf('/') + 1);
  return root;
}

/** One breadcrumb: a label (`~` for home) and the folder it opens. */
export type Crumb = { label: string; path: string; root?: boolean };

/**
 * Breadcrumb for `path` (UNI-218 F38): starts at the root being browsed
 * (`~`, `/tmp`, `proj`…), never at `/`, so the first crumb is always a folder
 * the host lets you open and an ancestor of the shown one. A path outside
 * every root (shouldn't happen) falls back to the full path from `/`.
 */
export function crumbsFor(path: string | undefined, roots: readonly string[]): Crumb[] {
  if (!path) return [];
  const root = activeRoot(path, roots);
  const out: Crumb[] = [];
  let acc: string;
  let rest: string;
  if (root) {
    out.push({ label: root === '/' ? '/' : rootLabel(root, roots[0]), path: root, root: true });
    acc = root === '/' ? '' : root;
    rest = path.slice(root.length);
  } else {
    out.push({ label: '/', path: '/', root: true });
    acc = '';
    rest = path;
  }
  for (const part of rest.split('/').filter(Boolean)) {
    acc += `/${part}`;
    out.push({ label: part, path: acc });
  }
  return out;
}

/** Places beyond this many (or with long labels) collapse into one menu pill (UNI-218). */
export const PLACES_INLINE_MAX = 3;
const PLACES_INLINE_CHARS = 24;

/**
 * How the Places control renders: a few short roots as inline pills in the
 * Files toolbar, otherwise one "current place ▾" pill that opens a menu —
 * so many roots never turn the toolbar into a sideways-scrolling strip.
 */
export function placesLayout(roots: readonly string[], max = PLACES_INLINE_MAX): 'none' | 'inline' | 'menu' {
  if (roots.length <= 1) return 'none';
  const chars = roots.reduce((n, r) => n + rootLabel(r, roots[0]).length, 0);
  return roots.length <= max && chars <= PLACES_INLINE_CHARS ? 'inline' : 'menu';
}
