import { describe, expect, it } from 'vitest';
import { buildTurns, createTurnsIndex, foldDetail, groupWork, toolGroupOf, type ToolRun, type TurnsInput, type WorkItem } from './turns';
import type { AgentMessage, SessionEntry } from './types';

const T0 = Date.UTC(2026, 9, 30, 9, 0, 0);

/** A tiny session builder: chained ids e1, e2, … one second apart. */
function session() {
  const entries: SessionEntry[] = [];
  let t = T0;
  const add = (e: Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>, dt = 1000): SessionEntry => {
    t += dt;
    const entry = { ...e, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry;
    entries.push(entry);
    return entry;
  };
  const msg = (message: AgentMessage, dt?: number) => add({ type: 'message', message: { timestamp: t + (dt ?? 1000), ...message } }, dt);
  const api = {
    entries,
    user: (text: string, extra: Record<string, unknown> = {}) => msg({ role: 'user', content: [{ type: 'text', text }], ...extra }),
    tools: (calls: Array<[name: string, args: Record<string, unknown>]>, opts: { thinking?: string; stop?: string; cost?: number } = {}) => {
      const content = [...(opts.thinking ? [{ type: 'thinking', thinking: opts.thinking }] : []), ...calls.map(([name, args], i) => ({ type: 'toolCall', id: `c${entries.length}_${i}`, name, arguments: args }))];
      const a = msg({ role: 'assistant', content, stopReason: opts.stop ?? 'toolUse', model: 'claude-sonnet-4-5', usage: { cost: { total: opts.cost ?? 0.01 } } });
      return { a, ids: content.filter((p) => p.type === 'toolCall').map((p) => (p as { id: string }).id) };
    },
    result: (id: string, name: string, isError = false) => msg({ role: 'toolResult', toolCallId: id, toolName: name, content: [{ type: 'text', text: `${name} ok` }], isError }),
    answer: (text: string, stop = 'stop', extra: Record<string, unknown> = {}) => msg({ role: 'assistant', content: text ? [{ type: 'text', text }] : [], stopReason: stop, model: 'claude-sonnet-4-5', usage: { cost: { total: 0.02 } }, ...extra }, 3000),
    add,
    /** One tool round: call(s) + result(s). */
    round(calls: Array<[string, Record<string, unknown>]>, thinking?: string) {
      const { ids } = api.tools(calls, { thinking });
      ids.forEach((id, i) => api.result(id, calls[i]![0]));
    },
  };
  return api;
}

const input = (entries: SessionEntry[], over: Partial<TurnsInput> = {}): TurnsInput => ({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: { running: false }, ...over });

describe('turns: boundaries', () => {
  it('entries before the first user message form a preamble; each user message opens a turn', () => {
    const s = session();
    s.add({ type: 'model_change', provider: 'anthropic', modelId: 'm' });
    const u1 = s.user('one');
    s.answer('done one');
    const u2 = s.user('two');
    s.answer('done two');
    const turns = buildTurns(input(s.entries));
    expect(turns.map((t) => [t.kind, t.turnId])).toEqual([
      ['preamble', 'pre:e1'],
      ['turn', u1.id],
      ['turn', u2.id],
    ]);
    expect(turns[1]!.user).toBe(u1);
    expect(turns[1]!.segments[0]!.answer?.text).toBe('done one');
  });

  it('a steer (user message after a tool result) opens a new segment in the same turn', () => {
    const s = session();
    const u = s.user('fix it');
    s.round([['read', { path: 'a.ts' }]]);
    const steer = s.user('also keep the API');
    s.round([['edit', { path: 'a.ts', oldText: 'a', newText: 'b' }]]);
    s.answer('done');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.turnId).toBe(u.id);
    expect(turn!.segments.map((g) => g.id)).toEqual([u.id, steer.id]);
    expect(turn!.segments[1]!.steer).toBe(steer);
    expect(turn!.segments[0]!.fold?.detail).toBe('read 1 file');
    expect(turn!.segments[1]!.fold?.detail).toBe('edited 1 file');
    expect(turn!.segments[1]!.answer?.text).toBe('done');
    expect(turn!.segments[0]!.answer).toBeUndefined();
  });

  it('a follow-up after a finished answer is a new turn, not a steer', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    s.user('two');
    expect(buildTurns(input(s.entries))).toHaveLength(2);
  });
});

