import type { RpcCommand } from './client';
import type { SessionEntry } from '../session/types';

type EntryReply = { entries?: SessionEntry[] };
type EntryChunk = { json: string; offset: number; nextOffset: number | null };

/** The readable reason of a rejection (the host's errors are plain `{code, message}` objects, not `Error`s). */
export function reasonOf(e: unknown): string {
  if (e instanceof Error) return e.message;
  const m = (e as { message?: unknown } | null)?.message;
  return typeof m === 'string' ? m : String(e);
}

/** The ids that came back whole, and why each of the others did not (one failing id never fails the batch). */
export type FullEntriesResult = { entries: SessionEntry[]; errors: Record<string, string> };

/**
 * A `message` entry the host sent without its `role` (seen on the chunk path of a cold host after a relaunch:
 * the typed message was serialised without its tag) would build no turn row at all — the tool call or answer
 * silently vanishes. The role is recoverable from the message shape; an entry that cannot be recognised is a failure.
 */
function withRole(entry: SessionEntry): SessionEntry {
  const m = entry.message as (Record<string, unknown> & { role?: unknown }) | undefined;
  if (entry.type !== 'message' || !m || typeof m.role === 'string') return entry;
  let role: string | undefined;
  if (typeof m.toolCallId === 'string' || typeof m.toolName === 'string') role = 'toolResult';
  else if ('command' in m && 'exitCode' in m) role = 'bashExecution';
  else if (m.provider !== undefined || m.model !== undefined || m.stopReason !== undefined || m.usage !== undefined || m.api !== undefined) role = 'assistant';
  else if (Array.isArray(m.content) || typeof m.content === 'string') role = 'user';
  if (!role) throw new Error('Entry has no role');
  return { ...entry, message: { ...m, role } } as SessionEntry;
}

async function fetchOne(
  cmd: <T>(command: RpcCommand) => Promise<T>,
  id: string,
  entry: SessionEntry | undefined,
): Promise<SessionEntry> {
  if (entry && !entry.truncated && !entry.message?.truncated) return withRole(entry);
  let offset = 0;
  const parts: string[] = [];
  for (;;) {
    const chunk = await cmd<EntryChunk>({ type: 'get_entries', ids: [id], chunkOffset: offset });
    if (typeof chunk?.json !== 'string' || chunk.offset !== offset) {
      // An older host has no chunk protocol: retain its bounded reply.
      if (entry) return withRole(entry);
      throw new Error('Entry chunk unavailable');
    }
    parts.push(chunk.json);
    if (chunk.nextOffset === null) return withRole(JSON.parse(parts.join('')) as SessionEntry);
    if (chunk.nextOffset <= offset) throw new Error('Entry chunk did not advance');
    offset = chunk.nextOffset;
  }
}

/**
 * Fetch frame-sized previews first, then losslessly assemble oversized entries. Every id settles on its own:
 * the ones that failed are listed in `errors`, the rest are returned (a lost preview call falls back to the chunks).
 */
export async function fetchFullEntriesSettled(
  cmd: <T>(command: RpcCommand) => Promise<T>,
  ids: readonly string[],
): Promise<FullEntriesResult> {
  const reply = await cmd<EntryReply>({ type: 'get_entries', ids: [...ids] }).catch(() => undefined);
  const previews = Array.isArray(reply?.entries) ? reply.entries : [];
  const settled = await Promise.allSettled(ids.map((id) => fetchOne(cmd, id, previews.find((entry) => entry.id === id))));
  const result: FullEntriesResult = { entries: [], errors: {} };
  settled.forEach((outcome, i) => {
    if (outcome.status === 'fulfilled') result.entries.push(outcome.value);
    else result.errors[ids[i]!] = reasonOf(outcome.reason);
  });
  return result;
}

/** The whole entries of `ids`. Rejects only when none could be fetched (with the first error). */
export async function fetchFullEntries(
  cmd: <T>(command: RpcCommand) => Promise<T>,
  ids: readonly string[],
): Promise<SessionEntry[]> {
  const { entries, errors } = await fetchFullEntriesSettled(cmd, ids);
  const first = Object.values(errors)[0];
  if (!entries.length && first !== undefined) throw new Error(first);
  return entries;
}
