import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost, type MockScenario, type MockSession, type MockWorld } from '../host/mock';
import type { SessionEventDto, SessionSnapshot } from '../host/client';
import { createSession, type SessionHandle } from './store';
import roundtrip from './fixtures/roundtrip.json';
import type { SessionEntry } from './types';

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

const tick = async () => { for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0)); };

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



const disposers: Array<() => void> = [];
afterEach(() => {
  for (const d of disposers.splice(0)) d();
});

describe('w5-ui D3: /goal /swarm /graph /ralph <text> keeps its bubble until the extension\'s message lands', () => {
  it('`handled` drops a plain extension command (/plan on) but keeps a mode command with text', async () => {
    const w = world(sessionSpec({ events: [], cmd: { prompt: () => ({ disposition: 'handled' }) } }));
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    s.send('/plan on');
    await tick();
    expect(s.state.outbox).toEqual([]);

    const item = s.send('/swarm refactor the parser');
    await tick();
    expect(s.state.outbox).toHaveLength(1);
    expect(s.state.outbox[0]).toMatchObject({ ref: item.ref, status: 'acked', text: '/swarm refactor the parser' });

    // The extension's own user message (the text without the command) settles it — seconds later, never before.
    c.advance(6_000);
    expect(s.state.outbox).toHaveLength(1);
    w.push('h1', 's1', [{ session: 's1', seq: 11, event: { type: 'message_start', message: { role: 'user', author: 'extension', timestamp: Date.now(), content: [{ type: 'text', text: 'refactor the parser' }] } } }]);
    c.frame();
    expect(s.state.outbox).toEqual([]);
  });

  it('a bare mode command (no text) and /goal status-style words without args are still dropped on `handled`', async () => {
    const w = world(sessionSpec({ events: [], cmd: { prompt: () => ({ disposition: 'handled' }) } }));
    const c = clock();
    const { s, dispose } = open(w, c);
    disposers.push(() => (s.close(), dispose()));
    await tick();
    s.send('/swarm');
    await tick();
    expect(s.state.outbox).toEqual([]);
  });
});