describe('turns: fold and tail data', () => {
  it('a settled segment folds as "Worked for …" with unipi summary words; tail has duration, cost, model', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }], ['read', { path: 'b' }]], 'look first');
    s.round([['edit', { path: 'a', oldText: 'x', newText: 'y' }]]);
    s.answer('ok');
    const [turn] = buildTurns(input(s.entries));
    const fold = turn!.segments[0]!.fold!;
    expect(fold.state).toBe('done');
    expect(fold.forcedOpen).toBe(false);
    expect(fold.label).toMatch(/^Worked for \d+s$/);
    expect(fold.detail).toBe('read 2 files, edited 1');
    expect(turn!.durationMs).toBeGreaterThan(0);
    expect(turn!.costUsd).toBeCloseTo(0.04);
    expect(turn!.model).toBe('claude-sonnet-4-5');
    expect(turn!.stopped).toBe(false);
  });

  it('forced open while running ("Working"), on stop ("Stopped", no duration) and on error', () => {
    const s = session();
    s.user('go');
    const { ids } = s.tools([['bash', { command: 'make' }]]);
    const running = buildTurns(input(s.entries, { run: { running: true, startedAt: T0 } }));
    expect(running[0]!.live).toBe(true);
    expect(running[0]!.segments[0]!.fold).toMatchObject({ state: 'running', label: 'Working', forcedOpen: true });
    expect(running[0]!.segments[0]!.work[0]).toMatchObject({ kind: 'tool', tool: { state: 'running' } });

    s.result(ids[0]!, 'bash');
    s.answer('', 'aborted');
    const stopped = buildTurns(input(s.entries))[0]!;
    expect(stopped.stopped).toBe(true);
    expect(stopped.segments[0]!.fold).toMatchObject({ state: 'stopped', label: 'Stopped', forcedOpen: true });
    expect(stopped.segments[0]!.fold!.durationMs).toBeUndefined();

    const e = session();
    e.user('go');
    e.round([['bash', { command: 'make' }]]);
    e.answer('', 'error', { errorMessage: 'overloaded_error' });
    const failed = buildTurns(input(e.entries))[0]!;
    expect(failed.error).toBe('overloaded_error');
    expect(failed.segments[0]!.fold).toMatchObject({ state: 'error', forcedOpen: true });
    expect(failed.segments[0]!.work.at(-1)).toMatchObject({ kind: 'error', text: 'overloaded_error' });
  });

  it('tool calls without results in a settled turn read "stopped"; streaming calls "preparing"', () => {
    const s = session();
    s.user('go');
    s.tools([['bash', { command: 'sleep 9' }]]);
    expect(buildTurns(input(s.entries))[0]!.segments[0]!.work[0]).toMatchObject({ tool: { state: 'stopped' } });
    const live = buildTurns(input(s.entries, { run: { running: true }, streaming: { startedAt: T0, message: { role: 'assistant', content: [{ type: 'toolCall', id: 'z', name: 'read', arguments: {} }] } } }))[0]!;
    expect(live.segments[0]!.work.at(-1)).toMatchObject({ tool: { id: 'z', state: 'preparing' } });
  });

  it('streaming prose is the live answer; live tools without a call part still show', () => {
    const s = session();
    s.user('go');
    const turns = buildTurns(
      input(s.entries, {
        run: { running: true },
        streaming: { startedAt: T0, message: { role: 'assistant', content: [{ type: 'text', text: 'Partial' }] } },
        liveTools: { t9: { id: 't9', name: 'bash', args: { command: 'ls' }, startedAt: T0, partial: 'a\nb' } },
      }),
    );
    const seg = turns[0]!.segments[0]!;
    expect(seg.work.at(-1)).toMatchObject({ kind: 'tool', tool: { id: 't9', state: 'running', live: 'a\nb' } });
    expect(seg.answer).toBeUndefined(); // the tool came after the prose
  });
});

