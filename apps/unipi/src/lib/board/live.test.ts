/**
 * UNI-132 v2 (WP-A22): the live-updates controller against a fake host —
 * `board_watch{on}` + `board_changed` hub events on the host event stream:
 * subscribe/unsubscribe on switch, debounce coalescing, reconnect + reload
 * after an error, give-up, visibility pause/resume, and the watcher turned
 * off when the last view goes away.
 */
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { createBoardLive, nonRetryable, useBoardLive, LIVE_DEBOUNCE_MS, type LiveTarget } from './live';
import { createBoardStore, type ConnState } from './store';
import { setBoardTransport, type BoardAvailability } from './transport';
import { setHostClientFactory } from '../host/factory';
import { fakeBoard, project, task } from './testing';
import type { AppError } from '@unipi/protocol';
import type { Closable, HostClient, HubEvent } from '../host/client';

type FakeHost = {
  client: HostClient;
  /** Pushes a hub event to every open events stream. */
  emit: (event: HubEvent) => void;
  /** Ends every open events stream (the PC dropped off). */
  drop: (message?: string) => void;
  requests: Array<{ op: string; args?: Record<string, unknown> }>;
};

function fakeHost(): FakeHost {
  const requests: Array<{ op: string; args?: Record<string, unknown> }> = [];
  const sinks = new Set<(event: HubEvent) => void>();
  const closers = new Set<(error?: AppError) => void>();
  const client: HostClient = {
    hostId: 'pc',
    conn: () => ({ state: 'connected' }),
    retry: () => Promise.resolve(),
    request: async <T,>(op: string, args?: Record<string, unknown>) => {
      requests.push({ op, args });
      return {} as T;
    },
    openEvents: (sink: (event: HubEvent) => void, closed?: (error?: AppError) => void) => {
      sinks.add(sink);
      const closable: Closable = {
        close: () => {
          sinks.delete(sink);
          closers.delete(closed ?? (() => {}));
        },
      };
      if (closed) closers.add(closed);
      return closable;
    },
    openSession: () => {
      throw new Error('not needed');
    },
    openTerm: () => {
      throw new Error('not needed');
    },
    openLsp: () => {
      throw new Error('not needed');
    },
  };
  return {
    client,
    requests,
    emit: (event) => sinks.forEach((sink) => sink(event)),
    drop: (message) =>
      closers.forEach((closed) => closed(message ? ({ code: 'E_TRANSPORT', message, retryable: true } as AppError) : undefined)),
  };
}

function setup(availability: () => BoardAvailability = () => ({ state: 'ok' })) {
  let tasks = [task({ id: 'A', title: 'First' })];
  const fake = fakeBoard({
    handlers: {
      'project list': () => [project({ slug: 'p1' }), project({ slug: 'p2' })],
      'list --all': ({ project }) => (project === 'p2' ? { tasks: [], problems: [] } : { tasks, problems: [] }),
    },
  });
  setBoardTransport(fake.transport);
  const host = fakeHost();
  setHostClientFactory(() => host.client);
  const store = createRoot(() => createBoardStore());
  const conns: ConnState[] = [];
  const reloads = vi.fn(store.reload.bind(store));
  const loadProjects = vi.fn(store.loadProjects.bind(store));
  const tracked = {
    ...store,
    reload: reloads as unknown as typeof store.reload,
    loadProjects: loadProjects as unknown as typeof store.loadProjects,
    setConn: (state: ConnState) => (conns.push(state), store.setConn(state)),
  };
  void tracked.open('pc', 'p1');
  const check = vi.fn(async () => availability());
  const live = createBoardLive({ store: tracked, checkAvailability: check });
  const loads = () => reloads.mock.calls.length;
  const projectLoads = () => loadProjects.mock.calls.length;
  const watchCalls = () => host.requests.filter((request) => request.op === 'board_watch');
  return { fake, host, store: tracked, live, conns, check, loads, projectLoads, watchCalls };
}

const P1: LiveTarget = { hostId: 'pc', slug: 'p1' };
const P2: LiveTarget = { hostId: 'pc', slug: 'p2' };
const changed = (project: string): HubEvent => ({ kind: 'board_changed', project } as HubEvent);
const flush = async (ms = LIVE_DEBOUNCE_MS) => {
  await vi.advanceTimersByTimeAsync(ms);
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  setBoardTransport(undefined);
  setHostClientFactory(undefined);
});

