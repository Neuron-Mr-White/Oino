/**
 * Outbox (app spec §3.4 step 5, Z-50; ported from the v1 chat store's
 * `Pending` / `Outboxed`): every prompt the app sends shows as a pending
 * bubble until the host's user `message_start` for it arrives.
 *
 *   waiting ──(live)──▶ sending ──(cmd ok)──▶ acked ──(message_start / queue_update)──▶ gone
 *      ▲                   │                    └──(20 s, no entry: template/extension)──▶ gone
 *      └──(not live)       └──(cmd failed / 30 s no ack)──▶ lost ──(Retry)──▶ sending
 *
 * `waiting` and `lost` items are persisted per host+session (they survive an
 * app restart); the rest live in memory. Pure functions over `SessionState`
 * (the store runs them inside `produce`).
 */
import { textOf, type Part as ModelPart } from '../chat/model';
import type { AgentMessage, OutboxImage, OutboxItem, OutboxMode, SessionState } from './types';

/** An acked prompt whose entry never came (template, extension input handler…) goes quietly after this. */
export const PENDING_ACKED_MS = 20_000;
/** No ack at all (the link dropped mid-send): reported as lost. */
export const PENDING_LOST_MS = 30_000;

let refSeq = 0;
/** A local correlation ref for one outgoing prompt. */
export function newRef(now = Date.now()): string {
  refSeq += 1;
  return `o${now.toString(36)}${refSeq.toString(36)}`;
}

const norm = (text: string) => text.replace(/\s+/g, ' ').trim();

/** Adds a prompt to the outbox: `sending` when the stream is live, else `waiting`. */
export function enqueue(s: SessionState, input: { text: string; images?: OutboxImage[]; mode?: OutboxMode; ref?: string }, live: boolean, now = Date.now()): OutboxItem {
  const item: OutboxItem = { ref: input.ref ?? newRef(now), text: input.text, mode: input.mode ?? 'prompt', at: now, status: live ? 'sending' : 'waiting' };
  if (input.images?.length) item.images = input.images;
  s.outbox.push(item);
  return item;
}

function find(s: SessionState, ref: string): OutboxItem | undefined {
  return s.outbox.find((o) => o.ref === ref);
}

/** The host acked the command. */
export function markAcked(s: SessionState, ref: string, now = Date.now()): void {
  const item = find(s, ref);
  if (!item || item.status === 'waiting') return;
  item.status = 'acked';
  item.ackedAt = now;
  delete item.error;
}

/** The command failed (or never got an ack). */
export function markLost(s: SessionState, ref: string, error?: string): void {
  const item = find(s, ref);
  if (!item) return;
  item.status = 'lost';
  if (error) item.error = error;
}

/** Back to `sending` for a resend (Retry / flush). */
export function markSending(s: SessionState, ref: string, now = Date.now()): void {
  const item = find(s, ref);
  if (!item) return;
  item.status = 'sending';
  item.at = now;
  delete item.ackedAt;
  delete item.error;
}

/** The user dismissed it (✕). */
export function removeItem(s: SessionState, ref: string): void {
  const i = s.outbox.findIndex((o) => o.ref === ref);
  if (i >= 0) s.outbox.splice(i, 1);
}

/** A user message reached the session: its bubble (oldest in-flight with the same text) goes. */
export function settleByMessage(s: SessionState, message: AgentMessage): void {
  if (!s.outbox.length) return;
  const text = norm(textOf(message.content as string | ModelPart[] | undefined));
  const i = s.outbox.findIndex((o) => (o.status === 'sending' || o.status === 'acked') && norm(o.text) === text);
  if (i >= 0) s.outbox.splice(i, 1);
}

/** Steers / follow-ups pi queued show in the queue strip (A11), not as pending bubbles. */
export function settleByQueue(s: SessionState, texts: readonly string[]): void {
  if (!s.outbox.length || !texts.length) return;
  const queued = texts.map(norm);
  for (let i = s.outbox.length - 1; i >= 0; i--) {
    const o = s.outbox[i]!;
    if (o.mode === 'prompt' || (o.status !== 'acked' && o.status !== 'sending')) continue;
    const k = queued.indexOf(norm(o.text));
    if (k >= 0) {
      queued.splice(k, 1);
      s.outbox.splice(i, 1);
    }
  }
}

/** Time-settles in-flight items; returns the refs that became `lost`. */
export function sweep(s: SessionState, now = Date.now()): string[] {
  const lost: string[] = [];
  for (let i = s.outbox.length - 1; i >= 0; i--) {
    const o = s.outbox[i]!;
    if (o.status === 'acked' && now - (o.ackedAt ?? o.at) > PENDING_ACKED_MS) s.outbox.splice(i, 1);
    else if (o.status === 'sending' && now - o.at > PENDING_LOST_MS) {
      o.status = 'lost';
      o.error = 'No answer from the session';
      lost.unshift(o.ref);
    }
  }
  return lost;
}

// ── persistence (waiting + lost only) ───────────────────────────────────

const key = (hostId: string, sessionId: string) => `unipi.session.outbox.${hostId}.${sessionId}`;

export function loadOutbox(hostId: string, sessionId: string): OutboxItem[] {
  try {
    const raw = localStorage.getItem(key(hostId, sessionId));
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((o): o is OutboxItem => !!o && typeof o === 'object' && typeof (o as OutboxItem).ref === 'string' && typeof (o as OutboxItem).text === 'string');
  } catch {
    return [];
  }
}

export function saveOutbox(hostId: string, sessionId: string, items: readonly OutboxItem[]): void {
  try {
    const keep = items.filter((o) => o.status === 'waiting' || o.status === 'lost').map((o) => ({ ...o }));
    if (keep.length) localStorage.setItem(key(hostId, sessionId), JSON.stringify(keep));
    else localStorage.removeItem(key(hostId, sessionId));
  } catch {
    // storage full / unavailable: the outbox just won't survive a restart.
  }
}
