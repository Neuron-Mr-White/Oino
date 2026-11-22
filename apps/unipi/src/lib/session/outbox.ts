/**
 * Outbox (app spec §3.4 step 5, Z-50; ported from the v1 chat store's
 * `Pending` / `Outboxed`): every prompt the app sends shows as a pending
 * bubble until the host's user `message_start` for it arrives.
 *
 *   waiting ──(live)──▶ preparing ──(dispatch)──▶ sending ──(cmd ok)──▶ acked ──(receipt)──▶ gone
 *      ▲                   │                    └──(20 s, no entry: template/extension)──▶ gone
 *      └──(not live)       └──(cmd failed / 30 s no ack)──▶ lost ──(Retry)──▶ sending
 *
 * `waiting` and `lost` items are persisted per host+session (they survive an
 * app restart); the rest live in memory. Pure functions over `SessionState`
 * (the store runs them inside `produce`).
 */
import { textOf, type Part as ModelPart } from '../chat/model';
import type { AgentMessage, OutboxImage, OutboxItem, OutboxMode, SessionEntry, SessionState } from './types';

/** An acked prompt whose entry never came (template, extension input handler…) goes quietly after this. */
export const PENDING_ACKED_MS = 20_000;
/** No ack at all (the link dropped mid-send): reported as lost. */
export const PENDING_LOST_MS = 30_000;

let refSeq = 0;
/**
 * The durable identity of one outgoing prompt, assigned when it is typed and sent as `retryMessageId`: the host adopts
 * it as the message's id, so everything the command produces (`oino.command-echo`, the user entry, an extension's own
 * user messages: `<id>:extension:…`) can be paired with this bubble by id — never by text and clock.
 */
export function newMessageId(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  if (c?.randomUUID) return c.randomUUID();
  const hex = (n: number) => Array.from({ length: n }, () => Math.floor(Math.random() * 16).toString(16)).join('');
  return `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`;
}

/** A local correlation ref for one outgoing prompt. */
export function newRef(now = Date.now()): string {
  refSeq += 1;
  return `o${now.toString(36)}${refSeq.toString(36)}`;
}

const norm = (text: string) => text.replace(/\s+/g, ' ').trim();

/** pi expands a `/skill:<name> args` prompt into `<skill name="…">…</skill>\n\nargs`. */
const SKILL_BLOCK_RE = /^\s*<skill name="([^"]+)"[^>]*>[\s\S]*?<\/skill>/;
const SKILL_CMD_RE = /^\s*\/skill:([^\s]+)/;

/** The text a sent prompt and its echoed user message share: a leading skill
 *  invocation (sent `/skill:x …`, echoed `<skill name="x">…</skill> …`) becomes `/skill:x`. */
function canon(text: string): string {
  const block = SKILL_BLOCK_RE.exec(text);
  if (block) return norm(`/skill:${block[1]} ${text.slice(block[0].length)}`);
  return norm(text);
}

/** The long-horizon commands whose own user message lands only after the extension's hooks ran (D3). */
const MODE_COMMAND_RE = /^\/(?:unipi:)?(goal|swarm|graph|ralph)$/;

/**
 * `/swarm text` → `/swarm text` normalised (whitespace collapsed), for a `goal` / `swarm` / `graph` / `ralph` command with
 * args; undefined for anything else. The outbox bubble and the command echo of the same command share this key.
 */
export function modeCommandKey(name: string, args: string): string | undefined {
  const base = MODE_COMMAND_RE.exec(`/${name.replace(/^\//, '')}`)?.[1];
  const rest = norm(args);
  return base && rest ? `/${base} ${rest}` : undefined;
}

/** The key of a typed `/swarm <args>` prompt. */
export function modeCommandKeyOf(text: string): string | undefined {
  const m = /^\s*\/(\S+)(?:\s+([\s\S]*))?$/.exec(text);
  return m ? modeCommandKey(m[1]!, m[2] ?? '') : undefined;
}

/** Clock slack between this app's `OutboxItem.at` and the host's entry timestamp. */
const ECHO_SKEW_MS = 2_000;

/**
 * Which outbox items already show as a command echo's own bubble.
 *
 * By identity (G3-2 beqa): the host's `oino.command-echo` carries the `messageId` of the request that ran the
 * command, and an outbox item carries the id it sent (`retryMessageId`) — equal ids pair, whatever the clocks say.
 *
 * Only when an id is missing on a side (an older host's echo, a waiting item an older app persisted) does the G2-4
 * heuristic apply: the echo pairs with ONE item of its key that was sent no later than the echo (2 s clock slack),
 * an echo being claimed by at most one item (oldest item first), so a repeated identical command keeps its own
 * pending bubble. Two different ids never pair, and an echo with an id is not claimed by an id-less item while
 * another item owns that id.
 */
