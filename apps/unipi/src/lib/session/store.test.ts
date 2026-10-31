import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { unwrap } from 'solid-js/store';
import { createMockHost, type MockScenario, type MockSession, type MockWorld } from '../host/mock';
import type { SessionEventDto, SessionSnapshot } from '../host/client';
import { createSession, createSessionRegistry, type SessionHandle } from './store';
import { applySnapshot, initialState } from './reducer';
import { connBanner } from './selectors';
import roundtrip from './fixtures/roundtrip.json';
import type { SessionEntry, SessionState } from './types';

const rt = roundtrip as unknown as {
  snapshot: SessionSnapshot;
  resume: SessionSnapshot;
  live: SessionEventDto[];
  getEntries: { reply: { entries: SessionEntry[]; leafId: string } };
  fresh: SessionSnapshot;
};

/** Manual timers + frames so reconnect backoff and rAF batching are deterministic. */
function clock() {
  let now = 1_000_000;
  let seq = 0;
  const timers = new Map<number, { at: number; cb: () => void }>();
  const frames: Array<() => void> = [];
  return {
    now: () => now,
    setTimer: (ms: number, cb: () => void) => {
      seq += 1;
      timers.set(seq, { at: now + ms, cb });
      return seq;
    },
    clearTimer: (h: unknown) => void timers.delete(h as number),
    schedule: (flush: () => void) => void frames.push(flush),
    frame() {
      for (const f of frames.splice(0)) f();
    },
    advance(ms: number) {
      now += ms;
      for (const [id, t] of [...timers]) {
        if (t.at <= now) {
          timers.delete(id);
          t.cb();
        }
      }
    },
    pendingTimers: () => timers.size,
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

function sessionSpec(over: Partial<MockSession> = {}): MockSession {
  return {
    snapshot: structuredClone(rt.snapshot),
    events: [...rt.resume.replay!, ...rt.live].map((dto) => ({ at: 0, dto: structuredClone(dto) })),
    entries: { entries: structuredClone(rt.fresh.entries!) as unknown[], leafId: 'a4' },
    ...over,
  };
}

function world(session: MockSession = sessionSpec()): MockWorld {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: session } }] };
  return createMockHost(scenario);
}

function open(w: MockWorld, c: ReturnType<typeof clock>): { s: SessionHandle; dispose: () => void } {
  return createRoot((dispose) => ({ s: createSession('h1', 's1', { client: w.client('h1'), schedule: c.schedule, setTimer: c.setTimer, clearTimer: c.clearTimer, now: c.now }), dispose }));
}

const opens = (w: MockWorld) => w.log.filter((l) => l.kind === 'open').map((l) => (l.args as { sinceSeq?: number }).sinceSeq ?? 0);

function fromFresh(): SessionState {
  const s = initialState('h1', 's1');
  applySnapshot(s, structuredClone(rt.fresh));
  return s;
}
const comparable = (s: SessionState) => {
  const { entriesRev: _r, outbox: _o, ...rest } = structuredClone(unwrap(s));
  return rest;
};

beforeEach(() => localStorage.clear());
let disposers: Array<() => void> = [];
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
});

