/**
 * Board live updates (UNI-132) — the phone's take on the kanboard web
 * UI's `watchBoard` (crates/kanboard/web/src/state.ts): one revision
 * stream per open project (`GET /events?project=<slug>` through the
 * board transport). The daemon sends the current revision on connect
 * and one `revision` event per board change (file watcher, so CLI and
 * agent writes show up too); the payload is just a counter, so every
 * event means "refetch": a debounced (250 ms, like the web) `reload()`
 * of the tasks plus `loadProjects()` (sidebar counts / running list).
 *
 * Unlike the web (EventSource retries by itself) the stream here can
 * end for good — the daemon idles out, the host restarts it, the PC
 * drops off — so the controller reconnects with backoff (500 ms → 8 s,
 * then every 15 s, `reconnect.ts` timings, no give-up while visible),
 * shows `reconnecting`, and reloads once the stream is back (missed
 * revisions). It goes `idle` (pull to refresh) only on non-retryable
 * failures: the PC has no board (`not_installed`) or the request itself
 * is invalid.
 *
 * `useBoardLive` binds it to the Board screen: subscribed while the tab
 * shows a project and the page is visible; a hidden page, a project /
 * PC switch or leaving the tab unsubscribes; coming back resubscribes
 * (the connect revision triggers the one catch-up reload).
 */
import { createEffect, onCleanup, createSignal, type Accessor } from 'solid-js';
import { BACKOFF_MS, STEADY_MS } from '../reconnect';
import { boardTransport, type BoardAvailability, type BoardEvent, type BoardStreamState, type BoardTransport } from './transport';
import type { BoardStore } from './store';
import { pcAvailability } from './pcs';

export const LIVE_DEBOUNCE_MS = 250;

export type LiveTarget = { hostId: string; slug: string };

export type BoardLiveOptions = {
  store: Pick<BoardStore, 'reload' | 'loadProjects' | 'setConn'>;
  transport?: () => BoardTransport;
  /** Re-checks the PC before a retry (default: the shared PC availability, so the chip dot follows). */
  checkAvailability?: (hostId: string) => Promise<BoardAvailability>;
  debounceMs?: number;
};

export type BoardLive = {
  /** Subscribes to `target` (null = nothing to watch). Same target = no-op. */
  watch: (target: LiveTarget | null) => void;
  /** Unsubscribes and cancels pending work (conn → idle). */
  stop: () => void;
};

/** Stream failures that retrying cannot fix. */
export function nonRetryable(message: string | undefined): boolean {
  if (!message) return false;
  return /not[ _-]?installed|unknown (pc|host)|not paired|invalid project|schema|unsupported/i.test(message);
}

export function backoffDelay(attempt: number): number {
  return attempt <= BACKOFF_MS.length ? BACKOFF_MS[attempt - 1]! : STEADY_MS;
}

export function createBoardLive(options: BoardLiveOptions): BoardLive {
  const { store } = options;
  const transport = options.transport ?? boardTransport;
  const check = options.checkAvailability ?? ((hostId: string) => pcAvailability().check(hostId));
  const debounceMs = options.debounceMs ?? LIVE_DEBOUNCE_MS;

  let current: LiveTarget | null = null;
  /** Bumped on every (re)subscribe / stop: callbacks of older streams are ignored. */
  let generation = 0;
  let unsubscribe: (() => void) | undefined;
  let attempt = 0;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  const clearTimers = () => {
    if (retryTimer !== undefined) clearTimeout(retryTimer);
    if (debounceTimer !== undefined) clearTimeout(debounceTimer);
    retryTimer = debounceTimer = undefined;
  };
  const close = () => {
    const off = unsubscribe;
    unsubscribe = undefined;
    off?.();
  };

  function schedule(gen: number) {
    if (debounceTimer !== undefined) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = undefined;
      if (gen !== generation) return;
      void store.reload();
      void store.loadProjects();
    }, debounceMs);
  }

  function connect() {
    const at = current;
    if (!at) return;
    close();
    const gen = ++generation;
    store.setConn(attempt === 0 ? 'connecting' : 'reconnecting');
    const onEvent = (event: BoardEvent) => {
      if (gen !== generation) return;
      if (event.event && event.event !== 'revision' && event.event !== 'message') return;
      attempt = 0;
      store.setConn('live');
      schedule(gen);
    };
    const onState = (state: BoardStreamState, message?: string) => {
      if (gen !== generation) return;
      if (state === 'open') {
        attempt = 0;
        store.setConn('live');
        // The daemon's connect revision follows and coalesces with this.
        schedule(gen);
        return;
      }
      dropped(message);
    };
    try {
      unsubscribe = transport().events(at.hostId, at.slug, onEvent, onState);
    } catch (error) {
      dropped(String((error as Error)?.message ?? error));
      return;
    }
    // The stream ended synchronously inside events(): drop its handle.
    if (gen !== generation) close();
  }

  function dropped(message: string | undefined) {
    ++generation; // ignore anything else from this stream
    close();
    if (nonRetryable(message)) return giveUp();
    attempt += 1;
    store.setConn('reconnecting');
    const delay = backoffDelay(attempt);
    if (retryTimer !== undefined) clearTimeout(retryTimer);
    const at = current;
    const retryGen = generation;
    retryTimer = setTimeout(() => {
      retryTimer = undefined;
      if (retryGen !== generation || current !== at || !at) return;
      void check(at.hostId)
        .catch((): BoardAvailability => ({ state: 'offline' }))
        .then((availability) => {
          if (retryGen !== generation || current !== at) return;
          if (availability.state === 'not_installed') return giveUp();
          if (availability.state === 'ok') return connect();
          // Offline / transient error: keep backing off.
          dropped(availability.state === 'error' ? availability.message : undefined);
        });
    }, delay);
  }

  function giveUp() {
    clearTimers();
    close();
    ++generation;
    attempt = 0;
    store.setConn('idle');
  }

  function stop() {
    current = null;
    giveUp();
  }

  function watch(target: LiveTarget | null) {
    if (target && current && target.hostId === current.hostId && target.slug === current.slug) return;
    stop();
    if (!target) return;
    current = { ...target };
    connect();
  }

  return { watch, stop };
}

/** `document.visibilityState === 'visible'` as a signal. */
export function createPageVisible(): Accessor<boolean> {
  const visible = () => typeof document === 'undefined' || document.visibilityState !== 'hidden';
  const [get, set] = createSignal(visible());
  if (typeof document !== 'undefined') {
    const onChange = () => set(visible());
    document.addEventListener('visibilitychange', onChange);
    onCleanup(() => document.removeEventListener('visibilitychange', onChange));
  }
  return get;
}

/**
 * Board screen binding: watches `target()` while the page is visible;
 * stops on hide and when the owner (the Board tab) unmounts.
 */
export function useBoardLive(store: BoardStore, target: Accessor<LiveTarget | null>, options: Omit<BoardLiveOptions, 'store'> = {}): BoardLive {
  const live = createBoardLive({ ...options, store });
  const visible = createPageVisible();
  createEffect(() => {
    const at = target();
    live.watch(visible() && at ? { hostId: at.hostId, slug: at.slug } : null);
  });
  onCleanup(() => live.stop());
  return live;
}