describe('turns: compaction and commands', () => {
  it('a compaction between turns is a marker unit; a mid-run one is a work item', () => {
    const s = session();
    s.user('one');
    s.answer('a');
    const c = s.add({ type: 'compaction', summary: 'earlier', tokensBefore: 48000 });
    s.user('two');
    s.round([['read', { path: 'x' }]]);
    const mid = s.add({ type: 'compaction', summary: 'overflow', tokensBefore: 90000 });
    s.answer('b');
    const turns = buildTurns(input(s.entries));
    expect(turns.map((t) => t.kind)).toEqual(['turn', 'compaction', 'turn']);
    expect(turns[1]!.turnId).toBe(`cmp:${c.id}`);
    expect(turns[1]!.compaction).toBe(c);
    expect(turns[2]!.segments[0]!.work.some((w) => w.kind === 'compaction' && w.entry === mid)).toBe(true);
  });

  it('a /compact echo is flagged hidden (Z-35); a command echo entry before a user message becomes its command', () => {
    const s = session();
    s.user('/compact keep the API notes');
    s.add({ type: 'compaction', summary: 's', tokensBefore: 1 });
    s.add({ type: 'custom', customType: 'oino.command-echo', data: { name: 'review', args: 'src/' } });
    const u = s.user('Review src/ for bugs');
    s.answer('ok');
    const turns = buildTurns(input(s.entries));
    expect(turns[0]!.command).toMatchObject({ name: 'compact', args: 'keep the API notes', hidden: true });
    expect(turns[1]!.kind).toBe('compaction');
    expect(turns[1]!.segments[0]!.work).toEqual([]); // the echo moved to the next turn
    expect(turns[2]!.turnId).toBe(u.id);
    expect(turns[2]!.command).toMatchObject({ name: 'review', args: 'src/', hidden: false });
  });

  it('cron and harness sources', () => {
    const s = session();
    s.user('nightly', { source: 'cron' });
    s.answer('ok');
    s.user('injected', { unipiHarness: { source: 'kanboard' } });
    expect(buildTurns(input(s.entries)).map((t) => t.source)).toEqual(['cron', 'harness']);
  });
});

describe('turns: tool groups (Z-38)', () => {
  const tool = (name: string, k: number): WorkItem => ({ kind: 'tool', key: `k${k}`, tool: { id: `t${k}`, name, args: {}, state: 'ok' } as ToolRun });
  it('classifies tools', () => {
    expect(['bash', 'read', 'grep', 'find', 'edit', 'write', 'web_search'].map(toolGroupOf)).toEqual(['execute', 'explore', 'explore', 'explore', 'changes', 'changes', undefined]);
  });
  it('merges consecutive same-class tools (≥ 2), thinking between them joins the group', () => {
    const think: WorkItem = { kind: 'thinking', key: 'th', text: 'hm', live: false };
    const nodes = groupWork([tool('bash', 1), think, tool('bash', 2), tool('bash', 3), tool('read', 4), tool('grep', 5), tool('edit', 6), tool('web_search', 7)]);
    expect(nodes.map((n) => (n.kind === 'group' ? `${n.group}:${n.tools.length}:${n.items.length}` : n.kind))).toEqual(['execute:3:4', 'explore:2:2', 'tool', 'tool']);
  });
  it('a lone tool and trailing thinking stay as they are', () => {
    const think: WorkItem = { kind: 'thinking', key: 'th', text: 'hm', live: false };
    expect(groupWork([tool('bash', 1), think]).map((n) => n.kind)).toEqual(['tool', 'thinking']);
  });
  it('fold detail wording', () => {
    const t = (name: string, state: ToolRun['state'] = 'ok'): ToolRun => ({ id: name, name, args: {}, state });
    expect(foldDetail([t('read'), t('read'), t('read'), t('read'), t('edit'), t('edit')])).toBe('read 4 files, edited 2');
    expect(foldDetail([t('bash', 'error'), t('read'), t('grep'), t('edit'), t('web_search')])).toBe('ran 1 command, read 1, searched 1, … · 1 failed');
    expect(foldDetail([], 2)).toBe('2 delegated steps');
  });
});

