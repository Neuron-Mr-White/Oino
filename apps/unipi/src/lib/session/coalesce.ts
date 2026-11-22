/**
 * w3-hidden: while the window is hidden a session stream that is still open keeps its
 * queued events compact instead of growing by one entry per delta (no animation frame ever
 * flushes it). `pushCoalescing` folds an event into the queue's tail when the reducer would
 * end in the very same state either way:
 *
 * - `message_update` carrying the full partial `message`: the reducer takes that message
 *   whole, so the newest one supersedes the previous update;
 * - `message_update` deltas without a full message: consecutive text / thinking deltas of
 *   one block concatenate;
 * - `tool_execution_update` of one tool: the newest partial output supersedes the previous.
 *
 * A folded event keeps the NEWEST `seq` and records the first folded `seq` as `fromSeq`, so
 * the reducer's gap check still sees a contiguous run. Anything else is appended as is.
 */
import type { SessionEventDto } from '../host/client';

export type QueuedEvent = SessionEventDto & {
  /** The first `seq` folded into this event (absent: just `seq`). */
  fromSeq?: number;
};

type Obj = Record<string, unknown>;
const isObject = (v: unknown): v is Obj => !!v && typeof v === 'object' && !Array.isArray(v);

/** The text a tool's partial result shows (mirrors the reducer's own reading). */
function partialText(result: unknown): string | undefined {
  if (!isObject(result)) return typeof result === 'string' ? result : undefined;
  const content = result.content;
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return undefined;
  return content
    .filter((p): p is { type: string; text: string } => isObject(p) && p.type === 'text' && typeof p.text === 'string')
    .map((p) => p.text)
    .join('');
}

const hasFullMessage = (event: Obj): boolean => isObject(event.message) && Array.isArray(event.message.content);

function deltaOf(event: Obj): Obj | undefined {
  const d = event.assistantMessageEvent;
  return isObject(d) && (d.type === 'text_delta' || d.type === 'thinking_delta') && typeof d.delta === 'string' ? d : undefined;
}

/** The event `item` folded into `prev`, or `undefined` when they cannot merge exactly. */
function fold(prev: QueuedEvent, item: QueuedEvent): QueuedEvent | undefined {
  if (prev.session !== item.session || item.seq <= prev.seq) return undefined;
  const a = (prev.event ?? {}) as Obj;
  const b = (item.event ?? {}) as Obj;
  if (a.type !== b.type || a.type === undefined) return undefined;
  const from = prev.fromSeq ?? prev.seq;
  if (a.type === 'message_update') {
    if (hasFullMessage(b)) return { ...item, fromSeq: from };
    const da = deltaOf(a);
    const db = deltaOf(b);
    if (!da || !db || hasFullMessage(a) || da.type !== db.type || da.contentIndex !== db.contentIndex) return undefined;
    return { ...item, fromSeq: from, event: { ...b, assistantMessageEvent: { ...db, delta: String(da.delta) + String(db.delta) } } };
  }
  if (a.type === 'tool_execution_update') {
    if (typeof a.toolCallId !== 'string' || a.toolCallId !== b.toolCallId || a.toolName !== b.toolName) return undefined;
    // The reducer keeps an earlier `details` / text a later update lacks, and builds the live
    // tool from the first update's `args`: only fold when nothing of that would be lost.
    if (partialText(b.partialResult) === undefined) return undefined;
    const bDetails = isObject(b.partialResult) ? b.partialResult.details : undefined;
    const aDetails = isObject(a.partialResult) ? a.partialResult.details : undefined;
    if (bDetails === undefined && aDetails !== undefined) return undefined;
    if (JSON.stringify(a.args) !== JSON.stringify(b.args)) return undefined;
    return { ...item, fromSeq: from };
  }
  return undefined;
}

/** Appends `item` to `queue`, folding it into the tail when that is exact. */
export function pushCoalescing(queue: QueuedEvent[], item: QueuedEvent): void {
  const tail = queue[queue.length - 1];
  const merged = tail && fold(tail, item);
  if (merged) queue[queue.length - 1] = merged;
  else queue.push(item);
}
