import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost, type MockScenario } from '../host/mock';
import type { HostClient, SessionCard } from '../host/client';
import { applyHubEvent, cardsOf, createHub, MAX_NOTICES, setHubUiSink, type HubState } from './hub';

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({ session_id: id, run_state: 'idle', cwd: '/home/u/p', updated_at: 1, ...over });

const tick = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => setHubUiSink(undefined));

describe('hub store', () => {
  it('perf-app: hub events of one frame land in one store write', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards: [card('a')] }] };
    const w = createMockHost(scenario);
    const frames: Array<() => void> = [];
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1'), schedule: (f) => frames.push(f) }), dispose }));
    await tick();
    for (let i = 0; i < 200; i++) w.emit({ kind: 'session_changed', card: card(`c${i}`, { run_state: 'working' }) });
    expect(frames).toHaveLength(1);
    expect(hub.state.cards.c0).toBeUndefined();
    frames.shift()!();
    expect(Object.keys(hub.state.cards)).toHaveLength(201);
    w.emit({ kind: 'session_removed', session: 'a' });
    expect(frames).toHaveLength(1);
    frames.shift()!();
    expect(hub.state.cards.a).toBeUndefined();
    dispose();
  });

  it('loads cards from sessions_list and keeps them current from hub events', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards: [card('a', { name: 'Fix', pinned: true }), card('b')] }] };
    const w = createMockHost(scenario);
    const ui: unknown[] = [];
    setHubUiSink((_h, e) => ui.push(e));
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    expect(hub.state.loaded).toBe(true);
    expect(Object.keys(hub.state.cards).sort()).toEqual(['a', 'b']);
    expect(hub.state.meta.a).toEqual({ pinned: true });
    expect(hub.state.live).toBe(true);

    w.emit({ kind: 'session_changed', card: card('b', { run_state: 'working', needs_you: true }) });
    w.emit({ kind: 'session_changed', card: card('c', { name: 'New' }) });
    w.emit({ kind: 'session_removed', session: 'a' });
    w.emit({ kind: 'host_notice', level: 'warning', text: 'Safe mode: extensions are off' });
    w.emit({ kind: 'host_notice', level: 'warning', text: 'Safe mode: extensions are off' });
    w.emit({ kind: 'ui', snapshot: { scope: 'host', rev: 1, entries: [], dialogs: [] } });
    // WP-A15: every git_changed ticks the repo's counter (the Git pane refreshes on it).
    w.emit({ kind: 'git_changed', root: '/home/u/proj' });
    w.emit({ kind: 'git_changed', root: '/home/u/proj' });
    w.emit({ kind: 'git_changed', root: '/other' });
    expect(hub.state.cards.b?.run_state).toBe('working');
    expect(hub.state.cards.c?.name).toBe('New');
    expect(hub.state.cards.a).toBeUndefined();
    expect(hub.state.meta.a).toBeUndefined();
    expect(hub.state.git).toEqual({ '/home/u/proj': 2, '/other': 1 });
    expect(hub.state.notices.map((n) => n.text)).toEqual(['Safe mode: extensions are off']);
    expect(ui).toHaveLength(1);
    hub.dismissNotice(hub.state.notices[0]!.id);
    expect(hub.state.notices).toEqual([]);
    hub.close();
    dispose();
  });

  it('a failed list keeps the error; reload retries', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', conn: { state: 'offline', message: 'No route' } }] };
    const w = createMockHost(scenario);
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    expect(hub.state.error?.code).toBe('E_TRANSPORT');
    expect(hub.state.loaded).toBe(false);
    w.client('h1').setConnection({ state: 'connected' });
    await hub.reload();
    expect(hub.state.loaded).toBe(true);
    expect(hub.state.error).toBeUndefined();
    hub.close();
    dispose();
  });

  it('the events stream dropping retries and resyncs the list', async () => {
    let closed: ((e?: unknown) => void) | undefined;
    let opened = 0;
    let lists = 0;
    const timers: Array<() => void> = [];
    const client = {
      request: async () => {
        lists++;
        return [card('a')];
      },
      openEvents: (_sink: unknown, onClosed: (e?: unknown) => void) => {
        opened++;
        closed = onClosed;
        return { close() {} };
      },
    } as never;
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client, setTimer: (_ms, cb) => timers.push(cb), clearTimer: () => {} }), dispose }));
    await tick();
    closed!({ code: 'E_TRANSPORT', message: 'gone', retryable: true });
    expect(hub.state.live).toBe(false);
    timers.shift()!();
    await tick();
    expect(opened).toBe(2);
    expect(lists).toBe(2);
    expect(hub.state.live).toBe(true);
    hub.close();
    dispose();
  });

  it('setMeta patches optimistically and sends session_meta_set (§3.14)', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards: [card('a'), card('b', { pinned: true })] }] };
    const w = createMockHost(scenario);
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    await hub.setMeta('a', { pinned: true });
    expect(hub.state.meta.a).toEqual({ pinned: true });
    expect(w.log.at(-1)).toMatchObject({ host: 'h1', kind: 'request', op: 'session_meta_set', args: { session: 'a', pinned: true } });
    // The mock's session_changed lands with the flag set.
    await tick();
    expect(hub.state.cards.a?.pinned).toBe(true);
    await hub.setMeta('b', { pinned: false, titleLock: true });
    expect(w.log.at(-1)).toMatchObject({ op: 'session_meta_set', args: { session: 'b', pinned: false, title_locked: true } });
    expect(hub.state.meta.b).toEqual({ titleLock: true });
    hub.close();
    dispose();
  });

  it('focus forwards onto the events connection (§3.14)', async () => {
    const focused: Array<string | null> = [];
    const client = {
      request: async () => [card('a', { unread: true })],
      openEvents: (_sink: unknown, _onClosed?: unknown) => ({ close() {}, focus: (s: string | null) => focused.push(s) }),
    } as never;
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client }), dispose }));
    await tick();
    hub.focus('a');
    hub.focus(null);
    expect(focused).toEqual(['a', null]);
    hub.close();
    dispose();
  });
});

