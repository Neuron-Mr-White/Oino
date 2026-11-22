import { describe, expect, it } from 'vitest';
import { SessionStreamClient } from './stream';
import type { HostClient } from '../host/client';
import type { ConnState } from './types';

/** r7 P2-3: a live chat whose host dies keeps "Lost the connection" until a real snapshot lands. */
describe('a live chat whose host is down', () => {
  it('never falls back to `connecting` (which hides the banner) on rejected reopens', async () => {
    let opens = 0;
    const conns: ConnState[] = [];
    const client = {
      hostId: 'local',
      openSession: (_req: unknown, sink: { snapshot: (s: unknown) => void; closed: (e?: unknown) => void }) => {
        opens += 1;
        const n = opens;
        queueMicrotask(() => {
          if (n === 1) sink.snapshot({ seq: 1, entries: [] });
          else sink.closed({ code: 'E_INTERNAL', message: 'host not reachable', retryable: false });
        });
        return { cmd: async () => undefined, close: () => {} };
      },
    } as unknown as HostClient;
    let seq = 0;
    const stream = new SessionStreamClient(
      client,
      's1',
      { seq: () => seq, snapshot: () => ((seq = 1), {}), events: () => ({}), conn: (c) => void conns.push(c) },
      { setTimer: (_ms, cb) => setTimeout(cb, 1), clearTimer: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) },
    );
    stream.open();
    await new Promise((r) => setTimeout(r, 5));
    // Drop the live link, as a killed host does.
    (stream as unknown as { stream: { close(): void } | undefined }).stream = undefined;
    stream.reopen(true);
    await new Promise((r) => setTimeout(r, 60));
    const afterLive = conns.slice(conns.indexOf('live') + 1);
    expect(afterLive).toContain('reconnecting');
    expect(afterLive).not.toContain('connecting');
    expect(afterLive).not.toContain('closed');
    expect(opens).toBeGreaterThan(3);
    stream.close();
  });
});
