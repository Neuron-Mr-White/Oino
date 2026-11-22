import { describe, expect, it } from 'vitest';
import { buildTurns, createTurnsIndex, foldDetail, foldErrors, groupWork, newestErrorKey, toolGroupOf, type Segment, type ToolRun, type TurnsInput, type WorkItem, type WorkNode } from './turns';
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

describe('turns: internal resume instruction (r7 P2-5)', () => {
  const RESUME = 'Your previous turn was interrupted (the host stopped or restarted while you were working); continue from where you stopped. Re-check the last tool results and the workspace before you repeat any outside action.';
  it('opening a turn, it shows no System card (the work after it still does)', () => {
    const s = session();
    s.user('build it');
    s.answer('half done', 'stop');
    s.user(RESUME, { unipiHarness: { source: 'Long-horizon', title: 'Continue' } });
    s.answer('resumed', 'stop');
    const turns = buildTurns(input(s.entries));
    expect(turns).toHaveLength(2);
    expect(turns[1]!.command?.hidden).toBe(true);
    expect(turns[0]!.command).toBeUndefined();
  });
  it('landing mid-turn, it is no steer bubble either', () => {
    const s = session();
    s.user('build it');
    s.round([['bash', { command: 'make' }]]);
    s.user(RESUME);
    s.answer('ok', 'stop');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments.some((seg) => seg.steer)).toBe(false);
  });
});

