/**
 * [chat-load] Turn-aligned chunks: a chunk is whole turns only, however the host pages it.
 * The fake host pages like the real one (`page_start`: the newest `limit` entries before
 * the cursor, moved forward to the first user message in the page — steers included).
 */
import { describe, expect, it } from 'vitest';
import { assembleChunk, CHUNK_MAX_ENTRIES, isTurnStart, oldestTurnStart, type HostPage } from './chunks';
import { buildTurns } from './turns';
import type { SessionEntry } from './types';

let n = 0;
const ts = '2026-11-04T00:00:00.000Z';
function user(id: string, text = id): SessionEntry {
  return { type: 'message', id, parentId: null, timestamp: ts, message: { role: 'user', content: text, timestamp: n++ } } as SessionEntry;
}
function assistant(id: string, toolUse = false): SessionEntry {
  return { type: 'message', id, parentId: null, timestamp: ts, message: { role: 'assistant', content: toolUse ? [{ type: 'toolCall', id: `c-${id}`, name: 'bash', arguments: {} }] : [{ type: 'text', text: id }], stopReason: toolUse ? 'toolUse' : 'stop', timestamp: n++ } } as SessionEntry;
}
function result(id: string, call: string): SessionEntry {
  return { type: 'message', id, parentId: null, timestamp: ts, message: { role: 'toolResult', toolCallId: `c-${call}`, toolName: 'bash', content: [{ type: 'text', text: 'ok' }], timestamp: n++ } } as SessionEntry;
}
function step(id: string): SessionEntry {
  return { type: 'custom', id, parentId: null, timestamp: ts, customType: 'subagent-step', data: { kind: 'text', text: id, agentId: 'ag1' } } as unknown as SessionEntry;
}
/** Chains parentIds so `branchOf` walks the list. */
function chain(list: SessionEntry[]): SessionEntry[] {
  list.forEach((e, i) => ((e as { parentId: string | null }).parentId = i ? list[i - 1]!.id : null));
  return list;
}

/** A branch: 150 small turns, one GIANT turn (tool loop + 700 subagent steps + a steer mid-way), a short last turn. */
function owner(): SessionEntry[] {
  const out: SessionEntry[] = [];
  for (let t = 0; t < 150; t++) out.push(user(`u${t}`), assistant(`a${t}`));
  out.push(user('ug'));
  for (let i = 0; i < 350; i++) out.push(step(`s${i}`));
  out.push(assistant('ag1', true), result('rg1', 'ag1'));
  out.push(user('steer')); // a steer: its previous message is a tool result
  for (let i = 350; i < 700; i++) out.push(step(`s${i}`));
  out.push(assistant('ag2'));
  out.push(user('ul'), assistant('al1', true), result('rl1', 'al1'), assistant('al2'));
  return chain(out);
}

function host(all: SessionEntry[]) {
  const asked: Array<{ before: string; limit: number }> = [];
  const isUser = (e: SessionEntry) => e.type === 'message' && e.message?.role === 'user';
  const fetch = async (before: string, limit: number): Promise<HostPage> => {
    asked.push({ before, limit });
    const end = all.findIndex((e) => e.id === before);
    let start = Math.max(0, end - limit);
    if (start > 0) {
      const aligned = all.slice(start, end).findIndex(isUser);
      if (aligned >= 0) start += aligned;
    }
    return { entries: all.slice(start, end), total: all.length, hasMore: start > 0, cursor: all[start]?.id ?? null };
  };
  return { fetch, asked };
}

const turnIds = (entries: SessionEntry[]) => buildTurns({ entries, leafId: entries.at(-1)!.id, entriesRev: 0, run: { phase: 'idle', running: false }, liveTools: {} } as never).map((t) => t.turnId);

describe('chunks: turn starts', () => {
  it('match the turns index exactly (steers are not starts; the first entry only at the branch start)', () => {
    const all = owner();
    const starts = all.flatMap((e, i) => (isTurnStart(all, i, true) ? [e.id] : []));
    expect(starts).toEqual(turnIds(all));
    expect(starts.slice(-4)).toEqual(['u148', 'u149', 'ug', 'ul']);
    expect(starts).toHaveLength(152);
    // A page that begins at the steer: its own first entry is unknown, the steer is no start.
    const from = all.findIndex((e) => e.id === 'steer');
    expect(oldestTurnStart(all.slice(from), false)).toBe(all.slice(from).findIndex((e) => e.id === 'ul'));
  });
});

describe('chunks: assembleChunk', () => {
  it('a giant turn spread over many host pages comes whole, in one commit, cut at its own start', async () => {
    const all = owner();
    const { fetch, asked } = host(all);
    const tailFrom = all.findIndex((e) => e.id === 'ul');
    const known = new Set(all.slice(tailFrom).map((e) => e.id));
    const chunk = await assembleChunk('ul', fetch, (id) => known.has(id));
    // Pages: [steer.. ul) aligned at the steer (no known start), then older pages until the
    // giant turn's user message has its previous message in the buffer.
    expect(asked.length).toBeGreaterThan(2);
    expect(chunk.partial).toBe(false);
    expect(chunk.hasMore).toBe(true);
    expect(chunk.cursor).toBe(chunk.entries[0]!.id);
    // Whole turns only: the giant turn is ONE unit (not split at the steer), no preamble, and
    // the chunk begins at a turn start (here a few small turns ride along from the last page).
    const ids = turnIds([...chunk.entries, ...all.slice(tailFrom)]);
    expect(ids[0]).toBe(chunk.entries[0]!.id);
    expect(ids.slice(-2)).toEqual(['ug', 'ul']);
    expect(ids.some((id) => id.startsWith('pre:') || id === 'steer')).toBe(false);
  });

  it('reads back to the branch start: everything older commits, hasMore ends', async () => {
    const all = owner();
    const { fetch } = host(all);
    const ug = all.findIndex((e) => e.id === 'ug');
    const known = new Set(all.slice(ug).map((e) => e.id));
    const chunk = await assembleChunk('ug', fetch, (id) => known.has(id));
    expect(chunk.hasMore).toBe(true);
    // Several chunks of small turns, then the branch start.
    let chunk2 = chunk;
    const loaded = new Set(known);
    for (let i = 0; i < 10 && chunk2.hasMore; i++) {
      for (const e of chunk2.entries) loaded.add(e.id);
      expect(isTurnStart(chunk2.entries, 0, !chunk2.hasMore) || chunk2.entries[0]!.id === chunk2.cursor).toBe(true);
      chunk2 = await assembleChunk(chunk2.cursor!, fetch, (id) => loaded.has(id));
    }
    expect(chunk2.hasMore).toBe(false);
    expect(chunk2.entries[0]!.id).toBe('u0');
  });

  it('one turn longer than the cap commits as a flagged partial chunk (bounded)', async () => {
    const list: SessionEntry[] = [user('u0'), assistant('a0'), user('big')];
    for (let i = 0; i < CHUNK_MAX_ENTRIES + 300; i++) list.push(step(`x${i}`));
    list.push(user('last'));
    const all = chain(list);
    const { fetch, asked } = host(all);
    const chunk = await assembleChunk('last', fetch, (id) => id === 'last');
    expect(chunk.partial).toBe(true);
    expect(asked.length).toBeLessThanOrEqual(10);
    expect(chunk.entries.length).toBeLessThanOrEqual(CHUNK_MAX_ENTRIES);
    expect(chunk.cursor).toBe(chunk.entries[0]!.id);
  });
});
