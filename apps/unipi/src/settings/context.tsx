/**
 * SettingsProvider / useSettings — the one typed settings store for the
 * app (DESIGN.md §3.10, §5.5). `useSettings()` returns a
 * `[settings, update]` tuple, Solid-store style:
 *
 *   const [settings, update] = useSettings();
 *   update('terminal.fontSize', 18);
 *
 * `update` validates the single targeted key against its own schema
 * before writing. An invalid value is rejected with an `E_SETTINGS`
 * AppError and the store (and persisted file) are left untouched.
 */
import {
  type Accessor,
  type JSX,
  createContext,
  createEffect,
  createSignal,
  onMount,
  useContext,
} from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { appError, type AppError } from '@unipi/protocol';
import {
  DEFAULT_SETTINGS,
  SETTINGS_GROUP_SCHEMAS,
  type SettingsGroup,
  type SettingsV1,
} from './schema';
import { createDefaultAdapter, type SettingsAdapter } from './adapter';
import { loadSettingsFromAdapter } from './load';
import { applyAppearance } from './appearance';

const GROUP_SCHEMAS = SETTINGS_GROUP_SCHEMAS;
type GroupName = SettingsGroup;

function settingsError(message: string, path: (string | number)[]): AppError {
  return appError('E_SETTINGS', message, { path });
}

/** Splits `"terminal.fontSize"` into `["terminal", "fontSize"]`, validating shape. */
function parsePath(path: string): { group: GroupName; key: string } | undefined {
  const parts = path.split('.');
  if (parts.length !== 2) return undefined;
  const [group, key] = parts;
  if (!(group in GROUP_SCHEMAS)) return undefined;
  return { group: group as GroupName, key };
}

/**
 * Validates a single `group.key` update against the group's schema by
 * merging the candidate value onto the current group value and
 * reparsing. Returns the accepted value (post zod coercion/defaults) or
 * an AppError.
 */
function validateUpdate(
  current: SettingsV1,
  group: GroupName,
  key: string,
  value: unknown,
): { ok: true; value: unknown } | { ok: false; error: AppError } {
  const schema = GROUP_SCHEMAS[group];
  const currentGroup = current[group] as Record<string, unknown>;
  if (!(key in currentGroup)) {
    return { ok: false, error: settingsError(`unknown settings key: ${group}.${key}`, [group, key]) };
  }
  const candidate = { ...currentGroup, [key]: value };
  const parsed = schema.safeParse(candidate);
  if (!parsed.success) {
    const message = parsed.error.issues[0]?.message ?? 'invalid settings value';
    return { ok: false, error: settingsError(message, [group, key]) };
  }
  return { ok: true, value: (parsed.data as Record<string, unknown>)[key] };
}

export type UpdateFn = (path: string, value: unknown) => AppError | undefined;

export type SettingsApi = {
  /** Reactive settings object (Solid store). */
  settings: SettingsV1;
  /** Validates and applies a single `"group.key"` update; persists on success. */
  update: UpdateFn;
  /** Last error from a rejected `update()` call, or undefined. */
  error: Accessor<AppError | undefined>;
  /** Dotted-path keys that were reset to default on load (missing/invalid). */
  resetKeys: Accessor<string[]>;
  /** True once the persisted settings were newer than this build understands. */
  newerVersionNotice: Accessor<boolean>;
  /** Clears the current error/notice state (e.g. after showing a toast). */
  dismissNotice: () => void;
};

const SettingsContext = createContext<SettingsApi>();

export type SettingsProviderProps = {
  children: JSX.Element;
  /** Adapter injection point for tests; defaults to plugin-store/localStorage. */
  adapter?: SettingsAdapter;
};

export function SettingsProvider(props: SettingsProviderProps): JSX.Element {
  const adapter = props.adapter ?? createDefaultAdapter();
  const [store, setStore] = createStore<SettingsV1>(DEFAULT_SETTINGS);
  const [error, setError] = createSignal<AppError | undefined>(undefined);
  const [resetKeys, setResetKeys] = createSignal<string[]>([]);
  const [newerVersionNotice, setNewerVersionNotice] = createSignal(false);

  onMount(async () => {
    try {
      const result = await loadSettingsFromAdapter(adapter);
      setStore(result.settings);
      setResetKeys(result.resetKeys);
      setNewerVersionNotice(result.newerVersion);
    } catch (cause) {
      setError(settingsError(`Could not load settings: ${String(cause)}`, []));
    }
  });

  // Z-114 (WP-A24): the appearance vars follow the store app-wide (the root
  // html font size itself is never touched — only text that reads the vars).
  createEffect(() => {
    if (typeof document !== 'undefined') applyAppearance(document.documentElement.style, store.appearance);
  });

  const persist = (next: SettingsV1) => {
    void adapter.write(next).catch(cause => setError(settingsError(`Could not save settings: ${String(cause)}`, [])));
  };

  const update: UpdateFn = (path, value) => {
    const parsedPath = parsePath(path);
    if (!parsedPath) {
      const err = settingsError(`invalid settings path: ${path}`, [path]);
      setError(err);
      return err;
    }
    const { group, key } = parsedPath;
    const current = { ...store } as SettingsV1;
    const result = validateUpdate(current, group, key, value);
    if (!result.ok) {
      setError(result.error);
      return result.error;
    }

    setStore(
      produce((s) => {
        (s[group] as Record<string, unknown>)[key] = result.value;
      }),
    );
    setError(undefined);
    persist({ ...current, [group]: { ...(current[group] as object), [key]: result.value } });
    return undefined;
  };

  const api: SettingsApi = {
    get settings() {
      return store;
    },
    update,
    error,
    resetKeys,
    newerVersionNotice,
    dismissNotice: () => {
      setError(undefined);
      setNewerVersionNotice(false);
    },
  };

  return <SettingsContext.Provider value={api}>{props.children}</SettingsContext.Provider>;
}

/**
 * Returns `[settings, update]`, Solid-store style. `settings` is a
 * reactive store (read any field, it tracks). `update(path, value)`
 * validates then applies+persists, or returns an `AppError` (E_SETTINGS)
 * without touching the store.
 */
export function useSettings(): [SettingsV1, UpdateFn] {
  const api = useContext(SettingsContext);
  if (!api) {
    throw new Error('useSettings() must be used within a <SettingsProvider>');
  }
  return [api.settings, api.update];
}

/** Full API, including the error/reset/notice signals the shell needs for toasts. */
export function useSettingsApi(): SettingsApi {
  const api = useContext(SettingsContext);
  if (!api) {
    throw new Error('useSettingsApi() must be used within a <SettingsProvider>');
  }
  return api;
}


