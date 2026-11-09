import { describe, expect, it } from 'vitest';
import type { SessionEventDto, SessionSnapshot } from '../host/client';
import { applyEvent, applyEvents, applySnapshot, branchOf, initialState, reconcileEntries, reconcileSince } from './reducer';
import { enqueue, markAcked } from './outbox';
import roundtrip from './fixtures/roundtrip.json';
import h4Fixtures from '../../../protocol/fixtures/session.json';
import type { SessionEntry, SessionState } from './types';

type Fixture = { name: string; schema: string; value: unknown; ok: boolean };
/** H4's contract fixtures (`apps/unipi/protocol/fixtures/session.json`). */
const h4 = h4Fixtures as unknown as Fixture[];
const h4Value = <T,>(name: string) => structuredClone(h4.find((f) => f.name === name)!.value) as T;

const rt = structuredClone(roundtrip) as unknown as {
  snapshot: SessionSnapshot;
  resume: SessionSnapshot;
  live: SessionEventDto[];
  getEntries: { since: string; reply: { entries: SessionEntry[]; leafId: string } };
  fresh: SessionSnapshot;
};

const NOW = 1_793_350_900_000;
const fresh = () => initialState('h1', 's1');

/** What must match a fresh snapshot: everything but the rebuild counter. */
function comparable(s: SessionState) {
  const { entriesRev: _rev, ...rest } = structuredClone(s);
  return rest;
}

/** Content-only view (provisional ids / parents / persist timestamps aside). */
function content(s: SessionState) {
  return s.entries.map(({ id: _id, parentId: _p, timestamp: _t, ...rest }) => rest);
}

function fromFresh(): SessionState {
  const s = fresh();
  applySnapshot(s, structuredClone(rt.fresh), NOW);
  return s;
}

describe('worker approval synchronization', () => {
  it('clears the matching card when answered on another surface', () => {
    const s = fresh();
    const approval = { type: 'approval_request', requestId: 'r1', sessionId: 'child', tool: 'bash', arguments: { command: 'git push' }, riskLevel: 'high', ceiling: 'medium', reason: 'needs approval' };
    applyEvent(s, { session: 's1', seq: 1, event: approval });
    applyEvent(s, { session: 's1', seq: 2, event: { ...approval, requestId: 'r2' } });
    applyEvent(s, { session: 's1', seq: 3, event: { type: 'approval_cleared', requestId: 'r1' } });
    expect(s.approvals?.map((r) => r.requestId)).toEqual(['r2']);
  });
});

describe('session reducer: H4 contract fixtures', () => {
  it('full snapshot seeds entries, leaf, commands, topics, run and the ui hand-off', () => {
    const s = fresh();
    const r = applySnapshot(s, h4Value<SessionSnapshot>('full snapshot'), NOW);
    expect(s.seq).toBe(42);
    expect(s.entries.map((e) => e.id)).toEqual(['e1']);
    expect(s.byId).toEqual({ e1: 0 });
    expect(s.leafId).toBe('e1');
    expect(s.commands.map((c) => c.name)).toEqual(['compact']);
    expect(s.topics.plan).toEqual({ data: { items: [] }, sticky: true, publisher: 'oino.core' });
    expect(s.run).toEqual({ phase: 'idle', running: false });
    expect(s.thinkingLevel).toBe('off');
    expect(s.conn).toBe('live');
    expect(r.ui?.[0]).toMatchObject({ type: 'oino_ui_snapshot', rev: 7 });
  });

  it('resume patch replays in order: agent_end, a delta, then the host crashed banner', () => {
    const s = fresh();
    applySnapshot(s, h4Value<SessionSnapshot>('full snapshot'), NOW);
    const r = applySnapshot(s, h4Value<SessionSnapshot>('resume patch'), NOW);
    expect(r.reconcile).toBe(true);
    expect(s.seq).toBe(45);
    expect(s.banner).toMatchObject({ kind: 'crashed', text: 'The session crashed: tool body exploded' });
    expect(s.streaming).toBeUndefined(); // the crash ended the run
    expect(s.run.running).toBe(false);
  });

  it('an events frame below the current seq is a duplicate and changes nothing', () => {
    const s = fresh();
    applySnapshot(s, h4Value<SessionSnapshot>('full snapshot'), NOW);
    const before = structuredClone(s);
    const frame = h4Value<{ items: SessionEventDto[] }>('events frame');
    expect(applyEvents(s, frame.items, NOW)).toEqual({});
    expect(s).toEqual(before);
  });
});

