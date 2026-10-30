/**
 * Filesystem browser/editor (UNI-155 "files — browser + editor"): the UI's
 * one seam onto the `fs_*` Tauri commands (`apps/mobile/src-tauri/src/fs.rs`
 * → `unipi-host`'s `fs_*` requests, `crates/host/src/fs.rs`).
 */
import { z } from 'zod';
import { call } from './ipc';
import { FsEntrySchema, FsListDataSchema, FsSearchDataSchema, FsStatDataSchema, FsWriteDataSchema } from '@unipi/protocol';

export type FsEntry = z.infer<typeof FsEntrySchema>;

const FsRootsResultSchema = z.object({ roots: z.array(z.string()) });

export async function fsRoots(hostId: string) {
  return call('fs_roots', { hostId }, FsRootsResultSchema);
}

export async function fsList(hostId: string, path: string, hidden?: boolean, outside?: boolean) {
  return call('fs_list', { hostId, path, hidden, outside }, FsListDataSchema);
}

export async function fsStat(hostId: string, path: string, outside?: boolean) {
  return call('fs_stat', { hostId, path, outside }, FsStatDataSchema);
}

export async function fsMkdir(hostId: string, path: string, outside?: boolean) {
  return call('fs_mkdir', { hostId, path, outside }, z.unknown());
}

export async function fsRename(hostId: string, from: string, to: string, outside?: boolean) {
  return call('fs_rename', { hostId, from, to, outside }, z.unknown());
}

export async function fsDelete(hostId: string, path: string, confirm: boolean, outside?: boolean) {
  return call('fs_delete', { hostId, path, confirm, outside }, z.unknown());
}

export async function fsSearch(hostId: string, root: string, query: string, kind: 'name' | 'content', outside?: boolean) {
  return call('fs_search', { hostId, root, query, kind, outside }, FsSearchDataSchema);
}

const FsReadResultSchema = z.object({
  size: z.number(),
  hash: z.string(),
  mtime: z.number(),
  truncated: z.boolean(),
  text: z.string(),
});
export type FsReadResult = z.infer<typeof FsReadResultSchema>;

export async function fsRead(hostId: string, path: string, outside?: boolean, asText?: boolean) {
  return call('fs_read', { hostId, path, outside, asText }, FsReadResultSchema);
}

export async function fsWrite(
  hostId: string,
  path: string,
  text: string,
  baseHash: string | undefined,
  create: boolean | undefined,
  outside?: boolean,
) {
  return call('fs_write', { hostId, path, text, baseHash, create, outside }, FsWriteDataSchema);
}

/** UNI-199: copies an already-uploaded blob (`blobTransport().putBytes`)
 * to a host path ("upload, choose the destination"). */
export async function fsImport(hostId: string, sha256: string, to: string, overwrite?: boolean, outside?: boolean) {
  return call('fs_import', { hostId, sha256, to, overwrite, outside }, z.unknown());
}

/** The default "Upload to…" destination: a shared folder the host
 * creates on demand, so an upload never needs a browse round trip. */
export const DEFAULT_UPLOAD_DIR = '~/.unipi/uploads';

/** A directory's basename, or the whole path for `/`. */
export function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  if (trimmed === '') return '/';
  const idx = trimmed.lastIndexOf('/');
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}

/** The parent directory of `path`, or `path` itself at the filesystem root. */
export function dirname(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const idx = trimmed.lastIndexOf('/');
  if (idx <= 0) return '/';
  return trimmed.slice(0, idx);
}

/** Joins a directory and a child name into a path (handles trailing slash). */
export function joinPath(dir: string, name: string): string {
  if (dir.endsWith('/')) return `${dir}${name}`;
  return `${dir}/${name}`;
}

/** Recently opened files, newest first, capped (localStorage, UNI-155). */
const RECENTS_KEY = 'unipi.files.recent';
const RECENTS_CAP = 20;

export type RecentFile = { hostId: string; path: string; openedAt: number };

export function readRecents(): RecentFile[] {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((r) => r && typeof r.hostId === 'string' && typeof r.path === 'string' && typeof r.openedAt === 'number');
  } catch {
    return [];
  }
}

export function pushRecent(hostId: string, path: string): void {
  const existing = readRecents().filter((r) => !(r.hostId === hostId && r.path === path));
  existing.unshift({ hostId, path, openedAt: Date.now() });
  try {
    localStorage.setItem(RECENTS_KEY, JSON.stringify(existing.slice(0, RECENTS_CAP)));
  } catch {
    // localStorage unavailable (private mode, quota): recents are best-effort.
  }
}

/** Formats a byte count the way the Files list shows sizes. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

/** Formats a ms-since-epoch mtime relative to now, short form. */
export function formatMtime(ms: number): string {
  const diff = Date.now() - ms;
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (diff < minute) return 'just now';
  if (diff < hour) return `${Math.floor(diff / minute)}m ago`;
  if (diff < day) return `${Math.floor(diff / hour)}h ago`;
  if (diff < 30 * day) return `${Math.floor(diff / day)}d ago`;
  return new Date(ms).toLocaleDateString();
}
