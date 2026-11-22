import { afterEach, describe, expect, it } from 'vitest';
import { applyEvents, applySnapshot, initialState } from './reducer';
import { pushCoalescing, type QueuedEvent } from './coalesce';
import { SessionStreamClient } from './stream';
import type { HostClient, SessionEventDto, SessionSink, SessionSnapshot } from '../host/client';
import type { SessionState } from './types';
import { createUiRegistry } from '../../ui/registry';

/**
 * w3-hidden: a window that was hidden while sessions streamed must show exactly what a window that
 * never was hidden shows. Two mechanisms: folding queued events (`coalesce.ts`) and pausing the
 * stream and resuming from `sinceSeq` (`stream.ts`).
 */

const NOW = 1_700_000_000_000;
const S = 's1';

type Ev = Record<string, unknown>;
const assistant = (content: unknown[]) => ({ role: 'assistant', content, timestamp: NOW });

/** A turn with thinking, two text blocks, a tool call (args streamed), a tool run with partial output, then the end. */
function script(mode: 'full' | 'delta'): Ev[] {
  const out: Ev[] = [{ type: 'agent_start' }];
  const content: Array<Record<string, unknown>> = [];
  out.push({ type: 'message_start', message: assistant([]) });
  const update = (assistantMessageEvent: Ev) => {
    out.push(
      mode === 'full'
        ? { type: 'message_update', message: assistant(structuredClone(content)), assistantMessageEvent }
        : { type: 'message_update', assistantMessageEvent },
    );
  };
  // thinking (block 0)
  content.push({ type: 'thinking', thinking: '' });
  update({ type: 'thinking_start', contentIndex: 0 });
  for (const w of ['Let ', 'me ', 'think ', 'about ', 'this.']) {
    (content[0] as { thinking: string }).thinking += w;
    update({ type: 'thinking_delta', contentIndex: 0, delta: w });
  }
  // text (block 1)
  content.push({ type: 'text', text: '' });
  update({ type: 'text_start', contentIndex: 1 });
  for (const w of ['alpha ', 'beta ', 'gamma ', '\n\n```ts\n', 'const x = 1;\n', '```\n']) {
    (content[1] as { text: string }).text += w;
    update({ type: 'text_delta', contentIndex: 1, delta: w });
  }
  // tool call (block 2)
  const call = { type: 'toolCall', id: 'call-1', name: 'bash', arguments: {} as Record<string, unknown> };
  content.push({ ...call });
  update({ type: 'toolcall_start', contentIndex: 2, toolCall: { id: 'call-1', name: 'bash' } });
  let partialJson = '';
  for (const piece of ['{"command":', '"echo ', 'hi"}']) {
    partialJson += piece;
    Object.assign(content[2]!, { partialJson });
    try {
      content[2]!.arguments = JSON.parse(partialJson);
    } catch {
      /* partial */
    }
    update({ type: 'toolcall_delta', contentIndex: 2, delta: piece, toolCall: { id: 'call-1', name: 'bash' } });
  }
  const finished = { ...call, arguments: { command: 'echo hi' } };
  content[2] = finished;
  update({ type: 'toolcall_end', contentIndex: 2, toolCall: finished });
  out.push({ type: 'message_end', message: assistant(structuredClone(content)) });
  // the tool runs: three partial outputs, the last one with details
  out.push({ type: 'tool_execution_start', toolCallId: 'call-1', toolName: 'bash', args: { command: 'echo hi' } });
  let text = '';
  for (let i = 0; i < 4; i++) {
    text += `line ${i}\n`;
    out.push({
      type: 'tool_execution_update',
      toolCallId: 'call-1',
      toolName: 'bash',
      args: { command: 'echo hi' },
      partialResult: { content: [{ type: 'text', text }], ...(i === 3 ? { details: { exit: null } } : {}) },
    });
  }
  out.push({ type: 'tool_execution_end', toolCallId: 'call-1', toolName: 'bash', result: { content: [{ type: 'text', text }] }, isError: false, durationMs: 12 });
  out.push({ type: 'message_start', message: { role: 'toolResult', toolCallId: 'call-1', content: [{ type: 'text', text }], timestamp: NOW } });
  out.push({ type: 'message_end', message: { role: 'toolResult', toolCallId: 'call-1', toolName: 'bash', content: [{ type: 'text', text }], timestamp: NOW } });
  // a second assistant message, then the turn ends
  const second = [{ type: 'text', text: '' }];
  out.push({ type: 'message_start', message: assistant([]) });
  for (const w of ['all ', 'done.']) {
    (second[0] as { text: string }).text += w;
    out.push(
      mode === 'full'
        ? { type: 'message_update', message: assistant(structuredClone(second)), assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: w } }
        : { type: 'message_update', assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: w } },
    );
  }
  out.push({ type: 'message_end', message: assistant(structuredClone(second)) });
  out.push({ type: 'agent_end', messages: [] });
  return out;
}