describe('session reducer: snapshot → replay → live → message_end round trip', () => {
  function playAll(): SessionState {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    applySnapshot(s, structuredClone(rt.resume), NOW);
    applyEvents(s, structuredClone(rt.live), NOW);
    return s;
  }

  it('streams thinking, tool call args and text into `streaming`, then settles them as entries', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    applySnapshot(s, structuredClone(rt.resume), NOW);
    const live = structuredClone(rt.live);
    const upTo = (seq: number) => applyEvents(s, live.filter((d) => d.seq <= seq && d.seq > s.seq), NOW);
    upTo(21);
    expect(s.run.running).toBe(true);
    expect(s.streaming?.message.content).toEqual([{ type: 'thinking', thinking: 'Add a case for negative numbers.' }]);
    upTo(23);
    const call = (s.streaming?.message.content as Array<Record<string, unknown>>)[1]!;
    expect(call).toMatchObject({ type: 'toolCall', id: 'call_2', name: 'edit', partialJson: '{"path":"src/math.test.ts"' });
    upTo(28);
    expect(s.streaming).toBeUndefined();
    expect(s.liveTools.call_2).toMatchObject({ done: true, isError: false, durationMs: 41, partial: 'Editing…' });
    upTo(29);
    expect(s.liveTools.call_2).toBeUndefined(); // its toolResult entry carries it now
    upTo(34);
    expect(s.streaming?.message.content).toEqual([{ type: 'text', text: 'Added a regression test.' }]);
  });

  it('after the run the content equals a fresh snapshot; ids are provisional until reconciled', () => {
    const s = playAll();
    const want = fromFresh();
    expect(s.seq).toBe(37);
    expect(content(s)).toEqual(content(want));
    expect(s.entries.slice(-4).every((e) => e.id.startsWith('live:'))).toBe(true);
    expect(s.leafId).toBe('live:35');
  });

  it('reconcile with get_entries{since} makes the state deep-equal a fresh snapshot', () => {
    const s = playAll();
    const since = reconcileSince(s);
    expect(since).toEqual({ since: rt.getEntries.since, needed: true });
    const rev = s.entriesRev;
    reconcileEntries(s, since.since, structuredClone(rt.getEntries.reply.entries), rt.getEntries.reply.leafId);
    expect(s.entriesRev).toBe(rev + 1);
    expect(comparable(s)).toEqual(comparable(fromFresh()));
    expect(reconcileSince(s).needed).toBe(false);
  });

  it('agent_end asks for the reconcile', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    expect(applyEvent(s, { session: 's1', seq: 11, event: { type: 'agent_end', messages: [] } }, NOW).reconcile).toBe(true);
  });
});

