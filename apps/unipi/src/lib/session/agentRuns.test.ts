/**
 * [ui-subagents] turn model: child steps join their `run_subagent` call as one
 * `agent` item (never loose rows), parallel calls share one `agents` node,
 * statuses / counters / summaries come from details, steps and completions.
 */
import { describe, expect, it } from 'vitest';
import { buildTurns, type TurnsInput, type WorkItem, type WorkNode } from './turns';
import { agentsSummary, liveLineOf, runCounters, type AgentRun } from './agentRuns';
import type { AgentMessage, LiveTool, SessionEntry } from './types';

const T0 = Date.UTC(2026, 10, 2, 13, 0, 0);

function session() {
  const entries: SessionEntry[] = [];
  let t = T0;
  const add = (e: Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>): SessionEntry => {
    t += 1000;
    const entry = { ...e, id: `e${entries.length + 1}`, parentId: entries.at(-1)?.id ?? null, timestamp: new Date(t).toISOString() } as SessionEntry;
    entries.push(entry);
    return entry;
  };
  const msg = (message: AgentMessage) => add({ type: 'message', message: { timestamp: t + 1000, ...message } });
  return {
    entries,
    add,
    user: (text: string) => msg({ role: 'user', content: [{ type: 'text', text }] }),
    calls: (calls: Array<[name: string, args: Record<string, unknown>, id: string]>) =>
      msg({ role: 'assistant', content: calls.map(([name, args, id]) => ({ type: 'toolCall', id, name, arguments: args })), stopReason: 'toolUse' }),
    result: (id: string, name: string, text: string, details?: Record<string, unknown>, isError = false) =>
      msg({ role: 'toolResult', toolCallId: id, toolName: name, content: [{ type: 'text', text }], isError, ...(details ? { details } : {}) }),
    step: (agentId: string, title: string, data: Record<string, unknown>) =>
      add({ type: 'custom', customType: 'subagent-step', data: { kind: 'tool', isError: false, durationMs: 5, ...data, agentId, id: agentId, title, profile: 'explore' } } as Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>),
    answer: (text: string) => msg({ role: 'assistant', content: [{ type: 'text', text }], stopReason: 'stop' }),
  };
}

const input = (entries: SessionEntry[], over: Partial<TurnsInput> = {}): TurnsInput => ({ entries, leafId: entries.at(-1)?.id ?? null, entriesRev: 1, liveTools: {}, run: { running: false }, ...over });

const doneDetails = (id: string, title: string, over: Record<string, unknown> = {}) => ({ owner: 'subagents', id, title, profile: 'explore', profileLabel: 'Explore subagent', model: 'zai/glm-5.3', status: 'completed', phase: 'done', toolCalls: 3, durationMs: 42_000, ...over });

const agents = (items: readonly (WorkItem | WorkNode)[]): AgentRun[] => items.flatMap((w) => (w.kind === 'agent' ? [w.run] : w.kind === 'agents' ? w.runs : []));

