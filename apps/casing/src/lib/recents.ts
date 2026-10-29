/**
 * Recent terminals (DESIGN §3.1) plus picker UI state: last mux per
 * host, the one-time follow-focus note dismissal, and recents
 * persistence with the same adapter pattern as settings
 * (plugin-store file `recents.json`, localStorage in tests).
 */
import { isTauri } from './platform';
import { z } from 'zod';
import { AttachTargetSchema, type AttachTarget } from '@unipi/protocol';

/** Canonical key for a recent entry (host + target). */
export function recentKey(hostId: string, target: AttachTarget | null): string {
  return target === null ? `${hostId}::shell` : `${hostId}::${JSON.stringify(target)}`;
}

export const RecentSchema = z.object({
  hostId: z.string().min(1),
  target: AttachTargetSchema.nullable(),
  label: z.string().min(1).max(128),
  sublabel: z.string().max(128).default(''),
  openedAt: z.number().int(),
});
export type Recent = z.infer<typeof RecentSchema>;

export type PickerMux = 'herdr' | 'tmux';

const MAX_RECENTS = 6;
const RECENTS_KEY = 'recents';
const PICKER_MUX_KEY = 'picker-mux';
export const NOTE_KEY = 'herdr-follow-focus-note';


/** Distinguishes store files: one adapter instance per file. */
export type SimpleAdapter = {
  read(): Promise<unknown>;
  write(value: unknown): Promise<void>;
};

function localStorageAdapter(key: string): SimpleAdapter {
  return {
    async read() {
      const raw = globalThis.localStorage?.getItem(key);
      if (raw == null) return undefined;
      try {
        return JSON.parse(raw);
      } catch {
        return undefined;
      }
    },
    async write(value) {
      globalThis.localStorage?.setItem(key, JSON.stringify(value));
    },
  };
}

function pluginStoreAdapter(file: string): SimpleAdapter {
  return {
    async read() {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(file, { autoSave: false });
      return (await store.get(RECENTS_KEY)) as unknown;
    },
    async write(value) {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(file, { autoSave: false });
      await store.set(RECENTS_KEY, value);
      await store.save();
    },
  };
}

export function createRecentsAdapter(): SimpleAdapter {
  if (isTauri()) {
    return pluginStoreAdapter('recents.json');
  }
  return localStorageAdapter('unipi:recents');
}

function targetKey(hostId: string, target: AttachTarget | null): string {
  return `${hostId}::${target === null ? 'shell' : JSON.stringify(target)}`;
}

/** Keeps at most 6, newest first, deduped by hostId + JSON(target). */
export function dedupeRecents(recents: Recent[]): Recent[] {
  const seen = new Set<string>();
  const out: Recent[] = [];
  for (const recent of [...recents].sort((a, b) => b.openedAt - a.openedAt)) {
    const key = targetKey(recent.hostId, recent.target);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(recent);
    if (out.length >= MAX_RECENTS) break;
  }
  return out;
}

/** Parses stored recents, silently dropping invalid entries. */
export function parseStoredRecents(raw: unknown): Recent[] {
  if (!Array.isArray(raw)) return [];
  const out: Recent[] = [];
  for (const entry of raw) {
    const parsed = RecentSchema.safeParse(entry);
    if (parsed.success) out.push(parsed.data);
  }
  return dedupeRecents(out);
}

/** Picker mux per host: `Record<hostId, 'herdr' | 'tmux'>`. */
export function parseStoredPickerMux(raw: unknown): Record<string, PickerMux> {
  const out: Record<string, PickerMux> = {};
  if (raw != null && typeof raw === 'object' && !Array.isArray(raw)) {
    for (const [host, mux] of Object.entries(raw as Record<string, unknown>)) {
      if (mux === 'herdr' || mux === 'tmux') out[host] = mux;
    }
  }
  return out;
}