describe('session reducer: reset, gaps, host events', () => {
  it('a reset snapshot rebuilds from entries, keeps unchanged entry objects, clears live state', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    const u1 = s.entries[1];
    applySnapshot(s, structuredClone(rt.resume), NOW);
    applyEvents(s, structuredClone(rt.live).slice(0, 8), NOW); // mid-run: streaming + provisional user entry
    expect(s.streaming).toBeDefined();
    const rev = s.entriesRev;
    applySnapshot(s, structuredClone(rt.fresh), NOW);
    expect(s.entriesRev).toBe(rev + 1);
    expect(s.entries[1]).toBe(u1); // same object: settled turns keep identity
    expect(s.streaming).toBeUndefined();
    expect(s.liveTools).toEqual({});
    expect(s.entries.some((e) => e.id.startsWith('live:'))).toBe(false);
    expect(comparable(s)).toEqual(comparable(fromFresh()));
  });

  it('restores queued steering and follow-up messages from reset snapshots and woken state (OIN-75)', () => {
    const s = fresh();
    const pending = enqueue(s, { text: 'check edge cases', mode: 'steer' }, true, NOW);
    markAcked(s, pending.ref, NOW);
    const snap = structuredClone(rt.fresh);
    snap.state = {
      ...((snap.state ?? {}) as Record<string, unknown>),
      isStreaming: true,
      pendingMessageCount: 2,
      steering: ['check edge cases'],
      followUp: ['run tests after'],
    };
    applySnapshot(s, snap, NOW);
    expect(s.queue).toEqual({ steering: ['check edge cases'], followUp: ['run tests after'] });
    expect(s.outbox).toEqual([]);

    // Empty host queue clears stale queue state on woken too.
    applyEvent(
      s,
      { session: 's1', seq: snap.seq + 1, event: { kind: 'woken', state: { sessionId: 's1', isStreaming: false, isCompacting: false, pendingMessageCount: 0 } } },
      NOW,
    );
    expect(s.queue).toEqual({ steering: [], followUp: [] });
  });

  it('a seq gap that coalesced deltas cannot explain is reported and not applied', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    const r = applyEvent(s, { session: 's1', seq: 14, event: { type: 'queue_update', steering: ['x'], followUp: [] } }, NOW);
    expect(r.gap).toBe(true);
    expect(s.seq).toBe(10);
    expect(s.queue.steering).toEqual([]);
    // A batch stops at the gap: later items wait for the reopen.
    const b = applyEvents(s, [
      { session: 's1', seq: 11, event: { type: 'agent_start' } },
      { session: 's1', seq: 13, event: { type: 'agent_end' } },
      { session: 's1', seq: 14, event: { type: 'agent_start' } },
    ], NOW);
    expect(b.gap).toBe(true);
    expect(s.seq).toBe(11);
  });

  it('a gap before a text/thinking delta (host coalescing) is benign', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    const r = applyEvent(s, { session: 's1', seq: 15, event: { type: 'message_update', message: { role: 'assistant' }, assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: 'hi' } } }, NOW);
    expect(r.gap).toBeUndefined();
    expect(s.seq).toBe(15);
    expect(s.streaming?.message.content).toEqual([{ type: 'text', text: 'hi' }]);
  });

  it('taken_over sets the banner, ends the run and tells the store to stop reconnecting', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    applyEvent(s, { session: 's1', seq: 11, event: { type: 'agent_start' } }, NOW);
    const r = applyEvent(s, { session: 's1', seq: 12, event: { kind: 'taken_over', from: 'tui' } }, NOW);
    expect(r.takenOver).toBe(true);
    expect(s.banner).toMatchObject({ kind: 'taken_over', text: 'Taken over from the tui.', from: 'tui' });
    expect(s.run.running).toBe(false);
    applyEvent(s, { session: 's1', seq: 13, event: { kind: 'reloaded' } }, NOW);
    expect(s.banner?.kind).toBe('reloaded');
  });

  it('dialogs, topics, model, thinking, name, retry and ui traffic', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    const r = applyEvents(s, [
      { session: 's1', seq: 11, event: { type: 'extension_ui_request', id: 'd1', method: 'confirm', title: 'Delete?' } },
      { session: 's1', seq: 12, event: { type: 'extension_ui_request', id: 'n1', method: 'notify', message: 'hi' } },
      { session: 's1', seq: 13, event: { type: 'topic', topic: 'todos', data: [1], sticky: true, publisher: 'x' } },
      { session: 's1', seq: 14, event: { type: 'model_changed', model: { provider: 'openai', id: 'gpt-5' }, source: 'set' } },
      { session: 's1', seq: 15, event: { type: 'thinking_level_changed', level: 'high' } },
      { session: 's1', seq: 16, event: { type: 'session_info_changed', name: 'Parser' } },
      { session: 's1', seq: 17, event: { type: 'auto_retry_start', attempt: 1, maxAttempts: 3, delayMs: 2000, errorMessage: 'overloaded' } },
      { session: 's1', seq: 18, event: { type: 'oino_ui_patch', scope: 'session', rev: 4, ops: [] } },
      { session: 's1', seq: 19, event: { type: 'commands_changed' } },
    ], NOW);
    expect(s.dialogs).toEqual(['d1']);
    expect(s.topics.todos?.data).toEqual([1]);
    expect(s.model).toMatchObject({ id: 'gpt-5' });
    expect(s.thinkingLevel).toBe('high');
    expect(s.name).toBe('Parser');
    expect(s.run).toMatchObject({ phase: 'retrying', retry: { attempt: 1, error: 'overloaded' } });
    expect(r.ui?.map((u) => u.type)).toEqual(['extension_ui_request', 'extension_ui_request', 'oino_ui_patch']);
    expect(r.commandsChanged).toBe(true);
    applyEvent(s, { session: 's1', seq: 20, event: { type: 'extension_ui_dismiss', id: 'd1', reason: 'answered' } }, NOW);
    expect(s.dialogs).toEqual([]);
    expect(applyEvent(s, { session: 's1', seq: 21, event: { type: 'lagged', dropped: 3 } }, NOW).resync).toBe(true);
  });

  it('a user message_start settles its acked outbox bubble; entry_appended entries are confirmed', () => {
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    applySnapshot(s, structuredClone(rt.resume), NOW);
    const item = enqueue(s, { text: 'Now add a regression test' }, true, NOW);
    markAcked(s, item.ref, NOW);
    applyEvents(s, structuredClone(rt.live).slice(0, 3), NOW);
    expect(s.outbox).toEqual([]);
    applyEvent(s, { session: 's1', seq: 16, event: { type: 'entry_appended', entry: { type: 'custom', id: 'c9', parentId: 'live:14', customType: 'oino.command-echo', data: { name: 'x', args: '' } } } }, NOW);
    expect(s.leafId).toBe('c9');
    expect(reconcileSince(s)).toEqual({ since: 'a2', needed: true });
  });
});

