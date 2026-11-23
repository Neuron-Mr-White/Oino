/**
 * DUP-SEND (owner, 1.0.13): one message with a pasted image showed as TWO user bubbles — the first followed by
 * "Done in 0s" — until the turn ended. Built from a REAL recording (`fixtures/dup-send.json`): a 1.0.13 host, a chat whose
 * mode had been flipped (so `long-horizon-decision` is published just before the user message), ONE prompt of multi-line
 * text + one pasted JPEG (~70 KB), recorded over app.sock together with the catch-up read the app issues when that
 * published entry names a parent it does not hold.
 *
 * What the recording shows: the live `message_end` keeps the image inline (the host leaves events ≤ 256 KiB whole) while
 * `get_entries` rewrites it to a `mediaRef` and flags the message `truncated` — so the persisted copy and the live copy of
 * the SAME message are not JSON-equal, and a catch-up reply that lands first left both in the timeline.
 */
import { describe, expect, it } from 'vitest';
import { applyEvent, applySnapshot, initialState, reconcileEntries } from './reducer';
import { enqueue, markAcked } from './outbox';
import { sameUserMessage } from './sameMessage';
import { buildTurns } from './turns';
import type { AgentMessage, SessionEntry, SessionState } from './types';
import type { SessionEventDto, SessionSnapshot } from '../host/client';
import fx from './fixtures/dup-send.json';

type Fixture = {
  messageId: string;
  snapshot: SessionSnapshot;
  events: SessionEventDto[];
  catchUpSince: string;
  replyThroughUser: { entries: SessionEntry[]; leafId: string };
  replyFull: { entries: SessionEntry[]; leafId: string };
};
const F = fx as unknown as Fixture;
const NOW = 1_795_428_960_000;

const eventsAt = (...types: string[]) => F.events.filter((e) => types.includes((e.event as { type: string }).type));
const isUserEvent = (e: SessionEventDto, type: string) => (e.event as { type: string; message?: { role?: string } }).type === type && (e.event as { message?: { role?: string } }).message?.role === 'user';
/** Events before the live user message_start / the user message_end / everything after it. */
const idx = (type: string) => F.events.findIndex((e) => isUserEvent(e, type));

function open() {
  const s = initialState('h1', F.snapshot.session_id);
  applySnapshot(s, structuredClone(F.snapshot), NOW);
  return s;
}
const userMessage = () => (F.events[idx('message_end')]!.event as { message: AgentMessage }).message;
/** The user text as the composer sent it (the outbox item the bubble was typed from). */
const sentText = () => (userMessage().content as { type: string; text?: string }[])[0]!.text!;

/** The sender side: the optimistic outbox item, acked like the host's `{disposition:'started'}` reply. */
function sendIt(s: SessionState) {
  const item = enqueue(s, { text: sentText(), images: [{ mimeType: 'image/jpeg', data: 'AAAA' }] }, true, NOW);
  item.messageId = F.messageId;
  markAcked(s, item.ref, NOW + 5);
  return item;
}

/** The app's way: apply each event; the first `catchUp` it asks for is answered with `reply` once `answerAfter` events are in. */
function run(reply: { entries: SessionEntry[]; leafId: string }, answerAfter: number): SessionState {
  const s = open();
  sendIt(s);
  let asked: string | undefined;
  let answered = false;
  const answer = () => {
    answered = true;
    reconcileEntries(s, asked!, structuredClone(reply.entries), reply.leafId);
  };
  F.events.forEach((dto, i) => {
    const r = applyEvent(s, structuredClone(dto), NOW + 10);
    if (r.catchUp && asked === undefined) asked = r.catchUp;
    if (!answered && asked !== undefined && i + 1 >= answerAfter) answer();
  });
  if (!answered && asked !== undefined) answer();
  return s;
}

const userEntries = (s: SessionState) => s.entries.filter((e) => e.type === 'message' && e.message?.role === 'user');
const userTurns = (s: SessionState) =>
  buildTurns({ entries: s.entries, leafId: s.leafId, entriesRev: s.entriesRev, liveTools: s.liveTools, run: s.run }).filter((t) => !!t.user);