export function echoedRefs(items: readonly OutboxItem[], turns: readonly { echoKey?: string; echoMessageId?: string; startedAt?: number }[]): Set<string> {
  const paired = new Set<string>();
  const claimed = new Set<number>();
  const ordered = [...items].sort((a, b) => a.at - b.at);
  const live = ordered.filter((item) => item.status !== 'lost' && modeCommandKeyOf(item.text));
  for (const item of live) {
    if (!item.messageId) continue;
    const i = turns.findIndex((t, k) => t.echoMessageId === item.messageId && !claimed.has(k));
    if (i < 0) continue;
    claimed.add(i);
    paired.add(item.ref);
  }
  const owned = new Set(items.flatMap((item) => (item.messageId ? [item.messageId] : [])));
  for (const item of live) {
    if (paired.has(item.ref)) continue;
    const key = modeCommandKeyOf(item.text);
    const i = turns.findIndex(
      (t, k) => t.echoKey === key && !claimed.has(k) && (t.echoMessageId === undefined || (!item.messageId && !owned.has(t.echoMessageId))) && (t.startedAt === undefined || t.startedAt >= item.at - ECHO_SKEW_MS),
    );
    if (i < 0) continue;
    claimed.add(i);
    paired.add(item.ref);
  }
  return paired;
}

