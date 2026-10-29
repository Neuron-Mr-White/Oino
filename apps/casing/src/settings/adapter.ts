/**
 * Storage adapter for raw settings JSON. Same interface in production
 * (tauri-plugin-store, settings.json) and in tests/browser (localStorage),
 * so the rest of the settings module never has to know which one it's
 * talking to (DESIGN.md §3.10).
 */
import { isTauri } from '../lib/platform';
export interface SettingsAdapter {
  /** Read the raw persisted value (parsed JSON), or undefined if absent. */
  read(): Promise<unknown>;
  /** Persist the raw value as JSON. */
  write(value: unknown): Promise<void>;
}

const STORE_FILE = 'settings.json';
const STORE_KEY = 'settings';
const LOCAL_STORAGE_KEY = 'unipi:settings';


/** tauri-plugin-store backed adapter: settings.json in the app data dir. */
export function createPluginStoreAdapter(): SettingsAdapter {
  return {
    async read() {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(STORE_FILE, { autoSave: false });
      return await store.get(STORE_KEY);
    },
    async write(value: unknown) {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(STORE_FILE, { autoSave: false });
      await store.set(STORE_KEY, value);
      await store.save();
    },
  };
}

/** localStorage-backed adapter, used in the browser and in tests. */
export function createLocalStorageAdapter(storage: Storage = globalThis.localStorage): SettingsAdapter {
  return {
    async read() {
      const raw = storage.getItem(LOCAL_STORAGE_KEY);
      if (raw == null) return undefined;
      try {
        return JSON.parse(raw);
      } catch {
        return undefined;
      }
    },
    async write(value: unknown) {
      storage.setItem(LOCAL_STORAGE_KEY, JSON.stringify(value));
    },
  };
}

/** In-memory adapter, useful for isolated unit tests with no storage at all. */
export function createMemoryAdapter(initial?: unknown): SettingsAdapter {
  let value = initial;
  return {
    async read() {
      return value;
    },
    async write(next: unknown) {
      value = next;
    },
  };
}

/** Picks the plugin-store adapter under Tauri, else localStorage (browser/tests). */
export function createDefaultAdapter(): SettingsAdapter {
  return isTauri() ? createPluginStoreAdapter() : createLocalStorageAdapter();
}