describe('board live updates', () => {
  it('turns the watcher on, goes live and unsubscribes on project / PC switch and stop', async () => {
    const { host, live, store, watchCalls } = setup();
    live.watch(P1);
    await flush(0);
    expect(watchCalls().at(-1)).toMatchObject({ op: 'board_watch', args: { on: true } });
    expect(store.conn()).toBe('live');

    live.watch(P2);
    await flush(0);
    expect(watchCalls().some((call) => call.args?.on === false)).toBe(true);

    live.stop();
    await flush(0);
    expect(store.conn()).toBe('idle');
    expect(watchCalls().filter((call) => call.args?.on === false)).toHaveLength(2);
  });

  it('coalesces a burst of changes into one reload (+ projects refresh)', async () => {
    const { host, live, loads, projectLoads } = setup();
    live.watch(P1);
    await flush(LIVE_DEBOUNCE_MS * 2); // settle the connect catch-up
    const before = loads();
    const projectsBefore = projectLoads();
    host.emit(changed('p1'));
    host.emit(changed('p1'));
    host.emit(changed('p1'));
    await flush();
    expect(loads()).toBe(before + 1);
    expect(projectLoads()).toBe(projectsBefore + 1);
  });

  it('a change in another project refreshes only the project list', async () => {
    const { host, live, loads, projectLoads } = setup();
    live.watch(P1);
    await flush(LIVE_DEBOUNCE_MS * 2); // settle the connect catch-up
    const before = loads();
    const projectsBefore = projectLoads();
    host.emit(changed('p2'));
    await flush();
    expect(loads()).toBe(before);
    expect(projectLoads()).toBe(projectsBefore + 1);
  });

  it('reconnects with backoff after the stream drops, then reloads (missed changes)', async () => {
    const { host, live, store, loads, check } = setup();
    live.watch(P1);
    await flush(LIVE_DEBOUNCE_MS * 2); // settle the connect catch-up
    const before = loads();
    host.drop('connection lost');
    expect(store.conn()).toBe('reconnecting');
    await vi.advanceTimersByTimeAsync(500); // first backoff step
    expect(store.conn()).toBe('live');
    expect(check).toHaveBeenCalled();
    await flush(); // the catch-up reload's debounce
    // The reconnect catch-up reload covered the missed changes.
    expect(loads()).toBeGreaterThan(before);
  });

  it('keeps retrying while the PC is offline; gives up into idle when the board is not installed', async () => {
    let state: BoardAvailability = { state: 'offline' };
    const { host, live, store } = setup(() => state);
    live.watch(P1);
    await flush(0);
    host.drop('connection lost');
    await vi.advanceTimersByTimeAsync(500);
    await vi.advanceTimersByTimeAsync(1000);
    expect(store.conn()).toBe('reconnecting');
    state = { state: 'not_installed', hint: 'install it' };
    await vi.advanceTimersByTimeAsync(2000);
    await vi.advanceTimersByTimeAsync(5000);
    await vi.advanceTimersByTimeAsync(15000);
    expect(store.conn()).toBe('idle');
  });

  it('a non-retryable stream error goes straight to idle', async () => {
    const { host, live, store } = setup();
    live.watch(P1);
    await flush(0);
    host.drop('unipi-kanboard is not installed');
    expect(store.conn()).toBe('idle');
  });

  it('pauses while the page is hidden and resubscribes + reloads once on return', async () => {
    const { store, loads } = setup();
    const target = () => P1;
    createRoot(() => useBoardLive(store, target));
    await flush(LIVE_DEBOUNCE_MS * 2);
    expect(store.conn()).toBe('live');
    const before = loads();

    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    document.dispatchEvent(new Event('visibilitychange'));
    await flush(0);
    expect(store.conn()).toBe('idle');

    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible');
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(LIVE_DEBOUNCE_MS * 4);
    expect(store.conn()).toBe('live');
    expect(loads()).toBeGreaterThan(before);
  });

  it('nonRetryable keeps its v1 vocabulary', () => {
    expect(nonRetryable('unipi-kanboard is not installed')).toBe(true);
    expect(nonRetryable('unknown pc coffee')).toBe(true);
    expect(nonRetryable(undefined)).toBe(false);
    expect(nonRetryable('connection lost')).toBe(false);
  });
});