describe('turns: "Done in" skips the outage before an Interrupted marker (r9 P3-2)', () => {
  it('a host-down gap between the last step and the marker is not work', () => {
    const s = session();
    s.user('long job');
    s.round([['read', { path: 'a' }]]);
    s.add({ type: 'custom', customType: 'oino.turn-interrupted' } as never, 60_000);
    s.round([['read', { path: 'b' }]]);
    s.answer('done');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.durationMs).toBeLessThan(15_000);
    // The same turn without the outage keeps its plain span.
    const plain = session();
    plain.user('long job');
    plain.round([['read', { path: 'a' }]]);
    plain.round([['read', { path: 'b' }]]);
    plain.answer('done');
    const [p] = buildTurns(input(plain.entries));
    expect(p!.durationMs).toBeGreaterThan(0);
  });
});

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

  describe('a prompt after a run that ended on a tool result (perf r3 D1)', () => {
    const ended = (s: ReturnType<typeof session>, marker: 'agent-end' | 'interrupted') => {
      s.add({ type: 'custom', customType: marker === 'agent-end' ? 'oino.agent-end' : 'oino.turn-interrupted' } as never);
    };

    it('is a new turn when core recorded the run end (loop guard: notice + oino.agent-end)', () => {
      const s = session();
      const u1 = s.user('loop it');
      s.round([['read', { path: 'a' }]]);
      s.add({ type: 'custom_message', customType: 'oino.loop-notice', content: 'Oino stopped before executing three consecutive identical tool calls.', display: true } as never);
      ended(s, 'agent-end');
      const u2 = s.user('what now?');
      s.answer('ok');
      const turns = buildTurns(input(s.entries));
      expect(turns.map((t) => t.turnId)).toEqual([u1.id, u2.id]);
      expect(turns[1]!.segments.every((g) => !g.steer)).toBe(true);
      expect(turns[0]!.segments.every((g) => !g.steer)).toBe(true);
    });

    it('is a new turn after the loop guard notice alone (chats from before oino.agent-end)', () => {
      const s = session();
      s.user('loop it');
      s.round([['read', { path: 'a' }]]);
      s.add({ type: 'custom_message', customType: 'oino.loop-notice', content: 'Oino stopped before executing three consecutive identical tool calls.', display: true } as never);
      s.user('what now?');
      expect(buildTurns(input(s.entries))).toHaveLength(2);
    });

    it('is a new turn after an interrupted marker (the Continue / follow-up case)', () => {
      const s = session();
      const u1 = s.user('long job');
      s.round([['read', { path: 'a' }]]);
      ended(s, 'interrupted');
      const u2 = s.user('continue');
      s.round([['read', { path: 'b' }]]);
      s.answer('done');
      const turns = buildTurns(input(s.entries));
      expect(turns.map((t) => t.turnId)).toEqual([u1.id, u2.id]);
      expect(turns[1]!.segments[0]!.steer).toBeUndefined();
    });

    it('is a new turn when an old run was cut off with no marker at all and the prompt came much later (r4 D12)', () => {
      const s = session();
      const u1 = s.user('long job');
      s.round([['read', { path: 'a' }]]);
      s.add({ type: 'model_change', provider: 'anthropic', modelId: 'm' } as never, 2 * 3_600_000);
      const u2 = s.user('Do the steps.');
      s.round([['read', { path: 'b' }]]);
      s.answer('done');
      const turns = buildTurns(input(s.entries));
      expect(turns.map((t) => t.turnId)).toEqual([u1.id, u2.id]);
      expect(turns.every((t) => t.segments.every((g) => !g.steer))).toBe(true);
    });

    it('also holds when the run ended on an unanswered tool call', () => {
      const s = session();
      s.user('ask');
      s.tools([['ask_user', { q: 'x' }]]);
      ended(s, 'agent-end');
      s.user('never mind');
      expect(buildTurns(input(s.entries))).toHaveLength(2);
    });

    it('a real steer (no run end between) is still a steer, and a later prompt after the end is not', () => {
      const s = session();
      const u1 = s.user('fix it');
      s.round([['read', { path: 'a' }]]);
      const steer = s.user('also keep the API');
      s.round([['read', { path: 'b' }]]);
      s.answer('done');
      const u2 = s.user('next');
      const turns = buildTurns(input(s.entries));
      expect(turns.map((t) => t.turnId)).toEqual([u1.id, u2.id]);
      expect(turns[0]!.segments[1]!.steer).toBe(steer);
    });

    it('the incremental index agrees when the end marker lands after the prompt was already seen', () => {
      const s = session();
      s.user('go');
      s.round([['read', { path: 'a' }]]);
      ended(s, 'agent-end');
      const index = createTurnsIndex();
      index.update(input(s.entries, { entriesRev: 1 }));
      s.user('again');
      const turns = index.update(input(s.entries, { entriesRev: 2 }));
      expect(turns).toHaveLength(2);
      expect(turns.map((t) => t.turnId)).toEqual(buildTurns(input(s.entries)).map((t) => t.turnId));
    });

    it('no run-end row is drawn for the boundary entry', () => {
      const s = session();
      s.user('go');
      s.round([['read', { path: 'a' }]]);
      ended(s, 'agent-end');
      const [turn] = buildTurns(input(s.entries));
      expect(turn!.segments.flatMap((g) => g.work).filter((w) => w.kind === 'card')).toHaveLength(0);
    });
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

  it('core state custom entries (image-policy, sent-frontier) never become work cards', () => {
    const s = session();
    s.user('do work');
    s.round([['read', { path: 'a.ts' }]]);
    s.add({ type: 'custom', customType: 'oino.image-policy', data: {} });
    s.add({ type: 'custom', customType: 'oino.sent-frontier', data: {} });
    s.round([['bash', { command: 'echo ok' }]]);
    s.answer('done');
    const stateTypes = new Set(['oino.image-policy', 'oino.sent-frontier']);
    const cards = buildTurns(input(s.entries)).flatMap((t) =>
      t.segments.flatMap((g) => [...(g.before ?? []), ...g.work, ...(g.after ?? [])]),
    ).filter((w) => w.kind === 'card' && stateTypes.has(String(w.entry.customType)));
    expect(cards).toEqual([]);
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

describe('turns: replies woken after the answer', () => {
  it('a reply woken by a background notification opens its own segment; the earlier answer stays out of the fold', () => {
    const s = session();
    s.user('finish CLO-1');
    s.round([['bash', { command: 'cargo test' }]]);
    s.answer('I moved CLO-1 to in_review with the full report.');
    s.add({ type: 'custom', customType: 'unipi:bg-tasks', data: {} });
    s.add({ type: 'custom_message', customType: 'background-task-notification', content: '<background-task-notification>done</background-task-notification>', display: true } as unknown as Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>);
    s.answer('That notification is from my first test attempt.');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments.map((g) => g.answer?.text)).toEqual(['I moved CLO-1 to in_review with the full report.', 'That notification is from my first test attempt.']);
    expect(turn!.segments[0]!.work.some((w) => w.kind === 'prose')).toBe(false);
    expect(turn!.segments[0]!.fold?.state).toBe('done');
  });

  it('prose between tool rounds still folds; only a finished reply starts a new segment', () => {
    const s = session();
    s.user('go');
    s.add({ type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: 'Checking first.' }, { type: 'toolCall', id: 'k1', name: 'read', arguments: { path: 'a' } }], stopReason: 'toolUse', timestamp: 0 } } as unknown as Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>);
    s.result('k1', 'read');
    s.answer('Done.');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments).toHaveLength(1);
    expect(turn!.segments[0]!.answer?.text).toBe('Done.');
  });
});

