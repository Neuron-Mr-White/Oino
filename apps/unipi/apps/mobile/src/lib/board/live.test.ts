// @vitest-environment jsdom
/**
 * UNI-132: the live-updates controller against the fake transport —
 * subscribe/unsubscribe on switch, debounce coalescing, reconnect +
 * reload after an error, give-up, visibility pause/resume.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { createBoardLive, nonRetryable, useBoardLive, LIVE_DEBOUNCE_MS, type LiveTarget } from './live';
import { createBoardStore, type ConnState } from './store';
import { setBoardTransport, type BoardAvailability } from './transport';
import { fakeBoard, project, task } from './testing';

function setup(availability: () => BoardAvailability = () => ({ state: 'ok' })) {
  let tasks = [task({ id: 'A', title: 'First' })];
  const fake = fakeBoard({
    routes: {
      'GET /api/projects': () => ({ json: [project({ slug: 'p1' }), project({ slug: 'p2' })] }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: {}, commentRequired: {}, final: [] } }),
      'GET /api/running': () => ({ json: { running: [] } }),
      'GET /api/projects/p1/tasks': () => ({ json: { tasks, problems: [] } }),
      'GET /api/projects/p2/tasks': () => ({ json: { tasks: [], problems: [] } }),
    },
  });
  setBoardTransport(fake.transport);
  const store = createRoot(() => createBoardStore());
  const conns: ConnState[] = [];
  const setConn = store.setConn;
  const tracked = { ...store, setConn: (state: ConnState) => (conns.push(state), setConn(state)) };
  const check = vi.fn(async () => availability());
  const live = createBoardLive({ store: tracked, checkAvailability: check });
  const loads = (slug = 'p1') => fake.requests.filter((r) => r.path === `/api/projects/${slug}/tasks`).length;
  const projectLoads = () => fake.requests.filter((r) => r.path === '/api/projects').length;
  return { fake, store, live, conns, check, loads, projectLoads, setTasks: (next: typeof tasks) => (tasks = next) };
}

const P1: LiveTarget = { hostId: 'pc', slug: 'p1' };
const P2: LiveTarget = { hostId: 'pc', slug: 'p2' };
const flush = async (ms = LIVE_DEBOUNCE_MS) => {
  await vi.advanceTimersByTimeAsync(ms);
};

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  setBoardTransport(undefined);
});

describe('board live updates', () => {
  it('subscribes per project and unsubscribes on project / PC switch and stop', async () => {
    const { fake, live, store } = setup();
    live.watch(P1);
    expect(fake.open().map((s) => `${s.hostId}/${s.project}`)).toEqual(['pc/p1']);
    expect(store.conn()).toBe('connecting');
    live.watch({ ...P1 }); // same target: no resubscribe
    expect(fake.streams).toHaveLength(1);
    live.watch(P2);
    expect(fake.streams[0]!.closed).toBe(true);
    expect(fake.open().map((s) => s.project)).toEqual(['p2']);
    live.watch({ hostId: 'pc2', slug: 'p2' });
    expect(fake.open().map((s) => s.hostId)).toEqual(['pc2']);
    live.stop();
    expect(fake.open()).toHaveLength(0);
    expect(store.conn()).toBe('idle');
  });

  it('coalesces a burst of revisions into one reload (+ projects refresh)', async () => {
    const { fake, live, store, loads, projectLoads, setTasks } = setup();
    await store.open('pc', 'p1');
    const before = loads();
    const projectsBefore = projectLoads();
    live.watch(P1);
    const stream = fake.open()[0]!;
    stream.state('open');
    expect(store.conn()).toBe('live');
    for (let i = 1; i <= 6; i++) {
      stream.revision(String(i));
      await vi.advanceTimersByTimeAsync(40);
    }
    expect(loads()).toBe(before); // still inside the debounce window
    setTasks([task({ id: 'A', title: 'Renamed live' })]);
    await flush();
    expect(loads()).toBe(before + 1);
    expect(projectLoads()).toBe(projectsBefore + 1);
    expect(store.taskById('A')?.title).toBe('Renamed live');
    stream.revision('7');
    await flush();
    expect(loads()).toBe(before + 2);
  });

  it('ignores events of an old subscription after a switch', async () => {
    const { fake, live, loads } = setup();
    live.watch(P1);
    const old = fake.open()[0]!;
    live.watch(P2);
    old.closed = false; // a late message that slipped past the unsubscribe
    old.revision('9');
    await flush();
    expect(loads('p1')).toBe(0);
  });

  it('reconnects with backoff after the stream drops, then reloads (missed revisions)', async () => {
    const { fake, live, store, conns, check, loads } = setup();
    await store.open('pc', 'p1');
    live.watch(P1);
    fake.open()[0]!.state('open');
    await flush();
    const before = loads();
    fake.open()[0]!.state('closed', 'The board daemon closed the event stream');
    expect(store.conn()).toBe('reconnecting');
    expect(fake.open()).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(499);
    expect(fake.streams).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(check).toHaveBeenCalledWith('pc');
    expect(fake.open()).toHaveLength(1);
    expect(store.conn()).toBe('reconnecting');
    // Second failure backs off longer (1 s).
    fake.open()[0]!.state('error', 'PC unreachable');
    await vi.advanceTimersByTimeAsync(999);
    expect(fake.streams).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);
    expect(fake.streams).toHaveLength(3);
    const stream = fake.open()[0]!;
    stream.state('open');
    stream.revision('42'); // the daemon's connect revision
    expect(store.conn()).toBe('live');
    await flush();
    expect(loads()).toBe(before + 1);
    expect(conns).toContain('reconnecting');
  });

  it('keeps retrying while the PC is offline; gives up into idle when the board is not installed', async () => {
    let availability: BoardAvailability = { state: 'offline' };
    const { fake, live, store } = setup(() => availability);
    live.watch(P1);
    fake.open()[0]!.state('error', 'PC unreachable');
    await vi.advanceTimersByTimeAsync(500);
    expect(fake.streams).toHaveLength(1); // offline: no new stream, still backing off
    expect(store.conn()).toBe('reconnecting');
    availability = { state: 'not_installed', hint: 'install unipi-host' };
    await vi.advanceTimersByTimeAsync(1000);
    expect(store.conn()).toBe('idle');
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fake.streams).toHaveLength(1);
  });

  it('a non-retryable stream error goes straight to idle', async () => {
    const { fake, live, store } = setup();
    live.watch(P1);
    fake.open()[0]!.state('error', 'invalid project slug');
    expect(store.conn()).toBe('idle');
    await vi.advanceTimersByTimeAsync(30_000);
    expect(fake.streams).toHaveLength(1);
    expect(nonRetryable('The board daemon closed the event stream')).toBe(false);
  });

  it('pauses while the page is hidden and resubscribes + reloads once on return', async () => {
    const { fake, store, loads } = setup();
    await store.open('pc', 'p1');
    let state: DocumentVisibilityState = 'visible';
    const spy = vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => state);
    const [target] = createSignal<LiveTarget | null>(P1);
    const dispose = createRoot((dispose) => {
      useBoardLive(store, target, { checkAvailability: async () => ({ state: 'ok' }) });
      return dispose;
    });
    expect(fake.open()).toHaveLength(1);
    fake.open()[0]!.state('open');
    fake.open()[0]!.revision('1');
    await flush();
    const before = loads();
    state = 'hidden';
    document.dispatchEvent(new Event('visibilitychange'));
    expect(fake.open()).toHaveLength(0);
    expect(store.conn()).toBe('idle');
    await vi.advanceTimersByTimeAsync(5_000);
    expect(loads()).toBe(before);
    state = 'visible';
    document.dispatchEvent(new Event('visibilitychange'));
    expect(fake.open()).toHaveLength(1);
    const stream = fake.open()[0]!;
    stream.state('open');
    stream.revision('5');
    await flush();
    expect(loads()).toBe(before + 1);
    dispose(); // leaving the tab
    expect(fake.open()).toHaveLength(0);
    spy.mockRestore();
  });
});
