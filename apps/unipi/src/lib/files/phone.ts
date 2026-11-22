/**
 * Phone Files (phone spec §3.11, PH-24): pure helpers for the Places root and
 * the folder list — which places to offer, how a folder sorts and filters,
 * the breadcrumb and the row subtitle. The components live in
 * `components/files/PhoneFiles.tsx`.
 */
import type { FsEntry } from '../fs';
import { formatSize } from '../fs';

export type Place = { path: string; label: string; detail?: string; kind: 'home' | 'project' | 'other' };
export type FilesSort = 'name' | 'modified';

const trimSlash = (p: string) => (p.length > 1 ? p.replace(/\/+$/, '') : p);
const leaf = (p: string) => trimSlash(p).split('/').pop() || p;

/** Shows `~/x` for paths inside home. */
export function tildify(path: string, home: string | undefined): string {
  if (!home) return path;
  const h = trimSlash(home);
  if (path === h) return '~';
  return path.startsWith(`${h}/`) ? `~${path.slice(h.length)}` : path;
}

/**
 * Places (§3.11): Home first, then the PC's projects (chat folders, most
 * recent first, deduped, home itself excluded, at most 8), then the other
 * roots the host offers (`/`, `/tmp`, mounts) for a collapsed group.
 */
export function placesFrom(roots: string[], projects: string[]): { home?: Place; projects: Place[]; other: Place[] } {
  const home = roots[0];
  const seen = new Set<string>();
  const projectPlaces: Place[] = [];
  for (const raw of projects) {
    const p = trimSlash(raw);
    if (!p || seen.has(p) || (home && p === trimSlash(home)) || p === '~') continue;
    seen.add(p);
    projectPlaces.push({ path: p, label: leaf(p), detail: tildify(p, home), kind: 'project' });
    if (projectPlaces.length >= 8) break;
  }
  return {
    home: home ? { path: trimSlash(home), label: 'Home', detail: '~', kind: 'home' } : undefined,
    projects: projectPlaces,
    other: roots.slice(1).map((r) => ({ path: trimSlash(r), label: r === '/' ? 'Computer' : r, detail: r === '/' ? '/' : undefined, kind: 'other' as const })),
  };
}

/** Folders first, then by name (natural, case-insensitive) or newest first; dotfiles only when asked. */
export function folderView(entries: FsEntry[], opts: { hidden: boolean; sort: FilesSort }): FsEntry[] {
  const list = opts.hidden ? entries.slice() : entries.filter((e) => !e.name.startsWith('.'));
  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
  return list.sort((a, b) => {
    const da = a.kind === 'dir' ? 0 : 1;
    const db = b.kind === 'dir' ? 0 : 1;
    if (da !== db) return da - db;
    if (opts.sort === 'modified' && a.mtime !== b.mtime) return b.mtime - a.mtime;
    return collator.compare(a.name, b.name);
  });
}

/** Breadcrumb segments from the place the user started in (`root`) down to `path`. */
export function breadcrumbs(path: string, root: string | undefined, home: string | undefined): Array<{ path: string; label: string }> {
  const p = trimSlash(path);
  const base = root && (p === trimSlash(root) || p.startsWith(`${trimSlash(root)}/`)) ? trimSlash(root) : home && (p === trimSlash(home) || p.startsWith(`${trimSlash(home)}/`)) ? trimSlash(home) : '/';
  const crumbs: Array<{ path: string; label: string }> = [];
  crumbs.push({ path: base, label: home && base === trimSlash(home) ? '~' : base === '/' ? '/' : leaf(base) });
  const rest = p.slice(base === '/' ? 1 : base.length).split('/').filter(Boolean);
  let at = base;
  for (const seg of rest) {
    at = at === '/' ? `/${seg}` : `${at}/${seg}`;
    crumbs.push({ path: at, label: seg });
  }
  return crumbs;
}

/** "Yesterday", "Mon", "11:47", "12 Mar", "12 Mar 2023" from epoch ms (or seconds). */
export function relativeDate(mtime: number, now = Date.now()): string {
  const ms = mtime < 1e12 ? mtime * 1000 : mtime;
  const d = new Date(ms);
  const today = new Date(now);
  const startOfToday = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  if (ms >= startOfToday) return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  if (ms >= startOfToday - 86_400_000) return 'Yesterday';
  if (ms >= startOfToday - 6 * 86_400_000) return d.toLocaleDateString(undefined, { weekday: 'short' });
  if (d.getFullYear() === today.getFullYear()) return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
  return d.toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });
}

/** Row subtitle: "12 KB · Yesterday" (files), "Folder · Yesterday" (folders). */
export function entrySubtitle(e: FsEntry, now = Date.now()): string {
  const when = e.mtime ? relativeDate(e.mtime, now) : '';
  const what = e.kind === 'dir' ? 'Folder' : formatSize(e.size);
  return when ? `${what} · ${when}` : what;
}

/** The file-type family for the row icon. */
export type FileFamily = 'folder' | 'code' | 'text' | 'image' | 'media' | 'archive' | 'pdf' | 'data' | 'file';
export function fileFamily(e: Pick<FsEntry, 'name' | 'kind'>): FileFamily {
  if (e.kind === 'dir') return 'folder';
  const ext = e.name.toLowerCase().split('.').pop() ?? '';
  if (/^(png|jpe?g|gif|webp|svg|bmp|ico|avif|heic)$/.test(ext)) return 'image';
  if (/^(mp4|mov|webm|mkv|mp3|wav|ogg|flac|m4a)$/.test(ext)) return 'media';
  if (/^(zip|tar|gz|tgz|xz|bz2|7z|rar|zst)$/.test(ext)) return 'archive';
  if (ext === 'pdf') return 'pdf';
  if (/^(json|ya?ml|toml|csv|tsv|xml|ini|lock|sql|db)$/.test(ext)) return 'data';
  if (/^(md|mdx|txt|rst|log|org)$/.test(ext)) return 'text';
  if (/^(ts|tsx|js|jsx|mjs|cjs|rs|go|py|rb|java|kt|swift|c|h|cc|cpp|hpp|cs|php|sh|bash|zsh|fish|lua|css|scss|html|vue|svelte|dart|zig|nix|ex|exs|hs|ml|scala|r|pl)$/.test(ext)) return 'code';
  return 'file';
}