describe('turns: repeated errors', () => {
  const err = (key: string, text: string): WorkItem => ({ kind: 'error', key, text });
  const RATE = 'Omniroute rate-limited the request (429). Try again shortly.';

  it('identical consecutive errors fold into one with a count, keyed by the newest', () => {
    const out = foldErrors([err('a', RATE), err('b', `  ${RATE}\n`), err('c', RATE), err('d', RATE)]);
    expect(out).toEqual([{ kind: 'error', key: 'd', text: RATE, count: 4 }]);
  });

  it('a lone error keeps no count; different messages do not fold', () => {
    expect(foldErrors([err('a', RATE)])).toEqual([err('a', RATE)]);
    expect(foldErrors([err('a', RATE), err('b', 'overloaded'), err('c', RATE)]).map((w) => (w.kind === 'error' ? w.count : 'x'))).toEqual([undefined, undefined, undefined]);
  });

  it('errors separated by a non-error item do not fold', () => {
    const think: WorkItem = { kind: 'thinking', key: 't', text: 'hm', live: false };
    const out = foldErrors([err('a', RATE), think, err('b', RATE)]);
    expect(out.map((w) => w.key)).toEqual(['a', 't', 'b']);
    expect(out.every((w) => w.kind !== 'error' || w.count === undefined)).toBe(true);
  });

  it('groupWork folds them into the segment nodes while work keeps every attempt', () => {
    const s = session();
    s.user('go');
    for (let i = 0; i < 4; i++) s.answer('', 'error', { errorMessage: '429: rate limited' });
    const [turn] = buildTurns(input(s.entries));
    const seg = turn!.segments[0]!;
    expect(seg.work.filter((w) => w.kind === 'error')).toHaveLength(4);
    const nodes = seg.nodes.filter((n) => n.kind === 'error');
    expect(nodes).toHaveLength(1);
    expect(nodes[0]).toMatchObject({ count: 4, key: seg.work.at(-1)!.key });
  });

  it('only the last error of the whole timeline is the newest', () => {
    const s = session();
    s.user('one');
    s.answer('', 'error', { errorMessage: 'first failure' });
    s.user('two');
    s.answer('', 'error', { errorMessage: 'second failure' });
    s.answer('', 'error', { errorMessage: 'third failure' });
    s.user('three');
    s.answer('fine');
    const turns = buildTurns(input(s.entries));
    const errs = turns.flatMap((t) => t.segments.flatMap((g) => g.work.filter((w) => w.kind === 'error')));
    expect(errs).toHaveLength(3);
    expect(newestErrorKey(turns)).toBe(errs[2]!.key);
    expect(newestErrorKey(turns.slice(0, 1))).toBe(errs[0]!.key);
    expect(newestErrorKey([])).toBeUndefined();
  });
});