describe('subagent runs: one card per run_subagent call', () => {
  it('parallel calls group into one container; child steps never become top-level rows', () => {
    const s = session();
    s.user('go');
    s.calls([
      ['run_subagent', { title: 'Provision sandbox', profile: 'general', task: 't' }, 'c1'],
      ['run_subagent', { title: 'Inventory apps', profile: 'argon', task: 't' }, 'c2'],
      ['run_subagent', { title: 'Find Stripe config', profile: 'scout', task: 't' }, 'c3'],
    ]);
    // Interleaved in arrival order, as the extension appends them.
    s.step('a1', 'Provision sandbox', { name: 'bash', arg: 'cat > /tmp/zmsb/build.sh <<EOF', args: { command: 'cat > /tmp/zmsb/build.sh <<EOF' } });
    s.step('a2', 'Inventory apps', { name: 'read', arg: '/p/a.vue', args: { path: '/p/a.vue' } });
    s.step('a3', 'Find Stripe config', { name: 'grep', arg: 'STRIPE', args: { pattern: 'STRIPE' } });
    s.step('a2', 'Inventory apps', { name: 'read', arg: '/p/b.vue', args: { path: '/p/b.vue' } });
    s.step('a1', 'Provision sandbox', { name: 'bash', arg: 'a37 up', args: { command: 'a37 up' } });
    const live: Record<string, LiveTool> = {
      c1: { id: 'c1', name: 'run_subagent', args: {}, startedAt: T0, details: { id: 'a1', title: 'Provision sandbox', status: 'running', startedAt: T0, current: { name: 'bash', arg: 'ssh box make' } } },
      c2: { id: 'c2', name: 'run_subagent', args: {}, startedAt: T0, details: { id: 'a2', title: 'Inventory apps', status: 'running' } },
    };
    s.result('c3', 'run_subagent', 'Found it in .env.test\n\n--- subagent a3 · completed · 1 tool call · 4.0s', doneDetails('a3', 'Find Stripe config', { toolCalls: 1 }));
    const [turn] = buildTurns(input(s.entries, { liveTools: live, run: { running: true, startedAt: T0 } }));
    const seg = turn!.segments[0]!;
    // No loose step rows anywhere in the fold.
    expect(seg.work.some((w) => w.kind === 'step')).toBe(false);
    expect(seg.nodes).toHaveLength(1);
    const node = seg.nodes[0]!;
    expect(node.kind).toBe('agents');
    const runs = agents([node]);
    expect(runs.map((r) => [r.title, r.status, r.steps.length])).toEqual([
      ['Provision sandbox', 'running', 2],
      ['Inventory apps', 'running', 2],
      ['Find Stripe config', 'done', 1],
    ]);
    // Each card owns only its child's steps.
    expect(runs[0]!.steps.every((e) => (e.data as { agentId: string }).agentId === 'a1')).toBe(true);
    expect(runs[1]!.steps.every((e) => (e.data as { agentId: string }).agentId === 'a2')).toBe(true);
    // Live line: the running tool from the progress details, else the latest step.
    expect(liveLineOf(runs[0]!)).toEqual({ name: 'bash', text: 'ssh box make' });
    expect(liveLineOf(runs[1]!)).toEqual({ name: 'read', text: '/p/b.vue' });
    expect(runs[2]!.report).toBe('Found it in .env.test');
    // Header: counted by subagent, apart from the parent's own tools.
    expect(seg.fold!.detail).toBe('3 subagents · 2 running · 1 done');
  });

  it('counters: tools from details or steps, distinct files read / edited', () => {
    const s = session();
    s.user('go');
    s.calls([['run_subagent', { title: 'Inventory', profile: 'explore' }, 'c1']]);
    s.step('a1', 'Inventory', { name: 'read', arg: '/p/a.ts', args: { path: '/p/a.ts' } });
    s.step('a1', 'Inventory', { name: 'read', arg: '/p/a.ts', args: { path: '/p/a.ts' } });
    s.step('a1', 'Inventory', { name: 'edit', arg: '/p/b.ts', args: { path: '/p/b.ts' } });
    s.step('a1', 'Inventory', { name: 'bash', arg: 'ls', args: { command: 'ls' } });
    s.result('c1', 'run_subagent', 'done', doneDetails('a1', 'Inventory', { toolCalls: 12 }));
    const [turn] = buildTurns(input(s.entries));
    const run = agents(turn!.segments[0]!.nodes)[0]!;
    expect(run.toolCalls).toBe(12);
    expect(run.files).toBe(2);
    expect(runCounters(run)).toBe('12 tools · 2 files');
    expect(run.durationMs).toBe(42_000);
    expect(run.profileLabel).toBe('Explore subagent');
  });

  it("the parent's own tools are counted separately from its subagents", () => {
    const s = session();
    s.user('go');
    s.calls([['bash', { command: 'ls' }, 'b1']]);
    s.result('b1', 'bash', 'a b');
    s.calls([['run_subagent', { title: 'One' }, 'c1']]);
    s.step('a1', 'One', { name: 'read', arg: '/x' });
    s.result('c1', 'run_subagent', 'Failed: boom', doneDetails('a1', 'One', { status: 'failed', error: 'model refused' }), true);
    s.answer('ok');
    const [turn] = buildTurns(input(s.entries));
    const seg = turn!.segments[0]!;
    expect(seg.fold!.detail).toBe('1 subagent · 1 failed · ran 1 command');
    const run = agents(seg.nodes)[0]!;
    expect(run.status).toBe('failed');
    expect(run.error).toBe('model refused');
  });

  it('a single run is a standalone card (no container)', () => {
    const s = session();
    s.user('go');
    s.calls([['run_subagent', { title: 'Solo' }, 'c1']]);
    s.step('a1', 'Solo', { name: 'read', arg: '/x' });
    s.result('c1', 'run_subagent', 'r', doneDetails('a1', 'Solo'));
    const [turn] = buildTurns(input(s.entries));
    expect(turn!.segments[0]!.nodes.map((n) => n.kind)).toEqual(['agent']);
  });

  it('a reopened mid-run call (no details yet) claims its steps by title', () => {
    const s = session();
    s.user('go');
    s.calls([
      ['run_subagent', { title: 'Alpha' }, 'c1'],
      ['run_subagent', { title: 'Beta' }, 'c2'],
    ]);
    s.step('b9', 'Beta', { name: 'read', arg: '/b' });
    s.step('a9', 'Alpha', { name: 'read', arg: '/a' });
    const [turn] = buildTurns(input(s.entries, { run: { running: true } }));
    const runs = agents(turn!.segments[0]!.nodes);
    expect(runs.map((r) => [r.title, r.agentId, r.steps.length])).toEqual([
      ['Alpha', 'a9', 1],
      ['Beta', 'b9', 1],
    ]);
    expect(turn!.segments[0]!.work.some((w) => w.kind === 'step')).toBe(false);
  });

  it('steps of a child whose call is in an earlier turn become one card at the first step', () => {
    const s = session();
    s.user('later');
    s.step('z1', 'Background job', { name: 'bash', arg: 'make' });
    s.step('z1', 'Background job', { name: 'bash', arg: 'make test' });
    s.add({ type: 'custom_message', customType: 'subagent-completion', content: 'done', display: true, details: { owner: 'subagents', id: 'z1', title: 'Background job', status: 'completed', toolCalls: 2, durationMs: 9000, report: 'All green' } } as unknown as Omit<SessionEntry, 'id' | 'parentId' | 'timestamp'>);
    s.answer('ok');
    const [turn] = buildTurns(input(s.entries));
    const seg = turn!.segments[0]!;
    const items = seg.work.filter((w) => w.kind === 'agent');
    expect(items).toHaveLength(1);
    const run = (items[0] as Extract<WorkItem, { kind: 'agent' }>).run;
    expect([run.title, run.status, run.steps.length, run.report]).toEqual(['Background job', 'done', 2, 'All green']);
  });

  it('a backgrounded run reads "in background" once the parent turn settled', () => {
    const s = session();
    s.user('go');
    s.calls([['run_subagent', { title: 'Slow', is_background: true }, 'c1']]);
    s.result('c1', 'run_subagent', 'Subagent a1 started in the background.', { owner: 'subagents', id: 'a1', title: 'Slow', status: 'running', phase: 'started' });
    s.answer('started');
    const [turn] = buildTurns(input(s.entries));
    const run = agents(turn!.segments[0]!.nodes)[0]!;
    expect(run.status).toBe('background');
    expect(run.report).toBeUndefined();
  });
});

describe('agentsSummary', () => {
  const r = (status: AgentRun['status']): AgentRun => ({ title: 't', status, toolCalls: 0, files: 0, steps: [], depth: 0 });
  it('counts by status', () => {
    expect(agentsSummary([r('running'), r('running'), r('done')])).toBe('3 subagents · 2 running · 1 done');
    expect(agentsSummary([r('done')])).toBe('1 subagent');
    expect(agentsSummary([r('failed'), r('cancelled'), r('background')])).toBe('3 subagents · 1 failed · 1 cancelled · 1 in background');
    expect(agentsSummary([])).toBe('');
  });
});
