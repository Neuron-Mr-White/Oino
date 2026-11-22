import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost } from '../host/mock';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { createSession, type SessionHandle } from './store';
import { connBanner } from './selectors';

const tick = () => new Promise((r) => setTimeout(r, 0));
let cleanup: Array<() => void> = [];
afterEach(() => {
  cleanup.forEach((c) => c());
  cleanup = [];
});

/** Opens `s001` of a WP-A2 scenario with instant frames and manual timers. */
function openScenario(name: string): { s: SessionHandle; advance: (ms: number) => void } {
  const world = createMockHost({ ...SCENARIOS[name]!, speed: 0 });
  const timers: Array<{ at: number; cb: () => void }> = [];
  let now = 0;
  const { s, dispose } = createRoot((dispose) => ({
    s: createSession('pc-coffee', 's001', {
      client: world.client('pc-coffee'),
      schedule: (f) => queueMicrotask(f),
      setTimer: (ms, cb) => timers.push({ at: now + ms, cb }),
      clearTimer: () => {},
      now: () => now,
    }),
    dispose,
  }));
  cleanup.push(() => (s.close(), dispose()));
  return {
    s,
    advance(ms) {
      now += ms;
      for (const t of timers.splice(0)) {
        if (t.at <= now) t.cb();
        else timers.push(t);
      }
    },
  };
}

describe('WP-A2 scenarios through the session store', () => {
  it('stream-basic: the live run settles into a fifth turn with a fold and an answer', async () => {
    const { s } = openScenario('stream-basic');
    await tick();
    await tick();
    const turns = s.turns().filter((t) => t.kind === 'turn');
    expect(turns).toHaveLength(5);
    const last = turns.at(-1)!;
    expect(last.live).toBe(false);
    expect(last.segments[0]!.fold).toMatchObject({ state: 'done', detail: 'ran 1 command' });
    expect(last.segments[0]!.answer?.text).toBe('`add` subtracted. It now returns `a + b`; all 9 tests pass.');
    expect(s.state.run.running).toBe(false);
  });

  it('stream-steer: the steer opens a second segment of the same turn', async () => {
    const { s } = openScenario('stream-steer');
    await tick();
    await tick();
    const last = s.turns().at(-1)!;
    expect(last.segments.map((g) => (g.steer ? 'steer' : 'start'))).toEqual(['start', 'steer']);
    expect(last.segments[1]!.answer?.text).toMatch(/all 9 tests pass/);
    expect(s.state.queue).toEqual({ steering: [], followUp: [] });
  });

  it('stream-reconnect: the drop leaves a running turn, the Lost banner and a waiting outbox', async () => {
    const { s, advance } = openScenario('stream-reconnect');
    await tick();
    await tick();
    expect(s.state.conn).toBe('reconnecting');
    expect(connBanner(s.state)?.action).toBe('retry');
    expect(s.turns().at(-1)!.live).toBe(true);
    s.send('Are you still there?');
    expect(s.state.outbox[0]!.status).toBe('waiting');
    for (let i = 0; i < 30 && s.state.conn !== 'lost'; i++) {
      advance(5_000);
      await tick();
    }
    expect(s.state.conn).toBe('lost');
  });
});