describe('turns index: incremental tail recompute', () => {
  function big(n: number) {
    const s = session();
    for (let i = 0; i < n; i++) {
      s.user(`q${i}`);
      s.round([['read', { path: `f${i}` }]]);
      s.answer(`a${i}`);
    }
    return s;
  }

  it('appends and live changes rebuild only the tail; settled turns keep identity', () => {
    const s = big(5);
    const index = createTurnsIndex();
    const first = index.update(input(s.entries));
    expect(index.lastMode).toBe('full');
    s.user('q5');
    const second = index.update(input(s.entries));
    expect(index.lastMode).toBe('tail');
    expect(second).toHaveLength(6);
    for (let i = 0; i < 5; i++) expect(second[i]).toBe(first[i]);
    // A streaming change: only the tail turn is new.
    const third = index.update(input(s.entries, { run: { running: true }, streaming: { startedAt: T0, message: { role: 'assistant', content: [{ type: 'text', text: 'x' }] } } }));
    expect(index.lastMode).toBe('tail');
    for (let i = 0; i < 5; i++) expect(third[i]).toBe(first[i]);
    expect(third[5]).not.toBe(second[5]);
    expect(third[5]!.segments[0]!.answer).toMatchObject({ text: 'x', live: true });
    expect(third).toEqual(buildTurns(input(s.entries, { run: { running: true }, streaming: { startedAt: T0, message: { role: 'assistant', content: [{ type: 'text', text: 'x' }] } } })));
  });

  it('a rev change (reset / reconcile) rebuilds fully but reuses unchanged turns', () => {
    const s = big(4);
    const index = createTurnsIndex();
    const first = index.update(input(s.entries));
    const replaced = s.entries.slice(0, -1).concat([{ ...s.entries.at(-1)!, id: 'real' }]);
    const next = index.update(input(replaced, { entriesRev: 2 }));
    expect(index.lastMode).toBe('full');
    for (let i = 0; i < 3; i++) expect(next[i]).toBe(first[i]);
    expect(next[3]).not.toBe(first[3]);
  });

  it('the steer case stays correct incrementally', () => {
    const s = session();
    s.user('go');
    const index = createTurnsIndex();
    s.round([['read', { path: 'a' }]]);
    index.update(input(s.entries, { run: { running: true } }));
    s.user('steer!');
    s.answer('done');
    const turns = index.update(input(s.entries));
    expect(index.lastMode).toBe('tail');
    expect(turns).toHaveLength(1);
    expect(turns[0]!.segments).toHaveLength(2);
    expect(turns).toEqual(buildTurns(input(s.entries)));
  });

  it('[owner-fix-2] one long streamed answer counts to when it ended (its entry), not when it started', () => {
    const s = session();
    s.user('marathon');
    const a = s.answer('tick 0 s. … Marathon stream finished.');
    // the message started streaming 50 ms after the prompt; the entry was appended 330 s later
    a.message!.timestamp = Date.parse(s.entries[0]!.timestamp!) + 50;
    a.timestamp = new Date(Date.parse(s.entries[0]!.timestamp!) + 330_000).toISOString();
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.durationMs).toBe(330_000);
    // a copied entry with a far later time does not inflate it
    a.timestamp = new Date(Date.parse(s.entries[0]!.timestamp!) + 86_400_000 * 3).toISOString();
    expect(buildTurns(input(s.entries))[0]!.durationMs).toBe(50);
  });

  it('trailing custom / wake-up entries never hide the assistant answer or inflate turn duration', () => {
    const s = session();
    s.user('click the button');
    s.round([['read', { path: 'SKILL.md' }]]);
    s.answer('After clicking, it changed to clicked 1');
    // Appended days later on session resume or auto-rename:
    s.add({ type: 'custom', customType: 'unipi:auto-name', data: { name: 'Button Click' } }, 86_400_000 * 4);
    s.add({ type: 'custom', customType: 'unipi-memory-session-card', data: {} }, 1000);
    s.add({ type: 'message', message: { role: 'system', content: '', timestamp: Date.now() } }, 86_400_000);
    s.add({ type: 'custom_message', customType: 'unipi:plan-mode-message', display: false } as unknown as Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>, 1000);
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments[0]!.answer?.text).toBe('After clicking, it changed to clicked 1');
    expect(turn!.durationMs).toBeLessThan(60_000);
    expect(turn!.segments[0]!.fold?.durationMs).toBeLessThan(60_000);
    const tool = turn!.segments[0]!.work.find((w) => w.kind === 'tool');
    expect(tool && tool.kind === 'tool' ? tool.tool.durationMs : undefined).toBeGreaterThan(0);
  });
});