describe('session store over the mock host', () => {
  it('snapshot → events (one frame) → agent_end reconcile equals a fresh snapshot', async () => {
    const w = world();
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    expect(s.state.conn).toBe('live');
    expect(s.state.seq).toBe(10); // events wait for the frame
    c.frame();
    expect(s.state.seq).toBe(37);
    await tick();
    expect(w.log.some((l) => l.kind === 'cmd' && l.op === 'get_entries' && (l.args as { since?: string }).since === 'a2')).toBe(true);
    expect(comparable(s.state)).toEqual(comparable(fromFresh()));
    const turns = s.turns();
    expect(turns.map((t) => t.turnId)).toEqual(['pre:m1', 'u1', 'u2']);
    expect(turns[2]!.segments[0]!.fold?.detail).toBe('edited 1 file');
  });

  it('a seq gap reopens with sinceSeq and the resume patch fills it', async () => {
    const w = world(sessionSpec({ events: [] }));
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    w.push('h1', 's1', [{ session: 's1', seq: 13, event: { type: 'agent_start' } }]);
    c.frame();
    await tick();
    expect(opens(w)).toEqual([0, 10]);
    expect(s.state.conn).toBe('live');
  });

  it('a dropped stream reconnects with sinceSeq after the backoff; give-up shows the Lost banner; Retry recovers', async () => {
    const w = world(sessionSpec({ events: [] }));
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    w.push('h1', 's1', [{ session: 's1', seq: 11, event: { type: 'agent_start' } }]);
    c.frame();
    w.drop('h1', 's1');
    expect(s.state.conn).toBe('reconnecting');
    expect(connBanner(s.state)?.text).toMatch(/^Lost the connection/);
    c.advance(500);
    await tick();
    expect(opens(w)).toEqual([0, 11]);
    expect(s.state.conn).toBe('live');
    expect(s.state.run.running).toBe(true); // state kept across the resume

    // Host unreachable: retries until the 2 min give-up → `lost`.
    w.markDown('h1/s1', Infinity);
    w.drop('h1', 's1');
    for (let i = 0; i < 40 && s.state.conn !== 'lost'; i++) {
      c.advance(5_000);
      await tick();
    }
    expect(s.state.conn).toBe('lost');
    expect(connBanner(s.state)).toEqual({ text: 'Lost the connection', action: 'retry' });
    w.restore('h1', 's1');
    s.retry();
    await tick();
    expect(s.state.conn).toBe('live');
  });

  it('taken_over stops reconnecting and keeps the banner until Take back', async () => {
    const w = world(sessionSpec({ events: [] }));
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    w.push('h1', 's1', [{ session: 's1', seq: 11, event: { kind: 'taken_over', from: 'tui' } }]);
    c.frame();
    expect(s.state.conn).toBe('closed');
    expect(s.state.banner?.kind).toBe('taken_over');
    c.advance(60_000);
    await tick();
    expect(opens(w)).toEqual([0]);
    s.takeBack();
    await tick();
    expect(opens(w)).toEqual([0, 11]);
    expect(s.state.banner).toBeUndefined();
  });
});

describe('outbox lifecycle (Z-50)', () => {
  it('pending → acked → settled by the host echo; lost on a failed command; Retry resends', async () => {
    let fail = false;
    const w = world(
      sessionSpec({
        events: [],
        cmd: {
          prompt: () => {
            if (fail) throw { code: 'E_TRANSPORT', message: 'The session stream closed', retryable: true };
            return {};
          },
        },
      }),
    );
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    const item = s.send('Now add a regression test');
    expect(s.state.outbox[0]).toMatchObject({ ref: item.ref, status: 'sending', mode: 'prompt' });
    await tick();
    expect(s.state.outbox[0]!.status).toBe('acked');
    expect(w.log.find((l) => l.op === 'prompt')?.args).toEqual({ type: 'prompt', message: 'Now add a regression test' });
    w.push('h1', 's1', [{ session: 's1', seq: 11, event: { type: 'message_start', message: { role: 'user', content: [{ type: 'text', text: 'Now add a regression test' }] } } }]);
    c.frame();
    expect(s.state.outbox).toEqual([]);

    fail = true;
    const lost = s.send('second');
    await tick();
    expect(s.state.outbox[0]).toMatchObject({ ref: lost.ref, status: 'lost', error: 'The session stream closed' });
    expect(JSON.parse(localStorage.getItem('unipi.session.outbox.h1.s1')!)).toHaveLength(1);
    fail = false;
    s.resend(lost.ref);
    await tick();
    expect(s.state.outbox[0]!.status).toBe('acked');
    expect(localStorage.getItem('unipi.session.outbox.h1.s1')).toBeNull();
  });

  it('typed while not live: waiting (persisted), sent in order once live; steer while running', async () => {
    const w = world(sessionSpec({ events: [] }));
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    w.markDown('h1/s1', Infinity);
    w.drop('h1', 's1');
    s.send('one');
    s.send('two');
    expect(s.state.outbox.map((o) => o.status)).toEqual(['waiting', 'waiting']);
    expect(JSON.parse(localStorage.getItem('unipi.session.outbox.h1.s1')!).map((o: { text: string }) => o.text)).toEqual(['one', 'two']);
    w.restore('h1', 's1');
    c.advance(500);
    await tick();
    expect(s.state.conn).toBe('live');
    await tick();
    expect(w.log.filter((l) => l.op === 'prompt').map((l) => (l.args as { message: string }).message)).toEqual(['one', 'two']);
    expect(s.state.outbox.map((o) => o.status)).toEqual(['acked', 'acked']);

    w.push('h1', 's1', [{ session: 's1', seq: 11, event: { type: 'agent_start' } }]);
    c.frame();
    s.send('also this');
    await tick();
    expect(w.log.at(-1)).toMatchObject({ op: 'steer', args: { type: 'steer', message: 'also this' } });
    w.push('h1', 's1', [{ session: 's1', seq: 12, event: { type: 'queue_update', steering: ['also this'], followUp: [] } }]);
    c.frame();
    expect(s.state.outbox.map((o) => o.text)).toEqual(['one', 'two']); // the queue strip shows it now
  });
});