const dtos = (events: Ev[], from = 1): SessionEventDto[] => events.map((event, i) => ({ session: S, seq: from + i, event }) as SessionEventDto);
/** What every window starts from: the first open's (empty) snapshot. */
const fresh = (): SessionState => {
  const s = { ...initialState('local', S), conn: 'live' as const };
  applySnapshot(s, { session_id: S, seq: 0, reset: true, entries: [], state: {} } as unknown as SessionSnapshot, NOW);
  return s;
};
const plain = (s: SessionState) => JSON.parse(JSON.stringify(s));

/** The control: every event applied in order, as a never-hidden window does. */
function control(items: SessionEventDto[]) {
  const s = fresh();
  const r = applyEvents(s, items, NOW);
  return { state: plain(s), result: r };
}

/** A hidden window: events fold into the queue, then ONE flush applies it. */
function folded(items: SessionEventDto[], flushEvery = Infinity) {
  const s = fresh();
  let queue: QueuedEvent[] = [];
  const result: Record<string, unknown> = {};
  const flush = () => {
    Object.assign(result, applyEvents(s, queue, NOW));
    queue = [];
  };
  items.forEach((item, i) => {
    pushCoalescing(queue, item);
    if ((i + 1) % flushEvery === 0) flush();
  });
  flush();
  return { state: plain(s), result, queued: queue.length };
}

describe('folding queued events while hidden', () => {
  for (const mode of ['full', 'delta'] as const) {
    it(`reaches the never-hidden state (${mode} message updates, thinking, parts, tool call, turn end)`, () => {
      const items = dtos(script(mode));
      const want = control(items);
      const got = folded(items);
      expect(got.state).toEqual(want.state);
      expect(got.result).toEqual(want.result);
      // The turn really ended inside the folded batch.
      expect(want.state.run.running).toBe(false);
      expect(want.state.entries.length).toBeGreaterThan(2);
    });

    it(`holds for any flush cadence (${mode})`, () => {
      const items = dtos(script(mode));
      const want = control(items).state;
      for (const every of [1, 2, 3, 5, 7, 11, 13]) expect(folded(items, every).state).toEqual(want);
    });
  }

  it('really folds: a long run of deltas stays a handful of queue entries', () => {
    const items: SessionEventDto[] = [];
    let seq = 1;
    items.push(...dtos([{ type: 'agent_start' }, { type: 'message_start', message: assistant([]) }], seq));
    seq += 2;
    let text = '';
    for (let i = 0; i < 5_000; i++) {
      text += 'w ';
      items.push(...dtos([{ type: 'message_update', message: assistant([{ type: 'text', text }]), assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: 'w ' } }], seq++));
    }
    const got = folded(items);
    expect(got.state).toEqual(control(items).state);
    const queue: QueuedEvent[] = [];
    for (const item of items) pushCoalescing(queue, item);
    expect(queue.length).toBe(3);
    expect(queue[2]!.seq).toBe(5_002);
    expect(queue[2]!.fromSeq).toBe(3);
  });

  it('delta-only text accumulates into one delta per block', () => {
    const items = dtos([
      { type: 'message_start', message: assistant([]) },
      ...Array.from({ length: 200 }, (_, i) => ({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: `${i},` } })),
    ]);
    const queue: QueuedEvent[] = [];
    for (const item of items) pushCoalescing(queue, item);
    expect(queue.length).toBe(2);
    expect(((queue[1]!.event as Ev).assistantMessageEvent as Ev).delta).toBe(Array.from({ length: 200 }, (_, i) => `${i},`).join(''));
  });

  it('never folds across different blocks, tools or event kinds', () => {
    const q: QueuedEvent[] = [];
    const add = (event: Ev) => pushCoalescing(q, { session: S, seq: q.length + 1, event } as QueuedEvent);
    add({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: 'a' } });
    add({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', contentIndex: 1, delta: 'b' } });
    add({ type: 'message_update', assistantMessageEvent: { type: 'thinking_delta', contentIndex: 1, delta: 'c' } });
    add({ type: 'tool_execution_update', toolCallId: 't1', toolName: 'bash', partialResult: { content: [{ type: 'text', text: 'x' }] } });
    add({ type: 'tool_execution_update', toolCallId: 't2', toolName: 'bash', partialResult: { content: [{ type: 'text', text: 'y' }] } });
    add({ type: 'queue_update', steering: [], followUp: [] });
    add({ type: 'queue_update', steering: ['x'], followUp: [] });
    expect(q.length).toBe(7);
  });

  it('keeps an earlier tool `details` that a later update lacks (no fold)', () => {
    const items = dtos([
      { type: 'tool_execution_update', toolCallId: 't', toolName: 'bash', args: {}, partialResult: { content: [{ type: 'text', text: 'a' }], details: { k: 1 } } },
      { type: 'tool_execution_update', toolCallId: 't', toolName: 'bash', args: {}, partialResult: { content: [{ type: 'text', text: 'ab' }] } },
    ]);
    expect(folded(items).state).toEqual(control(items).state);
  });
});