describe('DUP-SEND — the real recording', () => {
  it('documents the root cause: the live copy and the persisted copy of one message are not JSON-equal', () => {
    const live = userMessage();
    const persisted = F.replyThroughUser.entries.find((e) => e.id === F.messageId)!.message!;
    expect(JSON.stringify(live.content)).not.toBe(JSON.stringify(persisted.content));
    const liveImage = (live.content as { type: string; data?: string; mediaRef?: string }[])[1]!;
    const persistedImage = (persisted.content as { type: string; data?: string; mediaRef?: string; bytes?: number }[])[1]!;
    expect(liveImage.data).toBeTruthy();
    expect(liveImage.mediaRef).toBeUndefined();
    expect(persistedImage.mediaRef).toBeTruthy();
    expect(persistedImage.data).toBeUndefined();
    expect(persisted.truncated).toBe(true);
    // …and it is the same message: same instant, same parts.
    expect(live.timestamp).toBe(persisted.timestamp);
    expect(sameUserMessage(live, persisted)).toBe(true);
    expect(sameUserMessage(persisted, live)).toBe(true);
  });

  it('the host publishes an entry with an unknown parent before the user message, so the app reads the tail', () => {
    const s = open();
    const results = F.events.slice(0, idx('message_start')).map((dto) => applyEvent(s, structuredClone(dto), NOW));
    expect(results.find((r) => r.catchUp)?.catchUp).toBe(F.catchUpSince);
  });

  // The catch-up read is answered when the host has already persisted the user message; the live events are applied
  // before it, between its parts, or after it (the order the app sees depends on frame batching and image size).
  for (const [name, after, reply] of [
    ['before any live event of the turn', 1, F.replyThroughUser],
    ['between the user message_start and message_end', idx('message_start') + 1, F.replyThroughUser],
    ['right after the user message_end', idx('message_end') + 1, F.replyThroughUser],
    ['after the whole turn', F.events.length, F.replyFull],
  ] as const) {
    it(`one user message in the timeline when the reply lands ${name}`, () => {
      const s = run(reply, after);
      expect(userEntries(s)).toHaveLength(1);
      expect(userTurns(s)).toHaveLength(1);
      expect(s.outbox).toEqual([]);
    });
  }

  it('entries the reply already holds do not move the leaf back when their entry_appended lands afterwards', () => {
    const s = run(F.replyThroughUser, 1);
    // the real host order: [system, user, <live system, recall reminder, assistant>, usage] on ONE branch, the user turn intact
    expect(userTurns(s)).toHaveLength(1);
    expect(s.entries.filter((e) => e.type === 'message' && e.message?.role === 'system')).toHaveLength(1);
    expect(s.entries.filter((e) => e.type === 'message' && e.message?.role === 'assistant')).toHaveLength(1);
  });

  it('an assistant message the reply already holds is not appended again by its live message_end', () => {
    const s = run(F.replyFull, idx('message_end') + 1);
    expect(s.entries.filter((e) => e.type === 'message' && e.message?.role === 'assistant').map((e) => e.id)).toEqual(['91a1a76d']);
    expect(s.entries.some((e) => e.id.startsWith('live:') && e.message?.role !== undefined)).toBe(false);
  });

  it('one user message when no catch-up read happens at all (live event only)', () => {
    const s = open();
    sendIt(s);
    for (const dto of F.events) applyEvent(s, structuredClone(dto), NOW + 10);
    expect(userEntries(s)).toHaveLength(1);
    expect(s.outbox).toEqual([]);
  });

  it('the catch-up reply settles the outbox bubble by id, before any live event of the message arrives', () => {
    const s = open();
    sendIt(s);
    for (const dto of F.events.slice(0, idx('message_start'))) applyEvent(s, structuredClone(dto), NOW);
    expect(s.outbox).toHaveLength(1);
    reconcileEntries(s, F.catchUpSince, structuredClone(F.replyThroughUser.entries), F.replyThroughUser.leafId);
    expect(s.outbox).toEqual([]);
    expect(userEntries(s)).toHaveLength(1);
  });

  it('never settles a bubble of another request (ids are authoritative)', () => {
    const s = open();
    const other = enqueue(s, { text: 'something else entirely' }, true, NOW);
    markAcked(s, other.ref, NOW);
    reconcileEntries(s, F.catchUpSince, structuredClone(F.replyThroughUser.entries), F.replyThroughUser.leafId);
    expect(s.outbox.map((o) => o.ref)).toEqual([other.ref]);
  });

  it('a second message with the same text and image, sent later, is its own message', () => {
    const s = open();
    sendIt(s);
    for (const dto of eventsAt('entry_appended', 'agent_start', 'turn_start', 'message_start', 'message_end')) applyEvent(s, structuredClone(dto), NOW);
    const again = { ...structuredClone(userMessage()), timestamp: userMessage().timestamp! + 60_000 };
    applyEvent(s, { session: s.sessionId, seq: s.seq + 1, event: { type: 'message_end', message: again } }, NOW + 60_000);
    expect(userEntries(s)).toHaveLength(2);
  });
});

describe('sameUserMessage', () => {
  const T = 1_795_428_960_440;
  const user = (content: AgentMessage['content'], timestamp = T): AgentMessage => ({ role: 'user', content, timestamp });
  const img = (extra: Record<string, unknown>) => ({ type: 'image', mimeType: 'image/jpeg', ...extra });

  it('pairs an inline image with its mediaRef twin, text clipped by the host included', () => {
    const live = user([{ type: 'text', text: 'x'.repeat(100) }, img({ data: 'AAAA' })]);
    const kept = user([{ type: 'text', text: `${'x'.repeat(40)}\n…[truncated by the host — expand for the full output]` }, img({ mediaRef: 'ab'.repeat(32), bytes: 3 })]);
    expect(sameUserMessage(live, kept)).toBe(true);
  });
  it('keeps different messages apart', () => {
    const a = user([{ type: 'text', text: 'one' }, img({ data: 'AAAA' })]);
    expect(sameUserMessage(a, user([{ type: 'text', text: 'one' }, img({ data: 'AAAA' })], T + 1))).toBe(false); // another instant
    expect(sameUserMessage(a, user([{ type: 'text', text: 'two' }, img({ data: 'AAAA' })]))).toBe(false); // other text
    expect(sameUserMessage(a, user([{ type: 'text', text: 'one' }]))).toBe(false); // an image fewer
    expect(sameUserMessage(a, user([{ type: 'text', text: 'one' }, img({ data: 'BBBB' })]))).toBe(false); // other inline image
    expect(sameUserMessage(a, user([{ type: 'text', text: 'one' }, { type: 'image', mimeType: 'image/png', data: 'AAAA' }]))).toBe(false); // other type
    expect(sameUserMessage(a, { ...user(a.content), role: 'assistant' })).toBe(false);
    expect(sameUserMessage({ role: 'user', content: 'a' }, { role: 'user', content: 'a' })).toBe(false); // no instant, no identity
  });
  it('a plain string and a one-part text are the same text', () => {
    expect(sameUserMessage(user('hello'), user([{ type: 'text', text: 'hello' }]))).toBe(true);
  });
});
