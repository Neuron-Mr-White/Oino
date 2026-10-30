import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost, type MockScenario } from '../host/mock';
import type { SessionCard } from '../host/client';
import { applyHubEvent, cardsOf, createHub, MAX_NOTICES, setHubUiSink, type HubState } from './hub';

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({ session_id: id, run_state: 'idle', cwd: '/home/u/p', updated_at: 1, ...over });

const tick = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => setHubUiSink(undefined));

describe('hub store', () => {
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
    expect(hub.state.cards.b?.run_state).toBe('working');
    expect(hub.state.cards.c?.name).toBe('New');
    expect(hub.state.cards.a).toBeUndefined();
    expect(hub.state.meta.a).toBeUndefined();
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
    const s: HubState = { hostId: 'h', cards: {}, notices: [], meta: {}, loaded: true, loading: false, live: true };
    for (let i = 0; i < MAX_NOTICES + 3; i++) applyHubEvent(s, { kind: 'host_notice', level: 'info', text: `n${i}` });
    expect(s.notices).toHaveLength(MAX_NOTICES);
    expect(s.notices[0]!.text).toBe('n3');
  });
});