/** Adds a prompt to the outbox: `sending` when the stream is live, else `waiting`. */
export function enqueue(s: SessionState, input: { text: string; images?: OutboxImage[]; mode?: OutboxMode; ref?: string }, live: boolean, now = Date.now()): OutboxItem {
  const item: OutboxItem = { ref: input.ref ?? newRef(now), text: input.text, mode: input.mode ?? 'prompt', at: now, status: live ? 'sending' : 'waiting', messageId: newMessageId() };
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

/**
 * A user message ENTRY (it has its id) reached the session: the bubble whose id it carries goes — a plain prompt's entry
 * id is the request's id, an extension's own user message (`/goal …`, `/swarm …` hand the text over) is `<id>:extension:…`.
 * Returns whether an item was settled. An id mismatch NEVER permits a text fallback.
 */
export function settleByEntry(s: SessionState, entry: Partial<SessionEntry>): boolean {
  const echo = entry.customType === 'oino.command-echo' ? entry.data as { messageId?: string } | undefined : undefined;
  if (typeof echo?.messageId === 'string' && echo.messageId) { s.outboxIdentified = true; return settleIdentity(s, echo.messageId); }
  if (entry.message?.role !== 'user') return false;
  // An entry id is authoritative even when it belongs to a different request.
  if (entry.id) return settleIdentity(s, entry.id);
  settleByMessage(s, entry.message);
  return false;
}

const inFlight = (o: OutboxItem) => o.status === 'sending' || o.status === 'acked';

function settleIdentity(s: SessionState, id: string): boolean {
  // Old pi entries use short local ids; current durable receipts are UUIDs. A
  // matching request (including extension children) also proves identity support.
  if (/^[0-9a-f]{8}-[0-9a-f-]{27}(?::extension:.*)?$/i.test(id) || s.outbox.some((o) => o.messageId === id || id.startsWith(`${o.messageId}:extension:`))) s.outboxIdentified = true;
  const i = s.outbox.findIndex((o) => inFlight(o) && !!o.messageId && (id === o.messageId || id.startsWith(`${o.messageId}:extension:`)));
  if (i < 0) return false;
  s.outbox.splice(i, 1);
  return true;
}

/**
 * Identity first. The host's user `message_start` / `message_end` carry no id (only custom entries are published as
 * `entry_appended`), so an id-less lifecycle message settles the oldest dispatched bubble with its text, once.
 */
export function settleByMessage(s: SessionState, message: AgentMessage): void {
  if (message.role !== 'user') return;
  const id = [message.id, message.messageId].find((v): v is string => typeof v === 'string' && !!v);
  if (id) { s.outboxIdentified = true; settleIdentity(s, id); return; }
  if (!s.outbox.some(inFlight)) return;
  // A repeated start/end (or replay) of one message must not consume another request,
  // even after its successor is dispatched; `preparing` sends are never eligible.
  const receipt = JSON.stringify([message.timestamp, message.content]);
  if (s.outboxReceipts?.includes(receipt)) return;
  (s.outboxReceipts ??= []).push(receipt);
  if (s.outboxReceipts.length > 256) s.outboxReceipts.shift();
  const text = canon(textOf(message.content as string | ModelPart[] | undefined));
  let i = s.outbox.findIndex((o) => inFlight(o) && canon(o.text) === text);
  // A `/command` the host expands (prompt template, extension input) echoes as different text:
  // the first user message sent after it settles the oldest such bubble, so it never shows twice.
  if (i < 0) {
    const at = (message as { timestamp?: unknown }).timestamp;
    const sentAt = typeof at === 'number' ? at : typeof at === 'string' ? Date.parse(at) : NaN;
    const skill = SKILL_CMD_RE.exec(text)?.[1];
    i = s.outbox.findIndex(
      (o) =>
        inFlight(o) &&
        /^\s*\//.test(o.text) &&
        // An identified host settles `/goal` `/swarm` `/graph` `/ralph` by their echo's id.
        !(s.outboxIdentified && modeCommandKeyOf(o.text)) &&
        (skill === undefined || SKILL_CMD_RE.exec(o.text)?.[1] === skill) &&
        Number.isFinite(sentAt) &&
        sentAt >= o.at - 2_000,
    );
  }
  if (i >= 0) s.outbox.splice(i, 1);
}

/** Where the dock finds the pasted images of one queued row: its host id, else (a host that sends none) its text. */
export const queueImageKey = (id: string | null | undefined, text: string): string => id || `text:${norm(text)}`;

/**
 * The host's `queue_update` carries text only (`[image 1] …`), so while a message waits the dock would show a bare
 * token. This client still holds the pasted images in the outbox item it sent: keep them for the dock, paired by the
 * host's id when it gives one, and by the item's text (steer / follow-up items only) for a host that gives none. Entries of messages that left
 * the queue are dropped; `ids` is aligned with `texts`.
 */
function holdQueueImages(s: SessionState, texts: readonly string[], ids?: readonly (string | null)[]): void {
  const keep = new Set<string>();
  texts.forEach((text, i) => {
    const id = ids?.[i] || undefined;
    // Under the id and under the text: the host may list the same message again later without its id.
    const keys = [queueImageKey(id, text), queueImageKey(undefined, text)];
    for (const key of keys) keep.add(key);
    const twin = s.outbox.find((o) => !!o.images?.length && (id ? o.messageId === id : o.mode !== 'prompt' && norm(o.text) === norm(text)));
    if (twin) for (const key of keys) (s.queueImages ??= {})[key] = twin.images!;
  });
  if (!s.queueImages) return;
  for (const key of Object.keys(s.queueImages)) if (!keep.has(key)) delete s.queueImages[key];
  if (!Object.keys(s.queueImages).length) delete s.queueImages;
}

/** Steers / follow-ups pi queued show in the queue strip (A11), not as pending bubbles. */
export function settleByQueue(s: SessionState, texts: readonly string[], ids?: readonly (string | null)[]): void {
  holdQueueImages(s, texts, ids);
  for (const id of ids ?? []) if (id) settleIdentity(s, id);
  if (!s.outbox.length || !texts.length) return;
  // N-1: a row the host lists WITHOUT an id (durable delivery off) still settles its steer / follow-up twin by text —
  // even on a host that identifies other messages. Left in the outbox it stays hidden only while listed, and once the
  // row leaves the queue (delivered, edited, "Send now") it would be taken for the message's receipt instead of the
  // prompt that really carries it, showing the text twice until the 20 s timeout.
  const queued = texts.filter((_, i) => !ids?.[i]).map(norm);
  if (!queued.length) return;
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
    // A permission card may block command ACKs and queue delivery indefinitely.
    // Preserve the visible bubble and restart its clock while the user decides.
    if (s.approvals?.length && (o.status === 'sending' || o.status === 'acked')) {
      o.at = now;
      if (o.status === 'acked') o.ackedAt = now;
      continue;
    }
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

/**
 * A plain prompt has not started its run yet: still going out (no host ACK), or acked and waiting
 * for `agent_start` (r7 P3-1: the composer read idle for 16–32 ms in between). A `/command` the
 * host may answer without a run is not waited for after its ACK.
 */
export function promptInFlight(s: Pick<SessionState, 'outbox'>, exceptRef?: string): boolean {
  return s.outbox.some((o) => o.ref !== exceptRef && o.mode === 'prompt' && (o.status === 'preparing' || o.status === 'sending' || (o.status === 'acked' && !/^\s*\//.test(o.text))));
}

/** What the composer treats as "running" (D23): a live run, or a prompt between Send and the host's ACK. */
export function composerBusy(s: Pick<SessionState, 'run' | 'outbox'>): boolean {
  return s.run.running || promptInFlight(s);
}