// ---- pause / resume through the stream client -------------------------------------------------

const setHidden = (hidden: boolean) => {
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (hidden ? 'hidden' : 'visible') });
  document.dispatchEvent(new Event('visibilitychange'));
};
afterEach(() => {
  delete (document as unknown as { hidden?: boolean }).hidden;
  delete (document as unknown as { visibilityState?: string }).visibilityState;
});

/** A fake host: numbers events, keeps a ring, answers `sinceSeq` with a resume patch (or a snapshot when the ring rolled). */
function fakeHost(ringSize = Infinity) {
  const ring: SessionEventDto[] = [];
  const history: SessionEventDto[] = [];
  const opens: Array<{ sinceSeq?: number }> = [];
  let sink: SessionSink | undefined;
  let open = 0;
  let closes = 0;
  let sent = 0;
  const client = {
    hostId: 'local',
    openSession: (req: { sinceSeq?: number }, s: SessionSink) => {
      opens.push({ sinceSeq: req.sinceSeq });
      sink = s;
      open++;
      const seq = ring.at(-1)?.seq ?? 0;
      const since = req.sinceSeq ?? 0;
      queueMicrotask(() => {
        if (since > 0 && ring.length && ring[0]!.seq <= since + 1) {
          s.snapshot({ session_id: S, seq, reset: false, replay: ring.filter((e) => e.seq > since) } as unknown as SessionSnapshot);
        } else {
          // Snapshot: the settled state the ring's events lead to (what a fresh open would read).
          const settled = fresh();
          applyEvents(settled, history, NOW);
          const real = (id: string | null | undefined) => (id?.startsWith('live:') ? `e-${id.slice(5)}` : id);
          const entries = settled.entries.map((e) => ({ ...e, id: real(e.id)!, parentId: real(e.parentId) }));
          s.snapshot({ session_id: S, seq, reset: true, entries, state: {} } as unknown as SessionSnapshot);
        }
      });
      return {
        cmd: async () => {
          sent++;
          return 'ok';
        },
        close: () => {
          closes++;
          open--;
          if (sink === s) sink = undefined;
        },
      };
    },
  } as unknown as HostClient;
  return {
    client,
    opens,
    get liveStreams() {
      return open;
    },
    get closes() {
      return closes;
    },
    get sent() {
      return sent;
    },
    /** The host produces events; a connected stream receives them, a paused one does not. */
    emit(events: Ev[]) {
      const items = dtos(events, (ring.at(-1)?.seq ?? 0) + 1);
      ring.push(...items);
      history.push(...items);
      while (ring.length > ringSize) ring.shift();
      sink?.events(items);
    },
  };
}

