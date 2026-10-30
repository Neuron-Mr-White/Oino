/**
 * Board live updates v2 (WP-A22): `board_watch{on}` turns the host's file
 * watcher on, and `BoardChanged{project}` hub events arrive on the host event
 * stream (`events_open`, one per connection) — the SSE revision stream of the
 * daemon era is gone. Every event means "refetch": a debounced (250 ms, like
 * the web) `reload()` of the open project plus `loadProjects()` (sidebar
 * counts / running list); a change in another project refreshes only the
 * project list.
 *
 * The event stream can still end for good — the PC drops off, the host
 * restarts — so the controller reconnects with backoff (500 ms → 8 s, then
 * every 15 s, `reconnect.ts` timings, no give-up while visible), shows
 * `reconnecting`, and reloads once the stream is back (missed changes). It
 * goes `idle` (pull to refresh) only on non-retryable failures: the PC has
 * no board (`not_installed`) or the request itself is invalid.
 *
 * `useBoardLive` binds it to the Board screen: subscribed while the tab
 * shows a project and the page is visible; a hidden page, a project / PC
 * switch or leaving the tab unsubscribes; coming back resubscribes (the
 * connect-time catch-up reload covers what was missed).
 */
import { createEffect, onCleanup, createSignal, type Accessor } from 'solid-js';
import { BACKOFF_MS, STEADY_MS } from '../reconnect';
import { hostClient } from '../host/factory';
import type { HubEvent } from '@unipi/protocol';
import type { BoardAvailability } from './transport';
import type { BoardStore } from './store';
import { pcAvailability } from './pcs';

export const LIVE_DEBOUNCE_MS = 250;

export type LiveTarget = { hostId: string; slug: string };

export type BoardLiveOptions = {
  store: Pick<BoardStore, 'reload' | 'loadProjects' | 'setConn'>;
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
  const closeStream = () => {
    const off = unsubscribe;
    unsubscribe = undefined;
    off?.();
  };
  /** Turns the host's watcher off (best effort; the stream ends with it). */
  const watchOff = (hostId: string) => {
    void hostClient(hostId)
      .request('board_watch', { on: false })
      .catch(() => {});
  };

  let debounceTimerTasks = false;

  function schedule(gen: number, reloadTasks: boolean) {
    if (debounceTimer !== undefined) {
      // A pending reload of the open project outranks a counts-only refresh.
      debounceTimerTasks = debounceTimerTasks || reloadTasks;
      return;
    }
    debounceTimerTasks = reloadTasks;
    debounceTimer = setTimeout(() => {
      debounceTimer = undefined;
      if (gen !== generation) return;
      if (debounceTimerTasks) void store.reload();
      void store.loadProjects();
    }, debounceMs);
  }

  function connect() {
    const at = current;
    if (!at) return;
    closeStream();
    const gen = ++generation;
    store.setConn(attempt === 0 ? 'connecting' : 'reconnecting');
    const onEvent = (event: HubEvent) => {
      if (gen !== generation) return;
      if (event.kind !== 'board_changed') return;
      attempt = 0;
      store.setConn('live');
      schedule(gen, event.project === at.slug);
    };
    const onClosed = (error?: { message?: string }) => {
      if (gen !== generation) return;
      dropped(error?.message);
    };
    try {
      const stream = hostClient(at.hostId).openEvents(onEvent, onClosed);
      unsubscribe = () => stream.close();
    } catch (error) {
      dropped(String((error as Error)?.message ?? error));
      return;
    }
    // The stream ended synchronously inside openEvents(): drop its handle.
    if (gen !== generation) closeStream();
    void hostClient(at.hostId)
      .request('board_watch', { on: true })
      .then(() => {
        if (gen !== generation) return;
        attempt = 0;
        store.setConn('live');
        // No replay on connect: one catch-up reload covers what was missed.
        schedule(gen, true);
      })
      .catch((error: unknown) => {
        if (gen !== generation) return;
        dropped(String((error as Error)?.message ?? error));
      });
  }

  function dropped(message: string | undefined) {
    ++generation; // ignore anything else from this stream
    closeStream();
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
    closeStream();
    ++generation;
    attempt = 0;
    store.setConn('idle');
  }

  function stop() {
    const at = current;
    current = null;
    giveUp();
    if (at) watchOff(at.hostId);
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