it('collapses only consecutive identical attributed notifications, including on reload', async () => {
  const { collapseHarnessTurns } = await import('./turns');
  const s = session();
  s.user('Agent graph checkpoint: ready', { author: 'extension' });
  for (let i = 0; i < 3; i++) s.user('Loop warning', { author: 'extension' });
  s.user('Memory reminder', { author: 'extension' });
  s.user('Host wake', { author: 'extension' });
  s.user('do work');
  s.round([['read', {}]]);
  const steer = s.user('genuine steer');
  const render = (entries: SessionEntry[]) => collapseHarnessTurns(buildTurns(input(entries)));
  const turns = render(s.entries);
  expect(turns.map(t => t.harnessCount ?? 1)).toEqual([1, 3, 1, 1, 1]);
  expect(turns.at(-1)?.segments.at(-1)?.steer?.id).toBe(steer.id);
  expect(render(JSON.parse(JSON.stringify(s.entries)))).toEqual(turns);
});

describe('turns: consecutive folds merge into one (OIN-96)', () => {
  /** `rounds` wakes: each one tool round that ends on an empty `stop` message (no prose). */
  function wakes(rounds: number, failAt = -1) {
    const s = session();
    const u = s.user('run the swarm');
    for (let i = 0; i < rounds; i++) {
      const { ids } = s.tools([['bash', { command: `echo ${i}` }]]);
      s.result(ids[0]!, 'bash', i === failAt);
      s.answer('');
    }
    return { s, u };
  }

  it('N empty-prose segments become ONE fold with summed counts, failures and duration', () => {
    const { s, u } = wakes(4, 2);
    s.round([['bash', { command: 'echo 4' }]]);
    s.answer('All done.');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments).toHaveLength(1);
    const seg = turn!.segments[0]!;
    expect(seg.id).toBe(u.id);
    expect(seg.fold?.detail).toBe('ran 5 commands · 1 failed');
    expect(seg.fold?.label).toMatch(/^Worked for \d+s$/);
    expect(seg.fold?.state).toBe('done');
    // every step stays inside, in order
    const cmds = seg.nodes.flatMap((n) => (n.kind === 'group' ? n.tools : n.kind === 'tool' ? [n.tool] : [])).map((t) => (t.args as { command: string }).command);
    expect(cmds).toEqual(['echo 0', 'echo 1', 'echo 2', 'echo 3', 'echo 4']);
    // the final text stays outside the fold
    expect(seg.answer?.text).toBe('All done.');
    expect(seg.work.some((w) => w.kind === 'prose')).toBe(false);
  });

  it('a final text in its own work-free segment stays below the one fold', () => {
    const { s } = wakes(3);
    s.answer('All done.');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments.map((g) => [!!g.fold, g.answer?.text])).toEqual([[true, undefined], [false, 'All done.']]);
    expect(turn!.segments[0]!.fold?.detail).toBe('ran 3 commands');
  });

  it('a trailing run with no final text is one settled fold too', () => {
    const { s } = wakes(4);
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments).toHaveLength(1);
    expect(turn!.segments[0]!.fold?.detail).toBe('ran 4 commands');
    expect(turn!.segments[0]!.answer).toBeUndefined();
  });

  it('visible prose between folds keeps them apart', () => {
    const s = session();
    s.user('go');
    s.round([['bash', { command: 'a' }]]);
    s.answer('first');
    s.round([['bash', { command: 'b' }]]);
    s.answer('second');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments.map((g) => g.answer?.text)).toEqual(['first', 'second']);
    expect(turn!.segments.map((g) => g.fold?.detail)).toEqual(['ran 1 command', 'ran 1 command']);
  });

  it('a steer bubble between folds keeps them apart', () => {
    const s = session();
    s.user('go');
    const { ids } = s.tools([['bash', { command: 'a' }]]);
    s.result(ids[0]!, 'bash');
    s.user('actually, also b');
    s.round([['bash', { command: 'b' }]]);
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments).toHaveLength(2);
    expect(turn!.segments.every((g) => g.fold)).toBe(true);
  });

  it('merged folds survive the incremental index', () => {
    const { s } = wakes(3);
    const index = createTurnsIndex();
    const first = index.update(input(s.entries));
    expect(first[0]!.segments).toHaveLength(1);
    expect(index.update(input(s.entries, { entriesRev: 2 }))[0]!.segments).toHaveLength(1);
  });
});