/** A client wired to the reducer, like the store does (one `schedule` flush per queued batch). */
function wire(host: ReturnType<typeof fakeHost>, mayPause?: () => boolean) {
  // Unopened: the stream's first snapshot is what `fresh()` stands for in the control.
  const state: SessionState = { ...initialState('local', S), conn: 'live' };
  const frames: Array<() => void> = [];
  const stream = new SessionStreamClient(
    host.client,
    S,
    {
      seq: () => state.seq,
      snapshot: (snap) => applySnapshot(state, snap, NOW),
      events: (items) => applyEvents(state, items, NOW),
      conn: () => {},
      mayPause,
    },
    { schedule: (flush) => void frames.push(flush) },
  );
  return {
    state,
    stream,
    /** Runs the pending animation frames (a hidden page never does). */
    frame() {
      for (const f of frames.splice(0)) f();
    },
    settle: () => new Promise<void>((r) => setTimeout(r, 0)),
  };
}

describe('pausing the stream while the window is hidden', () => {
  it('hangs up when hidden, delivers nothing, and resumes from the last applied seq into the never-hidden state', async () => {
    const all = script('full');
    const host = fakeHost();
    const w = wire(host);
    w.stream.open();
    await w.settle();
    host.emit(all.slice(0, 9)); // visible: part of the turn
    w.frame();
    const appliedBefore = w.state.seq;
    expect(appliedBefore).toBe(9);

    setHidden(true);
    expect(host.liveStreams).toBe(0);
    expect(w.stream.live).toBe(false);
    host.emit(all.slice(9)); // the rest of the turn, tool call and turn end included, while hidden
    w.frame();
    expect(w.state.seq).toBe(appliedBefore); // nothing was delivered or applied

    setHidden(false);
    await w.settle();
    expect(host.opens.at(-1)).toEqual({ sinceSeq: appliedBefore });
    expect(host.liveStreams).toBe(1);
    expect(w.stream.live).toBe(true);
    expect(w.state.run.running).toBe(false);
    expect(w.state.seq).toBe(all.length);
    expect(plain(w.state)).toEqual(control(dtos(all)).state);
    w.stream.close();
  });

  it('falls back to a snapshot when the host ring rolled over while hidden', async () => {
    const all = script('full');
    const host = fakeHost(8); // far fewer events than the hidden stretch
    const w = wire(host);
    w.stream.open();
    await w.settle();
    host.emit(all.slice(0, 4));
    w.frame();
    setHidden(true);
    host.emit(all.slice(4));
    setHidden(false);
    await w.settle();
    expect(host.opens.at(-1)).toEqual({ sinceSeq: 4 });
    expect(w.state.seq).toBe(all.length);
    expect(w.state.run.running).toBe(false);
    expect(w.state.entries.length).toBeGreaterThan(2);
    // The same conversation the never-hidden window shows (ids are the host's, not provisional).
    const shown = (st: SessionState) => st.entries.map((e) => JSON.stringify(e.message));
    expect(shown(w.state)).toEqual(shown(control(dtos(all)).state as SessionState));
    w.stream.close();
  });

  it('applies every resumed session in one batch of frames and not one frame per delta', async () => {
    const hosts = [fakeHost(), fakeHost(), fakeHost()];
    const wires = hosts.map((h) => wire(h));
    for (const w of wires) w.stream.open();
    await wires[0]!.settle();
    setHidden(true);
    hosts.forEach((h, i) => h.emit(script('full').map((e) => ({ ...e, i }))));
    for (const w of wires) w.frame();
    expect(wires.every((w) => w.state.seq === 0)).toBe(true); // nothing delivered while hidden
    setHidden(false);
    await wires[0]!.settle();
    for (const w of wires) expect(w.state.seq).toBe(script('full').length);
    wires.forEach((w) => w.stream.close());
  });

  it('a command issued while paused waits for the reopened stream instead of failing', async () => {
    const host = fakeHost();
    const w = wire(host);
    w.stream.open();
    await w.settle();
    setHidden(true);
    const reply = w.stream.cmd({ type: 'get_state' } as never);
    let settled = false;
    void reply.then(() => (settled = true));
    await w.settle();
    expect(settled).toBe(false);
    expect(host.sent).toBe(0);
    setHidden(false);
    await expect(reply).resolves.toBe('ok');
    expect(host.sent).toBe(1);
    w.stream.close();
  });

  it('does not pause with a command in flight; its queue folds instead of growing', async () => {
    const host = fakeHost();
    const w = wire(host);
    // A command that never answers.
    const pending = (host.client as unknown as { openSession: (...a: unknown[]) => unknown }).openSession;
    (host.client as unknown as { openSession: (...a: unknown[]) => unknown }).openSession = (...a: unknown[]) => {
      const stream = pending(...a) as { cmd: () => Promise<unknown> };
      stream.cmd = () => new Promise(() => {});
      return stream;
    };
    w.stream.open();
    await w.settle();
    void w.stream.cmd({ type: 'abort' } as never).catch(() => {});
    setHidden(true);
    expect(host.liveStreams).toBe(1);
    const all = script('full');
    host.emit(all.slice(0, 3));
    const queued = () => (w.stream as unknown as { queue: unknown[] }).queue.length;
    let text = '';
    const ring: Ev[] = [];
    for (let i = 0; i < 2_000; i++) {
      text += 'x ';
      ring.push({ type: 'message_update', message: assistant([{ type: 'text', text }]), assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: 'x ' } });
    }
    host.emit(ring);
    expect(queued()).toBeLessThanOrEqual(5);
    setHidden(false);
    w.frame();
    expect(w.state.seq).toBe(3 + 2_000);
    w.stream.close();
  });

  it('stays open while an approval or dialog waits for an answer', async () => {
    const host = fakeHost();
    let waiting = true;
    const w = wire(host, () => !waiting);
    w.stream.open();
    await w.settle();
    setHidden(true);
    expect(host.liveStreams).toBe(1);
    expect(w.stream.live).toBe(true);
    setHidden(false);
    waiting = false;
    setHidden(true);
    expect(host.liveStreams).toBe(0);
    setHidden(false);
    await w.settle();
    w.stream.close();
  });

  it('opened while hidden: pauses right after its first frame and resumes on show', async () => {
    const host = fakeHost();
    host.emit(script('full').slice(0, 2));
    setHidden(true);
    const w = wire(host);
    w.stream.open();
    await w.settle();
    expect(host.liveStreams).toBe(0);
    setHidden(false);
    await w.settle();
    expect(host.liveStreams).toBe(1);
    w.stream.close();
  });
});


