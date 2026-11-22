import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost } from '../host/mock';
import type { SessionEventDto, SessionSnapshot } from '../host/client';
import { createSession } from './store';
import { applySnapshot, initialState } from './reducer';
import { enqueue, markSending, queueImageKey, settleByEntry, settleByMessage, settleByQueue } from './outbox';
import type { AgentMessage, SessionEntry } from './types';

const tick = async () => { for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0)); };
const empty = (): SessionSnapshot => ({ reset: true, seq: 0, session_id: 's', entries: [], leaf_id: null, state: {}, commands: [] });
const cleanup: (() => void)[] = [];
beforeEach(() => localStorage.clear());
afterEach(() => { for (const f of cleanup.splice(0)) f(); vi.restoreAllMocks(); });

function harness() {
  const calls: { retryMessageId?: string; message?: string }[] = [];
  const releases: ((reply: unknown) => void)[] = [];
  const frames: (() => void)[] = [];
  const w = createMockHost({ name: 'identity', hosts: [{ id: 'h', name: 'h', sessions: { s: {
    snapshot: empty(), events: [], cmd: { prompt: (c) => { calls.push(c as { retryMessageId?: string; message?: string }); return new Promise((r) => releases.push(r)); } },
  } } }] });
  const { s, dispose } = createRoot((dispose) => ({ s: createSession('h', 's', { client: w.client('h'), schedule: (f) => frames.push(f) }), dispose }));
  cleanup.push(() => { s.close(); dispose(); });
  let seq = 0;
  return { s, calls, releases, w,
    push(events: unknown[]) {
      w.push('h', 's', events.map((event) => ({ session: 's', seq: ++seq, event })) as SessionEventDto[]);
      for (const f of frames.splice(0)) f();
    },
  };
}

for (const skew of [0, -10_000, 10_000]) for (const text of ['/swarm repeat', 'repeat']) {
  it(`G4-1 real store: ${text}, clockSkew=${skew}`, async () => {
    const h = harness(); await tick();
    const a = h.s.send(text, { mode: 'prompt' }); await tick();
    const b = h.s.send(text, { mode: 'prompt' });
    const message = { role: 'user', timestamp: Date.now() - skew, content: [{ type: 'text', text: 'repeat' }] };
    const id = text.startsWith('/') ? `${a.messageId}:extension:abcdef:0` : a.messageId!;
    h.push([
      ...(text.startsWith('/') ? [{ type: 'entry_appended', entry: { id: 'echo-a', type: 'custom', customType: 'oino.command-echo', data: { name: 'swarm', args: 'repeat', messageId: a.messageId } } }] : []),
      { type: 'message_start', message },
      { type: 'entry_appended', entry: { id, type: 'message', message } },
      { type: 'message_end', message },
    ]);
    expect(h.s.state.outbox.map((o) => o.ref)).toEqual([b.ref]);
    expect(h.s.state.outbox[0].status).toBe('preparing');
    h.releases[0]({ messageId: a.messageId }); await tick();
    expect(h.calls.map((c) => c.retryMessageId)).toEqual([a.messageId, b.messageId]);
    // A replay after B dispatch cannot claim B, with or without lifecycle ids.
    h.push([{ type: 'message_start', message }, { type: 'message_end', message }]);
    expect(h.s.state.outbox.map((o) => o.ref)).toEqual([b.ref]);
    h.push([{ type: 'entry_appended', entry: { type: 'message', id: b.messageId, message } }]);
    expect(h.s.state.outbox).toEqual([]);
    h.releases[1]({ messageId: b.messageId }); await tick();
    expect(h.s.state.outbox).toEqual([]);
  });
}

it('G4-2: an unrelated identified reset never consumes an identical pending send', () => {
  for (const text of ['repeat', '/swarm repeat']) {
    const s = initialState('h', 's');
    const item = enqueue(s, { text }, true);
    applySnapshot(s, { ...empty(), seq: 1, entries: [{ type: 'message', id: 'different-request', message: { role: 'user', content: text } }] });
    expect(s.outbox.map((o) => o.ref)).toEqual([item.ref]);
  }
});

it('unidentified old-host single-send start/end still settles once', () => {
  for (const text of ['/swarm repeat', 'repeat']) {
    const s = initialState('h', 's');
    enqueue(s, { text }, true);
    const message = { role: 'user', timestamp: Date.now(), content: 'repeat' };
    settleByMessage(s, message);
    expect(s.outbox).toEqual([]);
    const b = enqueue(s, { text }, true);
    settleByMessage(s, message);
    expect(s.outbox.map((o) => o.ref)).toEqual([b.ref]);
  }
});