describe('hub helpers', () => {
  it('cardsOf accepts the bare array and the legacy {sessions} shape, dropping junk', () => {
    expect(cardsOf([card('a'), { nope: 1 }]).map((c) => c.session_id)).toEqual(['a']);
    expect(cardsOf({ sessions: [card('b')] }).map((c) => c.session_id)).toEqual(['b']);
    expect(cardsOf(null)).toEqual([]);
  });

  it('notices are capped', () => {
    const s: HubState = { hostId: 'h', cards: {}, archived: [], archivedLoaded: false, archivedLoading: false, notices: [], meta: {}, git: {}, loaded: true, loading: false, live: true };
    for (let i = 0; i < MAX_NOTICES + 3; i++) applyHubEvent(s, { kind: 'host_notice', level: 'info', text: `n${i}` });
    expect(s.notices).toHaveLength(MAX_NOTICES);
    expect(s.notices[0]!.text).toBe('n3');
  });

  it('a restored-then-deleted session also leaves the archived list (Z-13)', () => {
    const s: HubState = { hostId: 'h', cards: {}, archived: [card('z')], archivedLoaded: true, archivedLoading: false, notices: [], meta: {}, git: {}, loaded: true, loading: false, live: true };
    applyHubEvent(s, { kind: 'session_removed', session: 'z' });
    expect(s.archived).toEqual([]);
  });

  it('loadArchived asks sessions_list{archived:true} once, restore puts the card back (Z-13)', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards: [card('live')], archived: [card('gone', { name: 'Gone' })] }] };
    const w = createMockHost(scenario);
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    await hub.loadArchived();
    expect(hub.state.archivedLoaded).toBe(true);
    expect(hub.state.archived.map((c) => c.session_id)).toEqual(['gone']);
    // A second call without force does not re-ask.
    const before = w.log.filter((e) => e.kind === 'request' && e.op === 'sessions_list').length;
    await hub.loadArchived();
    expect(w.log.filter((e) => e.kind === 'request' && e.op === 'sessions_list').length).toBe(before);
    await hub.restore('gone');
    expect(hub.state.archived).toEqual([]);
    expect(hub.state.cards.gone?.name).toBe('Gone');
    hub.close();
    dispose();
  });

  it('emptyTrash answers how many went (Z-13)', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards: [], archived: [card('g1'), card('g2')] }] };
    const w = createMockHost(scenario);
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    const deleted = await hub.emptyTrash();
    expect(deleted).toBe(2);
    expect(hub.state.archived).toEqual([]);
    hub.close();
    dispose();
  });

  it('an archived fetch that raced the delete cannot resurrect the row (owner-fix-5)', async () => {
    // The archive-then-delete flow fires sessions_list{archived:true} (from archive) and
    // session_delete right after; the host answered the list from a snapshot that still
    // held the row. The row must end up gone, not back.
    const pending = new Map<string, (value: unknown) => void>();
    const client = {
      hostId: 'h1',
      conn: () => ({ state: 'connected' as const }),
      retry: async () => {},
      request: (op: string) =>
        new Promise((resolve) => {
          pending.set(op, resolve);
        }),
      openEvents: () => ({ close() {} }),
    };
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: client as unknown as HostClient }), dispose }));
    await tick();
    pending.get("sessions_list")!([]);
    await tick();
    // The Archived list fetch is in flight when the delete goes out.
    void hub.loadArchived();
    const deleting = hub.deleteArchived('gone');
    // The delete finishes first; the raced list answer (pre-delete snapshot) lands after.
    pending.get('session_delete')!(undefined);
    await tick();
    pending.get("sessions_list")!([card("gone")]);
    await deleting;
    await tick();
    expect(hub.state.archived.map((c) => c.session_id)).toEqual([]);
    hub.close();
    dispose();
  });

  it('[ux-sessions] batch archive: one op, rows leave at once, an open Archived list is exact after; failures come back', async () => {
    const cards = Array.from({ length: 450 }, (_, i) => card(`s${i}`));
    cards.push(card('busy', { run_state: 'working' }));
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards, archived: [card('old')] }] };
    const w = createMockHost(scenario);
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    await hub.loadArchived();
    const ids = [...cards.map((c) => c.session_id)];
    const run = hub.archiveMany(ids);
    // Optimistic: gone from the list and in the Archived list before the host answers.
    expect(Object.keys(hub.state.cards)).toHaveLength(0);
    expect(hub.state.archived).toHaveLength(452);
    expect(hub.state.bulk).toMatchObject({ kind: 'archive', total: 451 });
    const result = await run;
    expect(result.done).toHaveLength(450);
    expect(result.failed.map((f) => f.session)).toEqual(['busy']);
    expect(hub.state.cards.busy).toBeTruthy();
    expect(hub.state.bulk).toBeUndefined();
    await tick();
    await tick();
    expect(hub.state.archived.map((c) => c.session_id)).not.toContain('busy');
    expect(hub.state.archived).toHaveLength(451);
    expect(w.log.filter((e) => e.kind === 'request' && e.op === 'sessions_archive')).toHaveLength(1);
    const back = await hub.restoreMany(result.done);
    expect(back.done).toHaveLength(450);
    expect(Object.keys(hub.state.cards)).toHaveLength(451);
    hub.close();
    dispose();
  });

  it('[ux-sessions] chunk events and resync', async () => {
    const s: HubState = { hostId: 'h', cards: { a: card('a'), b: card('b') }, archived: [card('z')], archivedLoaded: true, archivedLoading: false, notices: [], meta: {}, git: {}, loaded: true, loading: false, live: true, bulk: { kind: 'archive', total: 2, done: 0 } };
    applyHubEvent(s, { kind: 'sessions_removed', sessions: ['a', 'b'], archived: true });
    expect(s.cards).toEqual({});
    expect(s.bulk?.done).toBe(2);
    s.bulk = { kind: 'restore', total: 1, done: 0 };
    applyHubEvent(s, { kind: 'sessions_changed', cards: [card('z')] });
    expect(s.cards.z).toBeTruthy();
    expect(s.archived).toEqual([]);
    expect(s.bulk.done).toBe(1);
    // resync re-lists from the host
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', cards: [card('a')] }] };
    const w = createMockHost(scenario);
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: w.client('h1') }), dispose }));
    await tick();
    const lists = () => w.log.filter((e) => e.kind === 'request' && e.op === 'sessions_list').length;
    const before = lists();
    w.emit({ kind: 'resync' });
    await tick();
    expect(lists()).toBe(before + 1);
    hub.close();
    dispose();
  });

  it('archive removes the row optimistically, ignores raced sessions_list snapshots, and treats E_NOT_FOUND as already gone without setting error', async () => {
    const resolvers: Array<{ op: string; resolve: (v: unknown) => void; reject: (e: unknown) => void }> = [];
    const client = {
      hostId: 'h1',
      conn: () => ({ state: 'connected' as const }),
      retry: async () => {},
      request: (op: string) =>
        new Promise((resolve, reject) => {
          resolvers.push({ op, resolve, reject });
        }),
      openEvents: () => ({ close() {} }),
    };
    const { hub, dispose } = createRoot((dispose) => ({ hub: createHub('h1', { client: client as unknown as HostClient }), dispose }));
    await tick();
    // Initial sessions_list returns [a, fileless]
    resolvers.shift()!.resolve([card('a'), card('fileless', { file: '' })]);
    await tick();
    expect(Object.keys(hub.state.cards)).toEqual(['a', 'fileless']);

    // Start a racing reload BEFORE archiving 'a'
    const racingReload = hub.reload();
    const reloadReq = resolvers.shift()!;
    expect(reloadReq.op).toBe('sessions_list');

    const archivingA = hub.archive('a');
    // Optimistic: 'a' is gone immediately before session_archive resolves
    expect(hub.state.cards.a).toBeUndefined();
    const archiveReqA = resolvers.shift()!;
    expect(archiveReqA.op).toBe('session_archive');

    // Raced sessions_list snapshot still contains 'a', then archive resolves
    reloadReq.resolve([card('a'), card('fileless', { file: '' })]);
    await racingReload;
    expect(hub.state.cards.a).toBeUndefined();
    archiveReqA.resolve(card('a'));
    await archivingA;
    expect(hub.state.cards.a).toBeUndefined();
    expect(hub.state.error).toBeUndefined();

    // Archiving 'fileless' when host answers E_NOT_FOUND removes it cleanly with no error
    const archivingFileless = hub.archive('fileless');
    expect(hub.state.cards.fileless).toBeUndefined();
    const archiveReqF = resolvers.shift()!;
    archiveReqF.reject({ code: 'E_NOT_FOUND', message: "No session found matching 'fileless'", retryable: false });
    // Resolve trailing background reload
    await tick();
    if (resolvers[0]?.op === 'sessions_list') resolvers.shift()!.resolve([]);
    await archivingFileless;
    expect(hub.state.cards.fileless).toBeUndefined();
    expect(hub.state.error).toBeUndefined();

    hub.close();
    dispose();
  });
});