describe('generation-owned scheduling', () => {
  it('QA F1: old callback after hide/show cannot erase event 3 (seqs [3,4])', () => {
    setHidden(false);
    const sinks: SessionSink[] = [];
    const callbacks: Array<() => void> = [];
    const applied: number[] = [];
    const client = { hostId: 'local', openSession: (_: unknown, sink: SessionSink) => {
      sinks.push(sink);
      return { close() {}, cmd: async () => ({}) };
    } } as unknown as HostClient;
    const stream = new SessionStreamClient(client, S, {
      seq: () => 1, snapshot: () => ({}), conn() {},
      events: (items) => { applied.push(...items.map((item) => item.seq)); return {}; },
    }, { schedule: (cb) => callbacks.push(cb) });
    const snap = { session_id: S, seq: 1, reset: true } as SessionSnapshot;
    stream.open();
    sinks[0]!.snapshot(snap);
    sinks[0]!.events(dtos([{ type: 'turn_start' }], 2));
    setHidden(true);
    setHidden(false);
    sinks[1]!.snapshot(snap);
    sinks[1]!.events(dtos([{ type: 'turn_start' }], 3));
    callbacks.shift()!(); // obsolete generation, after the new event was queued
    expect(applied).toEqual([]);
    sinks[1]!.events(dtos([{ type: 'turn_start' }], 4));
    for (const cb of callbacks.splice(0)) cb();
    expect(applied).toEqual([3, 4]);
    stream.close();
  });

  it('random hide/show, reconnect, takeover and callback interleavings apply every numbered event once in order', () => {
    // Seeded scheduler: replay uses the last APPLIED seq, including events detached before flush.
    for (let seed = 1; seed <= 50; seed++) {
      setHidden(false);
      let rng = seed;
      const random = (n: number) => { rng = (Math.imul(rng, 1664525) + 1013904223) >>> 0; return rng % n; };
      const callbacks: Array<() => void> = [];
      const history: SessionEventDto[] = [];
      const applied: number[] = [];
      let seq = 0;
      let sink: SessionSink;
      let active = false;
      let takeover = false;
      let hidden = false;
      const client = { hostId: 'local', openSession: (_: unknown, next: SessionSink) => {
        sink = next; active = true;
        return { close() { active = false; }, cmd: async () => ({}) };
      } } as unknown as HostClient;
      const consume = (items: SessionEventDto[]) => {
        for (const item of items) if (item.seq > seq) { expect(item.seq).toBe(seq + 1); seq = item.seq; applied.push(seq); }
        return {};
      };
      const stream = new SessionStreamClient(client, S, {
        seq: () => seq, conn() {},
        snapshot: () => consume(history.filter((item) => item.seq > seq)),
        events: (items) => takeover ? { takenOver: true } : consume(items),
      }, { schedule: (cb) => callbacks.push(cb) });
      const snapshot = () => sink!.snapshot({ session_id: S, seq: history.length, reset: false, replay: [] });
      stream.open(); snapshot();
      for (let step = 0; step < 200; step++) {
        switch (random(6)) {
          case 0: {
            const item = dtos([{ type: 'turn_start' }], history.length + 1)[0]!;
            history.push(item);
            if (active) sink!.events([item]);
            break;
          }
          case 1: hidden = true; setHidden(true); break;
          case 2:
            if (hidden) { hidden = false; setHidden(false); if (active) snapshot(); }
            break;
          case 3: stream.reopen(true); snapshot(); break;
          case 4: if (callbacks.length) callbacks.splice(random(callbacks.length), 1)[0]!(); break;
          case 5:
            if (active) {
              takeover = true;
              sink!.events(dtos([{ kind: 'taken_over' }], history.length + 1));
              stream.flush();
              takeover = false;
              stream.retryNow(); snapshot(); // explicit Take back
            }
        }
      }
      setHidden(false); stream.retryNow(); snapshot();
      for (const cb of callbacks) cb();
      expect(applied).toEqual(history.map((item) => item.seq));
      stream.close();
    }
  });
});