it('never settles preparing items, even by their own identity', () => {
  const s = initialState('h', 's');
  const item = enqueue(s, { text: 'same' }, true);
  item.status = 'preparing';
  settleByEntry(s, { id: item.messageId, message: { role: 'user', content: 'same' } });
  settleByMessage(s, { role: 'user', id: item.messageId, content: 'same' });
  expect(s.outbox).toHaveLength(1);
  markSending(s, item.ref);
  settleByEntry(s, { id: item.messageId, message: { role: 'user', content: 'same' } });
  expect(s.outbox).toEqual([]);
});

it('queue receipts settle by identity, never identical text from another request', () => {
  const s = initialState('h', 's');
  const a = enqueue(s, { text: 'same', mode: 'followUp' }, true);
  const b = enqueue(s, { text: 'same', mode: 'followUp' }, true);
  settleByQueue(s, ['same'], [a.messageId!]);
  expect(s.outbox.map((o) => o.ref)).toEqual([b.ref]);
  settleByQueue(s, ['same'], [a.messageId!]);
  expect(s.outbox.map((o) => o.ref)).toEqual([b.ref]);
});

it('queue receipts keep the pasted image by id for the dock, and drop it when the message leaves the queue', () => {
  const s = initialState('h', 's');
  const img = { mimeType: 'image/png', data: 'AAAA' };
  const a = enqueue(s, { text: '[image 1] hi', images: [img], mode: 'followUp' }, true);
  const b = enqueue(s, { text: 'no image', mode: 'followUp' }, true);
  settleByQueue(s, ['[image 1] hi', 'no image'], [a.messageId!, b.messageId!]);
  expect(s.outbox).toEqual([]);
  expect(s.queueImages).toEqual({ [a.messageId!]: [img], [queueImageKey(null, '[image 1] hi')]: [img] });
  settleByQueue(s, ['no image'], [b.messageId!]); // `a` was delivered
  expect(s.queueImages).toBeUndefined();
});

it('the image survives the host listing the same message again without its id', () => {
  const s = initialState('h', 's');
  const img = { mimeType: 'image/png', data: 'AAAA' };
  const a = enqueue(s, { text: '[image 1] hi', images: [img], mode: 'followUp' }, true);
  settleByQueue(s, ['[image 1] hi'], [a.messageId!]);
  settleByQueue(s, ['[image 1] hi'], [null]); // the twin is gone by now
  expect(s.queueImages?.[queueImageKey(null, '[image 1] hi')]).toEqual([img]);
});

it('a host that sends no queue ids: the image is kept by the queued text, and the outbox twin settles by text (N-1)', () => {
  const s = initialState('h', 's');
  const img = { mimeType: 'image/png', data: 'AAAA' };
  enqueue(s, { text: '[image 1]  hi there', images: [img], mode: 'steer' }, true);
  s.outboxIdentified = true; // the host proved id support elsewhere; an id-less row still settles its twin by text
  settleByQueue(s, ['[image 1] hi there'], [null]);
  expect(s.outbox.map((o) => o.ref)).toEqual([]);
  expect(s.queueImages).toEqual({ [queueImageKey(null, '[image 1] hi there')]: [img] });
  settleByQueue(s, [], []);
  expect(s.queueImages).toBeUndefined();
});

it('unexpected removal behind sendTail becomes a visible persisted Retry failure', async () => {
  const log = vi.spyOn(console, 'error').mockImplementation(() => {});
  const h = harness(); await tick();
  h.s.send('first', { mode: 'prompt' }); await tick();
  const b = h.s.send('second', { mode: 'prompt' });
  h.s.dismiss(b.ref);
  h.releases[0]({}); await tick();
  expect(h.calls).toHaveLength(1);
  expect(h.s.state.outbox.find((o) => o.ref === b.ref)).toMatchObject({ status: 'lost', error: expect.stringContaining('Retry') });
  expect(log).toHaveBeenCalledOnce();
  h.s.resend(b.ref); await tick();
  expect(h.calls.map((c) => c.retryMessageId)).toEqual([expect.any(String), b.messageId]);
  h.releases[1]({ messageId: b.messageId }); await tick();
});

