/**
 * HostsProvider / useHosts — the one shared "paired hosts" store
 * (DESIGN.md §3.10, §5.3). Sessions, Settings and the Add-host flow all
 * read and refresh through this context, so there is one source of
 * truth and one in-flight refresh at a time.
 */
import { type Accessor, type JSX, createContext, createSignal, onMount, useContext } from 'solid-js';
import type { AppError } from '@unipi/protocol';
import { call } from '../lib/ipc';
import { HostInfoSchema, type HostInfo, type PathKind } from '../lib/schemas';
import { localIdOf, setLocalHostId } from '../lib/host/localId';
import { z } from 'zod';
import { useSettings } from '../settings/context';
import { markAppReady } from '../lib/splash';

const ListHostsSchema = z.array(HostInfoSchema);

export type HostsApi = {
  paths:Accessor<Record<string,PathKind>>;
  setPath:(id:string,path:PathKind)=>void;
  /** Paired hosts, most-recently-paired order (as returned by `list_hosts`). */
  hosts: Accessor<HostInfo[]>;
  /** True while a `list_hosts` call is in flight. */
  loading: Accessor<boolean>;
  /** The last `list_hosts` error, or undefined. */
  error: Accessor<AppError | undefined>;
  /** Re-fetches the host list from the core. */
  refresh: () => Promise<void>;
  /** The globally selected host (sidebar host row; Board, Cron, Preview, New chat follow it), or undefined if none/empty. */
  selectedId: Accessor<string | undefined>;
  selectHost: (id: string | undefined) => void;
  /** Forgets a paired host, then refreshes the list. */
  forget: (id: string) => Promise<AppError | undefined>;
};

const HostsContext = createContext<HostsApi>();

const SELECTED_KEY = 'unipi.selectedHost';
function readSelectedHost(): string | undefined {
  try {
    return localStorage.getItem(SELECTED_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}
function writeSelectedHost(id: string | undefined): void {
  try {
    if (id) localStorage.setItem(SELECTED_KEY, id);
    else localStorage.removeItem(SELECTED_KEY);
  } catch {
    /* storage unavailable: the selection just isn't remembered */
  }
}

export function HostsProvider(props: { children: JSX.Element }): JSX.Element {
  const [settings] = useSettings();
  const [paths,setPaths]=createSignal<Record<string,PathKind>>({});
  const [hosts, setHosts] = createSignal<HostInfo[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<AppError | undefined>(undefined);
  // The one global host selection (sidebar host row): remembered across launches.
  const [selectedId, setSelectedIdRaw] = createSignal<string | undefined>(readSelectedHost());
  const setSelectedId = (id: string | undefined) => {
    setSelectedIdRaw(id);
    writeSelectedHost(id);
  };

  const refresh = async () => {
    setLoading(true);
    const result = await call('list_hosts', {}, ListHostsSchema);
    setLoading(false);
    if (result.ok) {
      // The `local` alias resolves to This computer's real id app-wide (lib/host/localId).
      setLocalHostId(localIdOf(result.value));
      setHosts(result.value);
      setError(undefined);
      const current = selectedId();
      if (!current || !result.value.some((h) => h.id === current)) {
        // This computer first, else the most recently paired PC.
        setSelectedId((result.value.find((h) => h.transport === 'local') ?? result.value[0])?.id);
      }
    } else {
      setError(result.error);
    }
  };

  const forget = async (id: string): Promise<AppError | undefined> => {
    const result = await call('forget_host', { id }, z.unknown());
    if (!result.ok) return result.error;
    await refresh();
    return undefined;
  };

  onMount(() => {
    void refresh().finally(markAppReady);
  });

  const api: HostsApi = {
    paths,
    setPath:(id,path)=>setPaths(previous=>({...previous,[id]:path})),
    hosts,
    loading,
    error,
    refresh,
    selectedId,
    selectHost: setSelectedId,
    forget,
  };

  // `settings` is read so a relay-mode change can be wired in later
  // (relay options are passed per-call, not stored here); referencing
  // it keeps the dependency explicit without affecting behaviour.
  void settings.connection.relay;

  return <HostsContext.Provider value={api}>{props.children}</HostsContext.Provider>;
}

/** The hosts store when inside a <HostsProvider>, else undefined (shared widgets that also render standalone). */
export function useHostsOptional(): HostsApi | undefined {
  return useContext(HostsContext);
}

export function useHosts(): HostsApi {
  const api = useContext(HostsContext);
  if (!api) throw new Error('useHosts() must be used within a <HostsProvider>');
  return api;
}
