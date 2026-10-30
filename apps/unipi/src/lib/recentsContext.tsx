/**
 * Recents + picker-state provider (DESIGN §3.1). Persists through
 * the same adapter pattern as settings: plugin-store `recents.json`
 * in Tauri, localStorage in tests. Invalid stored entries are
 * dropped silently.
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
import { createStore } from 'solid-js/store';
import type { AttachTarget } from '@unipi/protocol';
import { NOTE_KEY, createRecentsAdapter,
  recentKey,
  dedupeRecents,
  parseStoredPickerMux,
  parseStoredRecents,
  type PickerMux,
  type Recent,
} from './recents';
import { parseStoredSendHistory, pushSendHistory } from './composer';

export type RecentsApi = {
  recents: Recent[];
  addRecent: (recent: Recent) => void;
  renameLabel: (hostId: string, target: AttachTarget | null, label: string) => void;
  removeRecent: (hostId: string, target: AttachTarget | null) => void;
  pickerMuxFor: (hostId: string) => PickerMux | undefined;
  setPickerMux: (hostId: string, mux: PickerMux) => void;
  followFocusNoteDismissed: () => boolean;
  dismissFollowFocusNote: () => void;
  /** Composer send history for a host (DESIGN.md §3.5). */
  sendHistoryFor: (hostId: string) => string[];
  pushSend: (hostId: string, text: string) => void;
};

const RecentsContext = createContext<RecentsApi>();

export function RecentsProvider(props: { children: JSX.Element }): JSX.Element {
  const adapter = createRecentsAdapter();
  const [recents, setRecents] = createStore<{ list: Recent[] }>({ list: [] });
  const [pickerMux, setPickerMuxMap] = createStore<Record<string, PickerMux>>({});
  const [noteDismissed, setNoteDismissed] = createSignal(false);
  const [sendHistory, setSendHistory] = createStore<Record<string, string[]>>({});

  onMount(async () => {
    const stored = await adapter.read();
    if (stored != null && typeof stored === 'object' && !Array.isArray(stored)) {
      const blob = stored as {
        recents?: unknown;
        pickerMux?: unknown;
        [key: string]: unknown;
      };
      setRecents('list', parseStoredRecents(blob.recents));
      for (const [host, mux] of Object.entries(parseStoredPickerMux(blob.pickerMux))) {
        setPickerMuxMap(host, mux);
      }
      if (blob[NOTE_KEY] === true) setNoteDismissed(true);
      if (blob.sendHistory != null && typeof blob.sendHistory === 'object') {
        for (const [host, entries] of Object.entries(blob.sendHistory as Record<string, unknown>)) {
          setSendHistory(host, parseStoredSendHistory(entries));
        }
      }
    }
  });

  let writeTimer: ReturnType<typeof setTimeout> | undefined;
  const persist = (
    list: Recent[],
    muxMap: Record<string, PickerMux>,
    note: boolean,
    history: Record<string, string[]>,
  ) => {
    if (writeTimer) clearTimeout(writeTimer);
    // Deep-plain snapshot: Solid store proxies must not cross the IPC
    // bridge (they serialize as objects with numeric keys).
    const plain = JSON.parse(
      JSON.stringify({ recents: list, pickerMux: muxMap, [NOTE_KEY]: note, sendHistory: history }),
    );
    writeTimer = setTimeout(() => void adapter.write(plain), 150);
  };

  const writeNow = () => {
    persist(recents.list, { ...pickerMux }, noteDismissed(), JSON.parse(JSON.stringify(sendHistory)));
  };

  const addRecent = (recent: Recent) => {
    const list = dedupeRecents([recent, ...recents.list]);
    setRecents('list', list);
    writeNow();
  };

  const renameLabel = (hostId: string, target: AttachTarget | null, label: string) => {
    const key = recentKey(hostId, target);
    setRecents(
      'list',
      recents.list.map((recent) =>
        `${recent.hostId}::${JSON.stringify(recent.target)}` === key
          ? { ...recent, label }
          : recent,
      ),
    );
    writeNow();
  };

  const removeRecent = (hostId: string, target: AttachTarget | null) => {
    setRecents(
      'list',
      recents.list.filter((recent) => recentKey(recent.hostId, recent.target) !== recentKey(hostId, target)),
    );
    writeNow();
  };

  const api: RecentsApi = {
    get recents() {
      return recents.list;
    },
    addRecent,
    renameLabel,
    removeRecent,
    pickerMuxFor: (hostId) => pickerMux[hostId],
    setPickerMux: (hostId, mux) => {
      setPickerMuxMap(hostId, mux);
      writeNow();
    },
    followFocusNoteDismissed: noteDismissed,
    sendHistoryFor: (hostId) => sendHistory[hostId] ?? [],
    pushSend: (hostId, text) => {
      setSendHistory(hostId, pushSendHistory(sendHistory[hostId] ?? [], text));
      writeNow();
    },
    dismissFollowFocusNote: () => {
      setNoteDismissed(true);
      writeNow();
    },
  };

  createEffect(() => {
    void recents.list; // keep the store reference alive for tracking
  });

  return <RecentsContext.Provider value={api}>{props.children}</RecentsContext.Provider>;
}

export function useRecents(): RecentsApi {
  const api = useContext(RecentsContext);
  if (!api) throw new Error('useRecents() must be used within a <RecentsProvider>');
  return api;
}