// Seeded model-based property test: random event order, duplicates, resets and reconnect
// replay; all commands pass through the real serialized store with delayed ACKs.
it('random interleavings preserve transport exactly-once and identity-exact bubbles (40 seeds)', async () => {
  for (let seed = 1; seed <= 40; seed++) {
    let rng = Math.imul(seed, 0x9e3779b9) >>> 0;
    const random = () => { rng = (Math.imul(rng, 1664525) + 1013904223) >>> 0; return rng / 2 ** 32; };
    const h = harness(); await tick();
    const n = 3 + Math.floor(random() * 8);
    const items = Array.from({ length: n }, (_, i) => h.s.send(random() < 0.5 ? 'identical' : `distinct ${i}`, { mode: 'prompt' }));
    await tick();
    const entries: SessionEntry[] = [];
    const remaining = new Set(items.map((o) => o.messageId));
    // Every dispatch is gated by the previous command's ACK, but events may arrive on either side.
    const pending: (() => void)[] = [];
    for (let i = 0; i < n; i++) {
      expect(h.calls.map((c) => c.retryMessageId)).toEqual(items.slice(0, i + 1).map((o) => o.messageId));
      const item = items[i];
      const id = random() < 0.5 ? item.messageId! : `${item.messageId}:extension:abcdef:${i}`;
      const message: AgentMessage = { role: 'user', id, content: item.text, timestamp: i };
      const arrive = () => {
        const event = { type: 'entry_appended', entry: { type: 'message', id, message } };
        entries.push(event.entry);
        h.push([event]); remaining.delete(item.messageId);
      };
      // Identified lifecycle events may be the first admission observation.
      const lifecycle = () => { h.push([{ type: 'message_start', message }, { type: 'message_end', message }]); remaining.delete(item.messageId); };
      if (random() < 0.3) pending.push(arrive, lifecycle);
      else if (random() < 0.5) { lifecycle(); pending.push(arrive); }
      else { arrive(); pending.push(lifecycle); }
      if (random() < 0.6 && pending.length) pending.splice(Math.floor(random() * pending.length), 1)[0]();
      // Reset/reconnect snapshots carry only already observed entries, never unrelated fallback.
      if (random() < 0.5) {
        const snapshot = { ...empty(), seq: h.s.state.seq, entries: [...entries] };
        const state = initialState('h', 's');
        state.outbox = h.s.state.outbox.map((o) => ({ ...o }));
        applySnapshot(state, snapshot);
        applySnapshot(state, { ...snapshot, reset: false, replay: [] });
        expect(state.outbox.map((o) => o.messageId).sort()).toEqual([...remaining].sort());
      }
      expect(h.s.state.outbox.map((o) => o.messageId).sort()).toEqual([...remaining].sort());
      h.releases[i]({ messageId: item.messageId }); await tick();
      // A real stream reset while later sends wait behind sendTail, then a resume
      // reconnect. The mock snapshot is authoritative and contains only admitted entries.
      if (random() < 0.4) {
        h.w.scenario.hosts[0].sessions!.s.snapshot = { ...empty(), seq: h.s.state.seq, entries: [...entries] };
        h.w.skipFrame('h', 's'); await tick();
        expect(h.s.state.outbox.map((o) => o.messageId).sort()).toEqual([...remaining].sort());
        h.w.drop('h', 's'); h.s.retry(); await tick();
        expect(h.s.state.outbox.map((o) => o.messageId).sort()).toEqual([...remaining].sort());
      }
    }
    while (pending.length) pending.splice(Math.floor(random() * pending.length), 1)[0]();
    expect(h.s.state.outbox).toEqual([]);
    expect(new Set(h.calls.map((c) => c.retryMessageId)).size).toBe(n);
    h.s.close();
  }
});

// 1.0.10 owner report: the real host publishes a user message only as id-less `message_start` / `message_end`
// (no `entry_appended`), in a chat whose history already has UUID entries. The bubble must still go.
it('real-host shape: id-less user lifecycle settles a send in an identified chat, once', async () => {
  const h = harness(); await tick();
  h.push([{ type: 'entry_appended', entry: { type: 'custom', customType: 'oino.command-echo', id: 'e0', data: { name: 'goal', args: 'x', messageId: '0b0e5f3a-1111-4111-8111-111111111111' } } }]);
  const a = h.s.send('hello', { mode: 'prompt' }); await tick();
  const b = h.s.send('hello', { mode: 'prompt' }); await tick();
  const message = { role: 'user', timestamp: Date.now(), content: [{ type: 'text', text: 'hello' }] };
  h.push([{ type: 'message_start', message }]);
  expect(h.s.state.outbox.map((o) => o.ref)).toEqual([b.ref]);
  h.push([{ type: 'message_end', message }]);
  expect(h.s.state.outbox.map((o) => o.ref)).toEqual([b.ref]);
  h.releases[0]({ messageId: a.messageId }); await tick();
  const second = { ...message, timestamp: message.timestamp + 5 };
  h.push([{ type: 'message_start', message: second }, { type: 'message_end', message: second }]);
  expect(h.s.state.outbox).toEqual([]);
  h.releases[1]({ messageId: b.messageId }); await tick();
  expect(h.s.state.outbox).toEqual([]);
  expect(h.calls.map((c) => c.retryMessageId)).toEqual([a.messageId, b.messageId]);
});
