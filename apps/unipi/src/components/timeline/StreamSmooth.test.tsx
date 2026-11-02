// @vitest-environment jsdom
/**
 * ui-stream integration: host events → reducer (`createSession`) → Timeline.
 * The session state always holds the full streamed text at once; only the
 * live prose on screen reveals it over rAF frames, and `message_end` shows
 * the complete answer immediately.
 */
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createMockHost, type MockWorld } from '../../lib/host/mock';
import type { HostClient, SessionEventDto, SessionSnapshot } from '../../lib/host/client';
import { createSession, type SessionHandle } from '../../lib/session/store';
import { sessionRefChat } from '../../lib/timeline/refChat';
import { setStreamSmoothing } from '../../lib/timeline/smoothStream';
import { commandNames, type TimelineCtx } from './context';
import { Timeline } from './Timeline';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => clientOf(),
}));
let clientOf: () => HostClient = () => {
  throw new Error('clientOf not set');
};

let frames: FrameRequestCallback[] = [];
let clock = 1000;
const tick = (n = 1) => {
  for (let i = 0; i < n; i++) {
    clock += 16;
    const run = frames;
    frames = [];
    run.forEach((f) => f(clock));
  }
};

let disposers: Array<() => void> = [];
beforeEach(() => {
  localStorage.clear();
  frames = [];
  clock = 1000;
  vi.stubGlobal('requestAnimationFrame', (f: FrameRequestCallback) => (frames.push(f), frames.length));
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(performance, 'now').mockImplementation(() => clock);
  setStreamSmoothing(true);
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
  setStreamSmoothing(undefined);
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function open(): Promise<{ s: SessionHandle; w: MockWorld }> {
  const snap = { ...structuredClone(rt.fresh), entries: [], seq: 0, leaf_id: null } as unknown as SessionSnapshot;
  const w = createMockHost({ name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: { snapshot: snap, events: [], entries: { entries: [], leafId: null } } } }] });
  clientOf = () => w.client('h1');
  const { s, dispose } = createRoot((d) => ({
    s: createSession('h1', 's1', {
      client: w.client('h1'),
      schedule: (f) => f(),
      setTimer: (ms, cb) => setTimeout(cb, ms) as unknown as number,
      clearTimer: (h) => clearTimeout(h as number),
      now: () => Date.now(),
    }),
    dispose: d,
  }));
  disposers.push(() => {
    s.close();
    dispose();
  });
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return { s, w };
}

const ctx = (s: SessionHandle): TimelineCtx => ({
  session: s,
  hostId: 'h1',
  sessionId: s.sessionId,
  cwd: () => '/proj',
  refs: sessionRefChat(s, () => '/proj'),
  commands: () => commandNames(s.state.commands),
  columnWidth: () => 720,
});

const meta = { timestamp: 1, model: 'm', provider: 'p' };
const delta = (seq: number, text: string): SessionEventDto =>
  ({
    session: 's1',
    seq,
    event: { type: 'message_update', message: { role: 'assistant', ...meta }, assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: text } },
  }) as SessionEventDto;

it('state has the full text at once; the live prose reveals it over frames; message_end shows it all', async () => {
  const { s, w } = await open();
  const r = render(() => <Timeline ctx={ctx(s)} dock={<div />} />);
  const user = { role: 'user', content: [{ type: 'text', text: 'Q' }], ...meta };
  const answer = 'The quick brown fox jumps over the lazy dog. '.repeat(3).trim();
  w.push('h1', 's1', [
    { session: 's1', seq: 1, event: { type: 'agent_start' } },
    { session: 's1', seq: 2, event: { type: 'message_start', message: user } },
    { session: 's1', seq: 3, event: { type: 'message_end', message: user } },
    { session: 's1', seq: 4, event: { type: 'message_start', message: { role: 'assistant', content: [], ...meta } } },
    delta(5, answer),
  ] as SessionEventDto[]);
  await waitFor(() => expect(r.container.querySelector('.tl-prose[data-live]')).toBeTruthy());

  // Reducer: complete text immediately (no protocol / state change).
  expect((s.state.streaming?.message.content as unknown as Array<{ text: string }>)[0]!.text).toBe(answer);

  const prose = () => r.container.querySelector('.tl-tail .tl-prose')!.textContent ?? '';
  expect(prose().length).toBeLessThan(answer.length);
  tick(2);
  const mid = prose();
  expect(mid.length).toBeGreaterThan(0);
  expect(mid.length).toBeLessThan(answer.length);
  expect(answer.startsWith(mid.trim())).toBe(true);

  w.push('h1', 's1', [
    { session: 's1', seq: 6, event: { type: 'message_end', message: { role: 'assistant', content: [{ type: 'text', text: answer }], stopReason: 'stop', ...meta } } },
  ] as SessionEventDto[]);
  await waitFor(() => expect(r.container.textContent).toContain(answer));
});