describe('turns: the work trailing an answer joins the next fold (OIN-96, real swarm shape)', () => {
  type NewEntry = Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>;
  const raw = (s: ReturnType<typeof session>, message: Record<string, unknown>) =>
    s.add({ type: 'message', message: { timestamp: 0, ...message } } as unknown as NewEntry, 2000);
  /** A supervisor wake: prose + a tool call in ONE message, its result, then an empty `stop` (what a yielding swarm lead writes). */
  const proseAndWork = (s: ReturnType<typeof session>, id: string, text: string, command: string) => {
    raw(s, { role: 'assistant', content: [{ type: 'text', text }, { type: 'toolCall', id, name: 'bash', arguments: { command } }], stopReason: 'toolUse' });
    s.result(id, 'bash');
    s.answer('');
  };
  const cmdsOf = (nodes: ReadonlyArray<WorkNode>) => nodes.flatMap((n) => (n.kind === 'group' ? n.tools : n.kind === 'tool' ? [n.tool] : [])).map((t) => (t.args as { command: string }).command);

  it('answer + trailing work, then a work-only wake: ONE fold under the answer, all steps in order', () => {
    const s = session();
    s.user('go');
    s.round([['bash', { command: 'a' }]]);
    proseAndWork(s, 'p1', 'Status: started.', 'b');
    s.round([['bash', { command: 'c' }]]);
    s.answer('');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments).toHaveLength(2);
    const [first, second] = turn!.segments as [Segment, Segment];
    expect(first.answer?.text).toBe('Status: started.');
    expect(first.after).toBeUndefined();
    expect(cmdsOf(second.nodes)).toEqual(['b', 'c']);
    expect(second.fold?.detail).toBe('ran 2 commands');
    expect(first.fold?.detail).toBe('ran 1 command');
  });

  it('seven wakes after the answer stay ONE fold; the answer is the only prose', () => {
    const s = session();
    s.user('go');
    proseAndWork(s, 'p0', 'Dispatched.', 'w0');
    for (let i = 1; i <= 7; i++) {
      s.round([['bash', { command: `w${i}` }]]);
      s.answer('');
    }
    const [turn] = buildTurns(input(s.entries));
    const folds = turn!.segments.filter((g) => g.fold || g.after?.length);
    expect(folds).toHaveLength(1);
    expect(folds[0]!.fold?.detail).toBe('ran 8 commands');
    expect(cmdsOf(folds[0]!.nodes)).toEqual(['w0', 'w1', 'w2', 'w3', 'w4', 'w5', 'w6', 'w7']);
    expect(turn!.segments.map((g) => g.after).filter(Boolean)).toHaveLength(0);
  });

  it('visible prose between wakes keeps the folds apart: prose, ONE fold, prose', () => {
    const s = session();
    s.user('go');
    proseAndWork(s, 'p0', 'First.', 'a');
    s.round([['bash', { command: 'b' }]]);
    s.answer('Second.');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments.map((g) => g.answer?.text)).toEqual(['First.', 'Second.']);
    // the work after "First." and the wake's own work are ONE fold between the two texts
    expect(turn!.segments[0]!.after).toBeUndefined();
    expect(turn!.segments[0]!.fold).toBeUndefined();
    expect(turn!.segments[1]!.fold?.detail).toBe('ran 2 commands');
  });

  it('a visible card after the trailing work keeps the two rows apart', () => {
    const s = session();
    s.user('go');
    proseAndWork(s, 'p0', 'Dispatched.', 'a');
    s.add({ type: 'model_change', provider: 'mock', modelId: 'mock-2' } as unknown as NewEntry);
    s.round([['bash', { command: 'b' }]]);
    s.answer('');
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments[0]!.after?.length).toBeGreaterThan(0);
    expect(cmdsOf(turn!.segments.at(-1)!.nodes)).toEqual(['b']);
  });

  it('while a wake is running: the trailing run and the live "Working" fold are ONE row', () => {
    const s = session();
    s.user('go');
    proseAndWork(s, 'p0', 'Dispatched.', 'a');
    // the wake's call has no result yet
    raw(s, { role: 'assistant', content: [{ type: 'toolCall', id: 'live1', name: 'bash', arguments: { command: 'b' } }], stopReason: 'toolUse' });
    const [turn] = buildTurns(input(s.entries, { run: { running: true }, liveTools: { live1: { id: 'live1', name: 'bash', args: { command: 'b' }, done: false } as never } }));
    const folds = turn!.segments.filter((g) => g.fold || g.after?.length);
    expect(folds).toHaveLength(1);
    expect(turn!.segments[0]!.after).toBeUndefined();
    expect(folds[0]!.fold?.state).toBe('running');
    expect(cmdsOf(folds[0]!.nodes)).toEqual(['a', 'b']);
  });

  it('an answered ask_user card stays; the wakes after it are one fold', () => {
    const s = session();
    s.user('go');
    s.round([['bash', { command: 'a' }]]);
    const { ids } = s.tools([['ask_user', { questions: [{ question: 'Which?' }] }]]);
    s.result(ids[0]!, 'ask_user');
    for (let i = 0; i < 6; i++) {
      s.round([['bash', { command: `w${i}` }]]);
      s.answer('');
    }
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments).toHaveLength(2);
    expect(turn!.segments[0]!.ask?.name).toBe('ask_user');
    expect(turn!.segments[1]!.fold?.detail).toBe('ran 6 commands');
  });

  it('a question still waiting: the wakes that ran meanwhile are one fold below the card', () => {
    const s = session();
    s.user('go');
    s.tools([['ask_user', { questions: [{ question: 'Which?' }] }]]);
    for (let i = 0; i < 5; i++) {
      s.round([['bash', { command: `w${i}` }]]);
      s.answer('');
    }
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments.filter((g) => g.fold)).toHaveLength(1);
    expect(turn!.segments[0]!.ask?.name).toBe('ask_user');
  });
});

