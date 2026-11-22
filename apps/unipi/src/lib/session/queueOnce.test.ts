/**
 * queue-once: a waiting message lives in the Queue dock only; the host delivering it makes it an
 * ordinary user entry exactly once, and its turn is timed from delivery (QA-1.0.13-rc4 Q-1).
 */
import { describe, expect, it } from 'vitest';
import { applyEvent, applySnapshot, initialState } from './reducer';
import { enqueue, markAcked } from './outbox';
import { buildTurns } from './turns';
import type { SessionEntry } from './types';
import type { SessionSnapshot } from '../host/client';
import roundtrip from './fixtures/roundtrip.json';

const NOW = 1_793_350_900_000;
const snap = () => structuredClone((roundtrip as unknown as { fresh: SessionSnapshot }).fresh);

function open() {
  const s = initialState('h1', 's1');
  applySnapshot(s, snap(), NOW);
  return s;
}
const userEntries = (s: ReturnType<typeof open>, text: string) =>
  s.entries.filter((e) => e.message?.role === 'user' && JSON.stringify(e.message.content).includes(text));

describe('queue → delivery', () => {
  it('the queued message appears once, as a user entry, when the host delivers it', () => {
    const s = open();
    let seq = s.seq;
    const ev = (event: Record<string, unknown>, now: number) => applyEvent(s, { session: 's1', seq: ++seq, event }, now);
    const sent = enqueue(s, { text: 'then add tests', mode: 'followUp' }, true, NOW);
    markAcked(s, sent.ref, NOW);
    ev({ type: 'queue_update', steering: [], followUp: ['then add tests'], followUpIds: [sent.messageId] }, NOW + 100);
    expect(s.outbox).toEqual([]);
    expect(userEntries(s, 'then add tests')).toHaveLength(0);
    // The host delivers it a minute later: queue shrinks, then the user message lands.
    ev({ type: 'queue_update', steering: [], followUp: [] }, NOW + 60_000);
    ev({ type: 'message_end', message: { role: 'user', content: [{ type: 'text', text: 'then add tests' }], timestamp: NOW } }, NOW + 60_010);
    const entries = userEntries(s, 'then add tests');
    expect(entries).toHaveLength(1);
    // Stamped with the delivery time, not the time it was typed.
    expect(Date.parse(entries[0]!.timestamp!)).toBe(NOW + 60_010);
    expect(s.queue).toMatchObject({ steering: [], followUp: [] });
  });

  it('a message that was never queued keeps its own time', () => {
    const s = open();
    applyEvent(s, { session: 's1', seq: s.seq + 1, event: { type: 'message_end', message: { role: 'user', content: [{ type: 'text', text: 'plain' }], timestamp: NOW - 3_000 } } }, NOW);
    expect(Date.parse(userEntries(s, 'plain')[0]!.timestamp!)).toBe(NOW - 3_000);
  });

  it('a message that is still listed when it lands is stamped at delivery too', () => {
    const s = open();
    applyEvent(s, { session: 's1', seq: s.seq + 1, event: { type: 'queue_update', steering: ['mind the API'], followUp: [] } }, NOW);
    applyEvent(s, { session: 's1', seq: s.seq + 1, event: { type: 'message_end', message: { role: 'user', content: [{ type: 'text', text: 'mind the API' }], timestamp: NOW } } }, NOW + 5_000);
    expect(Date.parse(userEntries(s, 'mind the API')[0]!.timestamp!)).toBe(NOW + 5_000);
  });
});

describe('queued follow-up turn timing (Q-1)', () => {
  const at = (ms: number) => new Date(ms).toISOString();
  const T = Date.UTC(2026, 10, 1, 9, 0, 0);
  const entries = (queuedAtOffset: number, deliveredAtOffset: number): SessionEntry[] => [
    { type: 'message', id: 'u', parentId: null, timestamp: at(T + deliveredAtOffset), message: { role: 'user', content: [{ type: 'text', text: 'follow up' }], timestamp: T + queuedAtOffset } },
    { type: 'message', id: 'a', parentId: 'u', timestamp: at(T + deliveredAtOffset + 19_000), message: { role: 'assistant', content: [{ type: 'text', text: 'done' }], stopReason: 'stop', timestamp: T + deliveredAtOffset + 1_000 } },
  ];
  const turn = (e: SessionEntry[]) => buildTurns({ entries: e, leafId: 'a', entriesRev: 1, liveTools: {}, run: { running: false } })[0]!;

  it('counts from delivery, not from queueing', () => {
    const t = turn(entries(0, 120_000));
    expect(t.startedAt).toBe(T + 120_000);
    expect(t.durationMs).toBe(19_000);
  });
  it('an ordinary prompt (typed and delivered together) is unchanged', () => {
    const t = turn(entries(0, 5));
    expect(t.durationMs).toBeGreaterThanOrEqual(19_000);
    expect(t.durationMs).toBeLessThan(19_100);
  });
});