describe('session registry (LRU 8)', () => {
  it('evicts the least recently used unpinned session; acquired ones stay', () => {
    const closed: string[] = [];
    const fake = (hostId: string, sessionId: string) => {
      let refs = 0;
      return {
        key: `${hostId}/${sessionId}`,
        get refs() {
          return refs;
        },
        acquire: () => {
          refs++;
          return () => refs--;
        },
        close: () => closed.push(sessionId),
      } as unknown as SessionHandle;
    };
    const reg = createRoot(() => createSessionRegistry(3, fake));
    const a = reg.ensure('h', 'a');
    const releaseA = a.acquire();
    reg.ensure('h', 'b');
    reg.ensure('h', 'c');
    reg.ensure('h', 'b'); // touch b
    reg.ensure('h', 'd');
    expect(closed).toEqual(['c']); // a is pinned, b was touched
    expect(reg.keys()).toEqual(['h/a', 'h/b', 'h/d']);
    releaseA();
    reg.ensure('h', 'e');
    expect(closed).toEqual(['c', 'a']);
    expect(reg.peek('h', 'a')).toBeUndefined();
  });
});

describe('a rejected open recovers (never a blank chat forever)', () => {
  /** A host whose `session_open` is rejected `failures` times (E_INTERNAL: the file is not written yet). */
  function flakyClient(failures: number) {
    let calls = 0;
    const client = {
      hostId: 'h1',
      openSession(_req: unknown, sink: { snapshot(s: SessionSnapshot): void; closed(e?: unknown): void }) {
        calls += 1;
        const fail = calls <= failures;
        queueMicrotask(() =>
          fail ? sink.closed({ code: 'E_INTERNAL', message: 'no such session', retryable: false }) : sink.snapshot(structuredClone(rt.snapshot)),
        );
        return { cmd: async () => ({}), close() {} };
      },
    };
    return { client: client as never, calls: () => calls };
  }

  it('retries a rejected open on its backoff and goes live', async () => {
    const c = clock();
    const f = flakyClient(2);
    const { s, dispose } = createRoot((d) => ({ s: createSession('h1', 's1', { client: f.client, schedule: c.schedule, setTimer: c.setTimer, clearTimer: c.clearTimer, now: c.now }), dispose: d }));
    await tick();
    expect(s.state.conn).toBe('connecting');
    c.advance(600);
    await tick();
    expect(f.calls()).toBe(2);
    c.advance(1_500);
    await tick();
    expect(f.calls()).toBe(3);
    expect(s.state.conn).toBe('live');
    dispose();
  });

  it('after the retries it shows the failure; Retry (and a revisit) opens again', async () => {
    const c = clock();
    const f = flakyClient(5);
    const reg = createRoot(() => createSessionRegistry(8, (h, id) => createSession(h, id, { client: f.client, schedule: c.schedule, setTimer: c.setTimer, clearTimer: c.clearTimer, now: c.now })));
    const s = reg.ensure('h1', 's1');
    for (const ms of [600, 1_500, 3_000, 6_000]) {
      await tick();
      c.advance(ms);
    }
    await tick();
    expect(f.calls()).toBe(5);
    expect(s.state.conn).toBe('closed');
    expect(s.state.connError?.message).toBe('no such session');
    // Coming back to the chat (the route asks the registry again) retries — and this time it opens.
    reg.ensure('h1', 's1');
    await tick();
    await tick();
    expect(f.calls()).toBe(6);
    expect(s.state.conn).toBe('live');
  });
});