it('hidden request + ring overrun restores an answerable card through stream, reducer and UI registry', async () => {
  setHidden(false);
  const registry = createUiRegistry('session');
  const state = fresh();
  const frames: Array<() => void> = [];
  let sink: SessionSink;
  let active = false;
  let seq = 1;
  let pending: Record<string, unknown>[] = [];
  let resolveWait: (value: unknown) => void;
  const hostWait = new Promise((resolve) => { resolveWait = resolve; });
  const route = (result: ReturnType<typeof applySnapshot>) => {
    for (const event of result.ui ?? []) registry.applyRequest(event);
    return result;
  };
  const client = { hostId: 'local', openSession: (_: unknown, next: SessionSink) => {
    sink = next; active = true;
    queueMicrotask(() => next.snapshot({ session_id: S, reset: true, seq, entries: [], pending_requests: pending }));
    return {
      close() { active = false; },
      cmd: async (command: Record<string, unknown>) => {
        expect(command.id).toBe(pending[0]!.id);
        pending = [];
        resolveWait(command.confirmed);
        return {};
      },
    };
  } } as unknown as HostClient;
  const stream = new SessionStreamClient(client, S, {
    seq: () => state.seq, conn() {},
    snapshot: (snap) => route(applySnapshot(state, snap)),
    events: (items) => route(applyEvents(state, items)),
  }, { schedule: (cb) => frames.push(cb) });
  stream.open(); await Promise.resolve();
  setHidden(true);
  expect(active).toBe(false);
  pending = [{ type: 'extension_ui_request', id: 'hidden-ask', method: 'confirm', title: 'Permission?' }];
  seq += 5000; // host ring rolled: open always answers with a full snapshot
  setHidden(false); await Promise.resolve();
  expect(state.dialogs).toEqual(['hidden-ask']);
  expect(registry.dialogs().map((dialog) => dialog.id)).toEqual(['hidden-ask']);
  await stream.cmd({ type: 'extension_ui_response', id: registry.dialogs()[0]!.id, confirmed: true });
  expect(await hostWait).toBe(true);
  stream.reopen(false); await Promise.resolve();
  expect(state.dialogs).toEqual([]);
  expect(registry.dialogs()).toEqual([]);
  stream.close(); registry.dispose();
});