describe('turns: the last assistant text of a settled turn never hides in the fold (OIN-45)', () => {
  type NewEntry = Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>;
  const raw = (s: ReturnType<typeof session>, message: Record<string, unknown>) =>
    s.add({ type: 'message', message: { timestamp: 0, ...message } } as unknown as NewEntry, 2000);
  /** The one segment of a one-turn session; asserts the answer is outside and no prose is left in the fold. */
  const settled = (s: ReturnType<typeof session>) => {
    const [turn] = buildTurns(input(s.entries));
    return { turn: turn!, seg: turn!.segments.at(-1)! };
  };
  const expectOut = (seg: ReturnType<typeof settled>['seg'], text: string) => {
    expect(seg.answer?.text).toBe(text);
    expect(seg.work.some((w) => w.kind === 'prose' && w.text.includes(text))).toBe(false);
  };

  it('the final message also holds tool calls: its text is the answer, the calls fold or follow it', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'Here is the summary.' }, { type: 'toolCall', id: 'td', name: 'todowrite', arguments: { todos: [] } }], stopReason: 'toolUse' });
    s.result('td', 'todowrite');
    const { seg } = settled(s);
    expectOut(seg, 'Here is the summary.');
  });

  it('text, a todowrite round and an empty stop: the text stays outside, the todo round follows it', () => {
    const s = session();
    s.user('go');
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'All findings below.' }, { type: 'toolCall', id: 'td', name: 'todowrite', arguments: { todos: [] } }], stopReason: 'toolUse' });
    s.result('td', 'todowrite');
    s.answer('');
    const { seg } = settled(s);
    expectOut(seg, 'All findings below.');
    expect(seg.after?.length).toBeGreaterThan(0);
  });

  it('the final text followed by state-only, usage and card entries is still the answer', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    s.answer('The report.');
    s.add({ type: 'custom', customType: 'oino.sent-frontier', data: {} } as unknown as NewEntry);
    s.add({ type: 'custom', customType: 'oino.image-policy', data: {} } as unknown as NewEntry);
    s.add({ type: 'custom', customType: 'unipi-usage', data: { tokens: 12 } } as unknown as NewEntry);
    s.add({ type: 'custom_message', customType: 'todo-reminder', content: 'You have open todos', display: true } as unknown as NewEntry);
    const { seg, turn } = settled(s);
    expectOut(seg, 'The report.');
    expect(turn.segments).toHaveLength(1);
  });

  it('a todo reminder wake after the final text keeps the final text out of the fold', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    s.answer('Summary of the work.');
    s.add({ type: 'custom_message', customType: 'todo-reminder', content: 'reminder', display: true } as unknown as NewEntry);
    s.round([['todowrite', { todos: [] }]]);
    s.answer('');
    const { turn } = settled(s);
    const answers = turn.segments.map((g) => g.answer?.text).filter(Boolean);
    expect(answers).toEqual(['Summary of the work.']);
    expect(turn.segments.every((g) => !g.work.some((w) => w.kind === 'prose'))).toBe(true);
  });

  it('aborted after text: the text stays outside, the fold reads Stopped', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'Half an answer' }], stopReason: 'aborted' });
    const { seg } = settled(s);
    expectOut(seg, 'Half an answer');
    expect(seg.fold?.state).toBe('stopped');
  });

  it('error after text: the text stays outside and the error row follows it', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'Almost there.' }], stopReason: 'error', errorMessage: 'upstream 500' });
    const { seg } = settled(s);
    expectOut(seg, 'Almost there.');
    expect(seg.after?.some((n) => n.kind === 'error')).toBe(true);
  });

  it('thinking blocks around the final text do not pull it into the fold', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]], 'looking');
    raw(s, { role: 'assistant', content: [{ type: 'thinking', thinking: 'wrap up' }, { type: 'text', text: 'Done and dusted.' }, { type: 'thinking', thinking: 'anything else?' }], stopReason: 'stop' });
    const { seg } = settled(s);
    expectOut(seg, 'Done and dusted.');
  });

  it('a final message with several text blocks shows all of them outside the fold', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'Part one.' }, { type: 'text', text: 'Part two.' }], stopReason: 'stop' });
    const { seg } = settled(s);
    expect(seg.answer?.text).toBe('Part one.\n\nPart two.');
    expect(seg.work.some((w) => w.kind === 'prose')).toBe(false);
  });

  it('a final message whose content is a plain string still shows', () => {
    const s = session();
    s.user('go');
    s.round([['read', { path: 'a' }]]);
    raw(s, { role: 'assistant', content: 'Plain string answer.', stopReason: 'stop' });
    const { seg } = settled(s);
    expectOut(seg, 'Plain string answer.');
  });

  it('text before an unanswered tool call (run stopped mid-tool) stays outside the fold', () => {
    const s = session();
    s.user('go');
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'Running the build now.' }, { type: 'toolCall', id: 'b1', name: 'bash', arguments: { command: 'make' } }], stopReason: 'aborted' });
    const { seg } = settled(s);
    expectOut(seg, 'Running the build now.');
  });

  it('the text survives merged folds after wakes', () => {
    const s = session();
    s.user('swarm');
    for (let i = 0; i < 2; i++) {
      const { ids } = s.tools([['bash', { command: `echo ${i}` }]]);
      s.result(ids[0]!, 'bash');
      s.answer('');
    }
    raw(s, { role: 'assistant', content: [{ type: 'text', text: 'Swarm finished.' }, { type: 'toolCall', id: 'tz', name: 'todowrite', arguments: { todos: [] } }], stopReason: 'toolUse' });
    s.result('tz', 'todowrite');
    const { turn } = settled(s);
    expect(turn.segments.at(-1)!.answer?.text).toBe('Swarm finished.');
    expect(turn.segments.flatMap((g) => g.work).some((w) => w.kind === 'prose')).toBe(false);
  });
});