describe('branchOf', () => {
  const e = (id: string, parentId: string | null): SessionEntry => ({ type: 'message', id, parentId });
  it('keeps every entry when nothing forks, else the chain from the leaf', () => {
    const linear = [e('a', null), e('b', 'a'), e('c', 'b')];
    expect(branchOf(linear, 'c')).toBe(linear);
    const forked = [e('a', null), e('b', 'a'), e('c', 'b'), e('x', 'b'), e('y', 'x')];
    expect(branchOf(forked, 'y').map((x) => x.id)).toEqual(['a', 'b', 'x', 'y']);
    expect(branchOf(forked, 'c').map((x) => x.id)).toEqual(['a', 'b', 'c']);
  });

  it('bridges across unannounced intermediate parents (Fusion switch / compaction) instead of wiping history', () => {
    // Forked chat where model_change ('m1') + thinking_level_change ('t1') were persisted without
    // entry_appended before unipi:fusion-sidekick ('fs1', parentId: 't1') and a new system/user entry arrived.
    const forked = [
      e('a', null),
      e('b', 'a'),
      e('alt', 'a'),
      e('c', 'b'),
      e('fs1', 't1'),
      e('live:40', 'fs1'),
    ];
    expect(branchOf(forked, 'live:40').map((x) => x.id)).toEqual(['a', 'b', 'c', 'fs1', 'live:40']);

    // And applyEvent asks catchUp from the last connected confirmed entry on both entry_appended gap and compaction_end.
    const s = fresh();
    applySnapshot(s, structuredClone(rt.snapshot), NOW);
    const lastId = s.entries.at(-1)!.id;
    const r1 = applyEvent(
      s,
      { session: 's1', seq: 11, event: { type: 'entry_appended', entry: { type: 'custom', id: 'fs1', parentId: 'missing_t1', customType: 'unipi:fusion-sidekick', data: { active: true } } } },
      NOW,
    );
    expect(r1.catchUp).toBe(lastId);
    const r2 = applyEvent(
      s,
      { session: 's1', seq: 12, event: { type: 'compaction_end', aborted: false, result: { summary: 'Compacted summary', tokensBefore: 50000 } } },
      NOW,
    );
    expect(r2.catchUp).toBe(lastId);
  });
});
