/**
 * chat-vanish (owner 2026-11-03: "occasionally the chat fully disappears; only a full app
 * restart brings it back"): the store-level paths that left a shown chat with no content
 * and nothing that would ever bring it back.
 */
import { describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import type { SessionSnapshot } from '../host/client';
import { createSession, createSessionRegistry, openFailed, type SessionHandle } from './store';
import { createHub, onHostBack } from './hub';
import roundtrip from './fixtures/roundtrip.json';

const rt = roundtrip as unknown as { snapshot: SessionSnapshot };
const tick = () => new Promise((r) => setTimeout(r, 0));

function clock() {
  let now = 1_000_000;
  let seq = 0;
  const timers = new Map<number, { at: number; cb: () => void }>();
  return {
    now: () => now,
    setTimer: (ms: number, cb: () => void) => {
      seq += 1;
      timers.set(seq, { at: now + ms, cb });
      return seq;
    },
    clearTimer: (h: unknown) => void timers.delete(h as number),
    schedule: (flush: () => void) => void queueMicrotask(flush),
    advance(ms: number) {
      now += ms;
      for (const [id, t] of [...timers]) {
        if (t.at <= now) {
          timers.delete(id);
          t.cb();
        }
      }
    },
  };
}

/** A fake handle with a real refs/closed contract (the registry's view of a session). */
function fakeFactory(closedLog: string[]) {
  return (hostId: string, sessionId: string) => {
    let refs = 0;
    let closed = false;
    return {
      key: `${hostId}/${sessionId}`,
      hostId,
      get refs() {
        return refs;
      },
      get closed() {
        return closed;
      },
      acquire: () => {
        refs++;
        return () => refs--;
      },
      close: () => {
        closed = true;
        closedLog.push(sessionId);
      },
    } as unknown as SessionHandle;
  };
}

describe('session registry never hands out a dead handle', () => {
  it('with `limit` chats pinned, the chat being opened is NOT closed by its own ensure (root cause #1)', () => {
    const closed: string[] = [];
    const reg = createRoot(() => createSessionRegistry(3, fakeFactory(closed)));
    // The shown chat, the status pane's stale pin and the model dialog's: three pins.
    for (const id of ['a', 'b', 'c']) reg.ensure('h', id).acquire();
    const d = reg.ensure('h', 'd');
    // Before the fix the eviction loop skipped a/b/c (pinned) and closed `d` itself:
    // the Conversation then pinned a handle whose stream was already gone → blank chat,
    // and every further chat opened the same way until the app restarted.
    expect(closed).toEqual([]);
    expect(d.closed).toBe(false);
    expect(reg.keys()).toEqual(['h/a', 'h/b', 'h/c', 'h/d']);
    // Bounded: once a pin goes, the next ensure trims back to the limit.
    const releaseD = d.acquire();
    releaseD();
    reg.ensure('h', 'e');
    expect(closed).toEqual(['d']);
    expect(reg.keys()).toEqual(['h/a', 'h/b', 'h/c', 'h/e']);
  });

  it('a handle closed behind the registry is replaced on the next ensure', () => {
    const closed: string[] = [];
    const reg = createRoot(() => createSessionRegistry(3, fakeFactory(closed)));
    const a = reg.ensure('h', 'a');
    a.close();
    const again = reg.ensure('h', 'a');
    expect(again).not.toBe(a);
    expect(again.closed).toBe(false);
  });
});

/** A host whose session opens fail `failures` times with a dropped link, then succeed. */
function droppingClient(failures: () => boolean) {
  let calls = 0;
  const client = {
    hostId: 'h1',
    openSession(_req: unknown, sink: { snapshot(s: SessionSnapshot): void; closed(e?: unknown): void }) {
      calls += 1;
      const fail = failures();
      queueMicrotask(() => (fail ? sink.closed({ code: 'E_TRANSPORT', message: 'connection lost', retryable: true }) : sink.snapshot(structuredClone(rt.snapshot))));
      return { cmd: async () => ({}), close() {} };
    },
  };
  return { client: client as never, calls: () => calls };
}

describe('a chat that gave up during a host outage comes back with the host (root cause #2)', () => {
  it('lost after 2 min of failed reconnects; retryFailed(host) reopens it live', async () => {
    const c = clock();
    let down = false;
    const f = droppingClient(() => down);
    const reg = createRoot(() =>
      createSessionRegistry(3, (h, id) => createSession(h, id, { client: f.client, schedule: c.schedule, setTimer: c.setTimer, clearTimer: c.clearTimer, now: c.now })),
    );
    const s = reg.ensure('h1', 's1');
    await tick();
    expect(s.state.conn).toBe('live');
    // The owner's log 2026-11-03 02:10–02:13Z: the host wedges, the watchdog kills it twice.
    down = true;
    const stream = (s as unknown as { retry(): void });
    stream.retry();
    await tick();
    for (let i = 0; i < 40 && s.state.conn !== 'lost'; i++) {
      c.advance(5_000);
      await tick();
    }
    expect(s.state.conn).toBe('lost');
    expect(openFailed(s.state)).toBe(true);
    // The host is back (02:13:50Z) — before the fix nothing reopened the chat: zero
    // `session_open` in the host log until the owner restarted the app at 02:21:58Z.
    down = false;
    const before = f.calls();
    c.advance(60_000);
    await tick();
    expect(f.calls()).toBe(before);
    expect(reg.retryFailed('h1')).toBe(1);
    await tick();
    await tick();
    expect(s.state.conn).toBe('live');
    expect(reg.retryFailed('h1')).toBe(0);
  });

  it('the hub announces the host is back after its events stream dropped and a list reload succeeded', async () => {
    let closeEvents: ((error?: unknown) => void) | undefined;
    let listOk = true;
    const client = {
      hostId: 'h1',
      request: async () => {
        if (!listOk) throw { code: 'E_TRANSPORT', message: 'down', retryable: true };
        return { sessions: [] };
      },
      openEvents(_sink: unknown, closed: (error?: unknown) => void) {
        closeEvents = closed;
        return { close() {} };
      },
    };
    const c = clock();
    const back: string[] = [];
    const off = onHostBack((id) => back.push(id));
    const hub = createRoot(() => createHub('h1', { client: client as never, setTimer: c.setTimer, clearTimer: c.clearTimer, schedule: null } as never));
    await hub.reload();
    expect(back).toEqual([]); // a first load is not a "back"
    listOk = false;
    closeEvents?.({ code: 'E_TRANSPORT', message: 'gone', retryable: true });
    c.advance(30_000);
    await tick();
    expect(back).toEqual([]);
    listOk = true;
    closeEvents?.({ code: 'E_TRANSPORT', message: 'gone', retryable: true });
    c.advance(30_000);
    await tick();
    await tick();
    expect(back).toEqual(['h1']);
    off();
    hub.close?.();
  });
});
