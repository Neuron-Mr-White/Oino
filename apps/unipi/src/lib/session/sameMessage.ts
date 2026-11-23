/**
 * Is a live user `message_end` the message a persisted entry already holds?
 *
 * The host sends the same user message down two paths and bounds each one differently (host `sessions/bound.rs`):
 *  - a live event is left whole up to 256 KiB, so a pasted image arrives as inline base64;
 *  - `get_entries` / the snapshot rewrite EVERY inline image over 16 KiB into `{type:'image', mediaRef, bytes}`, flag the
 *    message `truncated: true` and cut long text to its head plus a note.
 * So the two copies of one message are not JSON-equal whenever it carries an image ≥ 16 KiB (or long text), and the live
 * copy that arrives after a catch-up read showed up as a second user bubble (DUP-SEND). The live event carries no id, so
 * the pairing is the message's own timestamp (the host's creation time, in ms, identical on both paths) plus its shape.
 */
import { textOf, type Part as ModelPart } from '../chat/model';
import { isProvisional, type AgentMessage, type Part, type SessionEntry } from './types';

/** The suffix the host appends to text it clipped (`TRUNCATION_NOTE`); a prefix of the whole text remains. */
const TRUNCATION_NOTE = '\n…[truncated by the host — expand for the full output]';

const partsOf = (content: AgentMessage['content']): Part[] => (typeof content === 'string' ? [{ type: 'text', text: content }] : Array.isArray(content) ? content : []);

const clipped = (text: string): string => (text.endsWith(TRUNCATION_NOTE) ? text.slice(0, -TRUNCATION_NOTE.length) : text);

/** Same text, or one is the other clipped by the host. */
function sameText(a: unknown, b: unknown): boolean {
  if (typeof a !== 'string' || typeof b !== 'string') return a === b;
  if (a === b) return true;
  const x = clipped(a);
  const y = clipped(b);
  return x.startsWith(y) || y.startsWith(x);
}

function samePart(a: Part, b: Part): boolean {
  if (a.type !== b.type) return false;
  if (a.type === 'text') return sameText(a.text, b.text);
  if (a.type === 'image') {
    if ((a.mimeType ?? a.mime) !== (b.mimeType ?? b.mime)) return false;
    // One side referenced out (`mediaRef`, no bytes inline): the bytes cannot be compared, the type and place can.
    return typeof a.data === 'string' && typeof b.data === 'string' ? a.data === b.data : true;
  }
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Both are the user message created at one instant with the same parts (images compared unless the host referenced one out). */
export function sameUserMessage(a: AgentMessage, b: AgentMessage): boolean {
  if (a.role !== 'user' || b.role !== 'user') return false;
  if (typeof a.timestamp !== 'number' || a.timestamp !== b.timestamp) return false;
  const x = partsOf(a.content);
  const y = partsOf(b.content);
  if (x.length !== y.length) return false;
  // A message of only text with no parts at all: fall back to its text.
  if (!x.length) return textOf(a.content as string | ModelPart[] | undefined) === textOf(b.content as string | ModelPart[] | undefined);
  return x.every((part, i) => samePart(part, y[i]!));
}

/** How far back from the tail a persisted twin is looked for: a catch-up reply that raced the live events ends at the tail. */
const TWIN_WINDOW = 200;

/**
 * The persisted entry that IS this live message (a catch-up / reconcile read already brought it): the same message by
 * `sameUserMessage`, an assistant / system message created at the same instant, a tool result of the same call. Appending the
 * live copy too showed the message twice (and, for a system message, forked the branch) until the next reconcile.
 */
export function hasPersistedMessage(entries: readonly SessionEntry[], message: AgentMessage): boolean {
  const role = message.role;
  if (role !== 'user' && role !== 'assistant' && role !== 'system' && role !== 'toolResult') return false;
  for (let i = entries.length - 1; i >= 0 && i >= entries.length - TWIN_WINDOW; i--) {
    const e = entries[i]!;
    const m = e.type === 'message' ? e.message : undefined;
    if (!m || isProvisional(e.id) || m.role !== role) continue;
    if (role === 'user' ? sameUserMessage(m, message) : role === 'toolResult' ? typeof message.toolCallId === 'string' && m.toolCallId === message.toolCallId : typeof message.timestamp === 'number' && m.timestamp === message.timestamp) return true;
  }
  return false;
}
