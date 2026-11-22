/**
 * N-1 / N-3: "Send now" on a queued row (remove → abort → send as a prompt) and Edit on a held row must not leave the
 * text twice in the chat. The real host lists queue rows without ids (durable delivery off) while it identifies other
 * messages, so the queued twin settles by text when it is listed — not later, when the row has left the queue and the
 * delivered message would be taken for ITS receipt instead of the escalated prompt's.
 */
import { describe, expect, it } from 'vitest';
import { applyEvent, applySnapshot, initialState } from './reducer';
import { composerBusy, enqueue, markAcked } from './outbox';
import type { SessionSnapshot } from '../host/client';
import roundtrip from './fixtures/roundtrip.json';

const NOW = 1_793_350_900_000;
function open() {
  const s = initialState('h1', 's1');
  applySnapshot(s, structuredClone((roundtrip as unknown as { fresh: SessionSnapshot }).fresh), NOW);
  s.outboxIdentified = true; // the real host has proved id support by the time anything is queued
  return s;
}
const user = (text: string, at: number) => ({ role: 'user', content: [{ type: 'text', text }], timestamp: at });

function rig() {
  const s = open();
  let seq = s.seq;
  const ev = (event: Record<string, unknown>, now: number) => applyEvent(s, { session: 's1', seq: ++seq, event }, now);
  return { s, ev };
}

describe('Send now on a queued row', () => {
  it('lone row: the escalated prompt is one bubble and the composer goes idle', () => {
    const { s, ev } = rig();
    ev({ type: 'agent_start' }, NOW);
    const a = enqueue(s, { text: 'QA-C1', mode: 'steer' }, true, NOW);
    markAcked(s, a.ref, NOW);
    ev({ type: 'queue_update', steering: ['QA-C1'], followUp: [] }, NOW + 100);
    expect(s.outbox).toEqual([]); // dock only: nothing left to be mistaken for a receipt later
    // escalate: the row is removed, the run is aborted, the text goes out as a prompt
    ev({ type: 'queue_update', steering: [], followUp: [] }, NOW + 5_000);
    ev({ type: 'agent_end' }, NOW + 5_100);
    const b = enqueue(s, { text: 'QA-C1', mode: 'prompt' }, true, NOW + 5_200);
    markAcked(s, b.ref, NOW + 5_300);
    ev({ type: 'agent_start' }, NOW + 5_400);
    ev({ type: 'message_start', message: user('QA-C1', NOW + 5_400) }, NOW + 5_400);
    ev({ type: 'message_end', message: user('QA-C1', NOW + 5_400) }, NOW + 5_400);
    expect(s.outbox).toEqual([]);
    expect(s.entries.filter((e) => e.message?.role === 'user' && JSON.stringify(e.message.content).includes('QA-C1'))).toHaveLength(1);
    ev({ type: 'agent_end' }, NOW + 5_500);
    expect(composerBusy(s)).toBe(false); // no leftover "prompt in flight" keeping Stop up
  });

  it('several rows: the escalated one is one bubble, the others stay queued', () => {
    const { s, ev } = rig();
    ev({ type: 'agent_start' }, NOW);
    const a = enqueue(s, { text: 'QA-A', mode: 'steer' }, true, NOW);
    const c = enqueue(s, { text: 'QA-B', mode: 'followUp' }, true, NOW + 10);
    markAcked(s, a.ref, NOW);
    markAcked(s, c.ref, NOW + 10);
    ev({ type: 'queue_update', steering: ['QA-A'], followUp: ['QA-B'] }, NOW + 100);
    expect(s.outbox).toEqual([]);
    ev({ type: 'queue_update', steering: [], followUp: ['QA-B'] }, NOW + 5_000);
    ev({ type: 'agent_end' }, NOW + 5_100);
    const b = enqueue(s, { text: 'QA-A', mode: 'prompt' }, true, NOW + 5_200);
    markAcked(s, b.ref, NOW + 5_300);
    ev({ type: 'message_end', message: user('QA-A', NOW + 5_400) }, NOW + 5_400);
    expect(s.outbox).toEqual([]);
    expect(s.queue.followUp).toEqual(['QA-B']);
    expect(s.entries.filter((e) => JSON.stringify(e.message?.content ?? '').includes('QA-A'))).toHaveLength(1);
  });

  it('a prompt still going out is never taken for a queue row of the same text', () => {
    const { s, ev } = rig();
    ev({ type: 'queue_update', steering: ['same'], followUp: [] }, NOW);
    const b = enqueue(s, { text: 'same', mode: 'prompt' }, true, NOW + 10);
    markAcked(s, b.ref, NOW + 20);
    ev({ type: 'queue_update', steering: ['same'], followUp: [] }, NOW + 30);
    expect(s.outbox.map((o) => o.ref)).toEqual([b.ref]);
  });
});

describe('Edit on a queued / held row (N-3)', () => {
  it('taking the row out of the queue leaves no outbox bubble behind', () => {
    const { s, ev } = rig();
    const k = enqueue(s, { text: 'QA-K2', mode: 'followUp' }, true, NOW);
    markAcked(s, k.ref, NOW);
    ev({ type: 'queue_update', steering: [], followUp: ['QA-K1', 'QA-K2'] }, NOW + 100);
    ev({ type: 'queue_update', steering: [], followUp: ['QA-K1'] }, NOW + 2_000); // Edit = remove from the host queue
    expect(s.outbox).toEqual([]);
  });
});
