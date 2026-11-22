/**
 * OIN-19 — three chat-transcript bugs:
 *  1. a summarizer prompt that went out as a chat message renders as a compaction card, never
 *     as a user bubble carrying the prompt verbatim (turn head and steer alike);
 *  2. the render cutoff divider marks exactly one turn, and a reload / incremental update never
 *     duplicates it (the empty gray pill is covered in ui/slots/slots.test.tsx);
 *  3. a long-horizon decision that repeats the branch's current mode is not a "Mode" card — one
 *     card per change, the same on a full rebuild as on an incremental tail update.
 */
import { describe, expect, it } from 'vitest';
import { buildTurns, createTurnsIndex, type Turn, type TurnsInput, type WorkNode } from './turns';
import { describeEvent } from '../timeline/eventKinds';
import type { AgentMessage, SessionEntry } from './types';

const T0 = Date.UTC(2026, 10, 7, 9, 0, 0);
const SUMMARIZER =
  'You are a context summarization assistant. Your task is to read a conversation between a user and an AI coding assistant, then produce a structured summary.';

function session() {
  const entries: SessionEntry[] = [];
  let t = T0;
  const add = (e: Record<string, unknown>): SessionEntry => {
    t += 1000;
    const entry = { ...e, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry;
    entries.push(entry);
    return entry;
  };
  const msg = (message: AgentMessage) => add({ type: 'message', message: { timestamp: t, ...message } });
  return {
    entries,
    add,
    user: (text: string) => msg({ role: 'user', content: [{ type: 'text', text }] }),
    answer: (text: string, stop = 'stop') => msg({ role: 'assistant', content: text ? [{ type: 'text', text }] : [], stopReason: stop, model: 'm' }),
    call: (name: string) => {
      const a = msg({ role: 'assistant', content: [{ type: 'toolCall', id: `c${entries.length}`, name, arguments: {} }], stopReason: 'toolUse', model: 'm' });
      const id = `c${entries.length - 1}`;
      msg({ role: 'toolResult', toolCallId: id, toolName: name, content: [{ type: 'text', text: 'ok' }] });
      return a;
    },
    mode: (mode: string, source = 'sticky') => add({ type: 'custom', customType: 'long-horizon-decision', data: { mode, source, reason: '' } }),
  };
}

const input = (entries: SessionEntry[], rev = 1): TurnsInput => ({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: rev, liveTools: {}, run: { running: false } });

const cards = (turn: Turn): WorkNode[] =>
  turn.segments.flatMap((s) => [...(s.before ?? []), ...s.nodes, ...(s.after ?? [])]).filter((n) => n.kind === 'card');
const modeCards = (turns: Turn[]) =>
  turns.flatMap(cards).filter((n) => n.kind === 'card' && n.entry.type === 'custom' && n.entry.customType === 'long-horizon-decision');
const steers = (turns: Turn[]) => turns.flatMap((t) => t.segments.filter((s) => s.steer));

describe('OIN-19 (1): the summarizer prompt never renders as a user bubble', () => {
  it('as a turn head: the bubble is hidden and a compaction card shows instead', () => {
    const s = session();
    s.user('hello');
    s.answer('hi');
    const req = s.user(SUMMARIZER);
    s.answer('## Goal\nship v3');
    const turns = buildTurns(input(s.entries));
    const turn = turns.find((t) => t.turnId === req.id)!;
    expect(turn.command).toMatchObject({ name: 'compact', hidden: true });
    const card = cards(turn).find((n) => n.kind === 'card' && n.entry.id === req.id);
    expect(card).toBeTruthy();
    expect(describeEvent(req)).toMatchObject({ kind: 'summary-request', source: 'Compaction', title: 'summary requested · /compact', audience: 'both' });
    // the reply still reads as the turn's answer
    expect(turn.segments[0]!.answer?.text).toContain('ship v3');
  });

  it('as a steer (after a tool result): no steer bubble, a card in its segment', () => {
    const s = session();
    s.user('run the graph');
    s.call('yield_agent_graph');
    const req = s.user(SUMMARIZER);
    s.answer('summary');
    const turns = buildTurns(input(s.entries));
    expect(steers(turns)).toHaveLength(0);
    expect(turns.flatMap(cards).some((n) => n.kind === 'card' && n.entry.id === req.id)).toBe(true);
  });

  it('ordinary user messages stay bubbles', () => {
    const s = session();
    s.user('Summarize what you are a context summarization assistant for');
    s.answer('ok');
    const turns = buildTurns(input(s.entries));
    expect(turns.at(-1)!.command).toBeUndefined();
    expect(describeEvent(s.entries[0]!)).toBeUndefined();
  });
});

describe('OIN-19 (2): one render cutoff divider, stable across reloads', () => {
  it('marks only the turn holding the anchor; a full rebuild and a tail update agree', () => {
    const s = session();
    s.user('one');
    s.answer('a1');
    const kept = s.user('two');
    s.answer('a2');
    s.add({ type: 'compaction_state', anchorMessage: { id: kept.id }, summaryText: 'sum', tokensBefore: 1000 });
    s.user('three');
    s.answer('a3');
    const count = (turns: Turn[]) => turns.filter((t) => t.renderCutoff).length;
    const full = buildTurns(input(s.entries));
    expect(count(full)).toBe(1);
    expect(full.find((t) => t.renderCutoff)!.turnId).toBe(kept.id);
    const index = createTurnsIndex();
    expect(count(index.update(input(s.entries, 1)))).toBe(1);
    s.user('four');
    s.answer('a4');
    expect(count(index.update(input(s.entries, 2)))).toBe(1);
    expect(count(buildTurns(input(s.entries)))).toBe(1);
  });
});

describe('OIN-19 (3): a repeated long-horizon mode is not a Mode card', () => {
  it('one card per change: sticky repeats and retry re-runs are folded away', () => {
    const s = session();
    s.mode('graph', 'owner');
    s.user('restarted. progress?');
    s.answer('15 in review');
    // the sticky mode re-applies on each prompt; retries re-run the gate
    s.mode('graph');
    s.user('next');
    s.mode('graph');
    s.mode('graph');
    s.answer('ok');
    s.mode('none', 'explicit');
    s.user('plain now');
    s.mode('none');
    s.answer('ok');
    const modes = (turns: Turn[]) => modeCards(turns).map((n) => (n.kind === 'card' ? String((n.entry.data as { mode: string }).mode) : ''));
    expect(modes(buildTurns(input(s.entries)))).toEqual(['graph', 'none']);
  });

  it('is idempotent across re-opens: an incremental tail update shows the same cards as a full build', () => {
    const s = session();
    s.mode('graph', 'owner');
    s.user('one');
    s.answer('a1');
    const index = createTurnsIndex();
    expect(modeCards(index.update(input(s.entries, 1)))).toHaveLength(1);
    s.mode('graph');
    s.user('two');
    s.answer('a2');
    s.mode('graph');
    const incremental = index.update(input(s.entries, 2));
    expect(modeCards(incremental)).toHaveLength(1);
    expect(modeCards(buildTurns(input(s.entries)))).toHaveLength(1);
  });
});
