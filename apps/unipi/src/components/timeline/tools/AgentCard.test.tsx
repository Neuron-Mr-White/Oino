// @vitest-environment jsdom
/**
 * [ui-subagents] cards: parallel runs stack in one "Running 3 subagents"
 * container (never interleaved rows), each card shows its live line
 * collapsed and its own grouped steps expanded, nested runs nest, the step
 * list is capped, and a progress update re-renders only its card.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, describe, expect, it } from 'vitest';
import { buildTurns, type TurnsInput, type WorkNode } from '../../../lib/session/turns';
import type { LiveTool, SessionEntry } from '../../../lib/session/types';
import { WorkNodes } from './WorkItem';
import { AGENT_STEP_CAP } from './AgentCard';
import { fakeCtx } from './testutil';

afterEach(cleanup);

const T0 = Date.UTC(2026, 10, 2, 13, 0, 0);
let n = 0;
const entry = (e: Partial<SessionEntry>): SessionEntry => ({ id: `x${++n}`, parentId: null, timestamp: new Date(T0 + n * 1000).toISOString(), ...e }) as SessionEntry;
const user = () => entry({ type: 'message', message: { role: 'user', content: [{ type: 'text', text: 'go' }] } });
const calls = (cs: Array<[title: string, id: string]>) =>
  entry({ type: 'message', message: { role: 'assistant', stopReason: 'toolUse', content: cs.map(([title, id]) => ({ type: 'toolCall', id, name: 'run_subagent', arguments: { title, profile: 'explore', task: 't' } })) } });
const step = (agentId: string, title: string, name: string, arg: string, extra: Record<string, unknown> = {}) =>
  entry({ type: 'custom', customType: 'subagent-step', data: { kind: 'tool', name, arg, args: name === 'bash' ? { command: arg } : name === 'run_subagent' ? { title: arg, profile: 'scout' } : { path: arg }, output: 'ok', isError: false, durationMs: 3, agentId, id: agentId, title, profile: 'explore', ...extra } });
const liveRun = (id: string, agentId: string, title: string, current?: { name: string; arg: string }): LiveTool => ({
  id,
  name: 'run_subagent',
  args: {},
  startedAt: T0,
  details: { owner: 'subagents', id: agentId, title, profile: 'explore', profileLabel: 'Explore subagent', model: 'zai/glm-5.3', status: 'running', startedAt: T0, ...(current ? { current } : {}) },
});
const input = (entries: SessionEntry[], liveTools: Record<string, LiveTool>, running = true): TurnsInput => ({ entries, leafId: null, entriesRev: 1, liveTools, run: { running } });
/** All entries are parentless (`leafId: null` keeps them all on the branch through `branchOf`'s fallback). */
const nodesOf = (entries: SessionEntry[], live: Record<string, LiveTool>, running = true): WorkNode[] => {
  const chain = entries.map((e, i) => ({ ...e, parentId: i ? entries[i - 1]!.id : null }));
  return buildTurns({ ...input(chain, live, running), leafId: chain.at(-1)!.id })[0]!.segments[0]!.nodes;
};

function parallel() {
  const es = [
    user(),
    calls([
      ['Provision sandbox', 'c1'],
      ['Inventory apps', 'c2'],
      ['Find Stripe config', 'c3'],
    ]),
    step('a1', 'Provision sandbox', 'bash', 'cat > /tmp/zmsb/build.sh <<EOF'),
    step('a2', 'Inventory apps', 'read', '/p/a.vue'),
    step('a3', 'Find Stripe config', 'grep', 'STRIPE'),
    step('a2', 'Inventory apps', 'read', '/p/b.vue'),
    step('a2', 'Inventory apps', 'read', '/p/c.vue'),
    step('a1', 'Provision sandbox', 'bash', 'a37 up'),
  ];
  const live = {
    c1: liveRun('c1', 'a1', 'Provision sandbox', { name: 'bash', arg: 'ssh box   make\n all' }),
    c2: liveRun('c2', 'a2', 'Inventory apps'),
    c3: liveRun('c3', 'a3', 'Find Stripe config'),
  };
  return { es, live };
}

describe('AgentsGroup + AgentCard', () => {
  it('parallel runs stack under one container; no child step is a top-level row', () => {
    const { es, live } = parallel();
    const ctx = fakeCtx([]);
    const r = render(() => <WorkNodes nodes={nodesOf(es, live)} ctx={ctx} live />);
    const group = r.getByTestId('tl-agents');
    expect(r.getByTestId('tl-agents-title').textContent).toBe('Running 3 subagents');
    expect(r.getByTestId('tl-agents-detail').textContent).toBe('3 running');
    const cards = r.getAllByTestId('tl-agent');
    expect(cards.map((c) => c.querySelector('.tl-agent-title')!.textContent)).toEqual(['Provision sandbox', 'Inventory apps', 'Find Stripe config']);
    cards.forEach((c) => expect(group.contains(c)).toBe(true));
    // Collapsed: no tool rows at all (the old layout had one per child step, interleaved).
    expect(r.container.querySelectorAll('[data-testid=tl-tool-row]')).toHaveLength(0);
    expect(r.container.querySelectorAll('[data-testid=tl-step]')).toHaveLength(0);
    // Live line per card: current tool (whitespace collapsed) or the latest step.
    const lines = cards.map((c) => c.querySelector('[data-testid=tl-agent-line]')!.textContent);
    expect(lines).toEqual(['bashssh box make all', 'read/p/c.vue', 'grepSTRIPE']);
    expect(cards[0]!.querySelector('[data-testid=tl-agent-chip]')!.textContent).toContain('Explore subagent');
    expect(cards[1]!.querySelector('[data-testid=tl-agent-counters]')!.textContent).toBe('3 tools · 3 files');
    expect(cards[0]!.querySelector('.tl-agent-status')!.getAttribute('data-status')).toBe('running');
  });

  it('expanding a card shows only its own steps, grouped like the parent tools', () => {
    const { es, live } = parallel();
    const ctx = fakeCtx([]);
    const r = render(() => <WorkNodes nodes={nodesOf(es, live)} ctx={ctx} live />);
    const card = r.getAllByTestId('tl-agent')[1]!;
    fireEvent.click(card.querySelector('[data-testid=tl-agent-toggle]')!);
    const body = card.querySelector('[data-testid=tl-agent-body]')!;
    expect(body).toBeTruthy();
    // Three consecutive reads fold into one "Explored · 3 steps" group.
    const groups = body.querySelectorAll('[data-testid=tl-group]');
    expect(groups).toHaveLength(1);
    expect(groups[0]!.textContent).toContain('3 steps');
    // The other cards stay collapsed; nothing leaked out of the card.
    expect(r.container.querySelectorAll('[data-testid=tl-agent-body]')).toHaveLength(1);
    expect(card.querySelector('[data-testid=tl-agent-line]')).toBeNull();
  });

  it('a done run shows its report excerpt; a failed one its error', () => {
    const es = [
      user(),
      calls([
        ['Good', 'd1'],
        ['Bad', 'd2'],
      ]),
      step('g1', 'Good', 'read', '/a'),
      entry({ type: 'message', message: { role: 'toolResult', toolCallId: 'd1', toolName: 'run_subagent', content: [{ type: 'text', text: 'Stripe keys live in .env.test\n\n--- subagent g1 · completed · 1 tool call · 2.0s' }], details: { id: 'g1', title: 'Good', status: 'completed', phase: 'done', toolCalls: 1, durationMs: 2000 } } }),
      entry({ type: 'message', message: { role: 'toolResult', toolCallId: 'd2', toolName: 'run_subagent', isError: true, content: [{ type: 'text', text: 'Failed: quota' }], details: { id: 'b1', title: 'Bad', status: 'failed', phase: 'done', error: 'quota exceeded' } } }),
    ];
    const ctx = fakeCtx([]);
    const r = render(() => <WorkNodes nodes={nodesOf(es, {}, false)} ctx={ctx} />);
    expect(r.getByTestId('tl-agents-title').textContent).toBe('Ran 2 subagents');
    expect(r.getByTestId('tl-agents-detail').textContent).toBe('1 done · 1 failed');
    const [good, bad] = r.getAllByTestId('tl-agent');
    expect(good!.querySelector('[data-testid=tl-agent-line]')!.textContent).toBe('Stripe keys live in .env.test');
    expect(good!.querySelector('.tl-agent-elapsed')!.textContent).toBe('2s');
    expect(bad!.querySelector('[data-testid=tl-agent-line]')!.getAttribute('data-kind')).toBe('error');
    expect(bad!.querySelector('[data-testid=tl-agent-line]')!.textContent).toBe('quota exceeded');
  });

  it("a child's own run_subagent step nests as a card inside the expanded body", () => {
    const es = [user(), calls([['Lead', 'n1']]), step('l1', 'Lead', 'read', '/a'), step('l1', 'Lead', 'run_subagent', 'Grandchild task')];
    const ctx = fakeCtx([]);
    const r = render(() => <WorkNodes nodes={nodesOf(es, { n1: liveRun('n1', 'l1', 'Lead') })} ctx={ctx} live />);
    expect(r.queryByTestId('tl-agents')).toBeNull(); // a single run: no container
    const outer = r.getAllByTestId('tl-agent');
    expect(outer).toHaveLength(1);
    fireEvent.click(outer[0]!.querySelector('[data-testid=tl-agent-toggle]')!);
    const nested = outer[0]!.querySelector('[data-testid=tl-agent-body] [data-testid=tl-agent]')!;
    expect(nested).toBeTruthy();
    expect(nested.getAttribute('data-depth')).toBe('1');
    expect(nested.querySelector('.tl-agent-title')!.textContent).toBe('Grandchild task');
    expect(nested.querySelector('.tl-agent-status')!.getAttribute('data-status')).toBe('done');
  });

  it(`caps the rendered steps at ${AGENT_STEP_CAP}; older ones behind "Show earlier"`, () => {
    const es = [user(), calls([['Big', 'm1']])];
    for (let i = 0; i < AGENT_STEP_CAP + 50; i++) es.push(step('big', 'Big', i % 2 ? 'bash' : 'edit', `cmd-${i}`));
    const ctx = fakeCtx([]);
    const r = render(() => <WorkNodes nodes={nodesOf(es, { m1: liveRun('m1', 'big', 'Big') })} ctx={ctx} live />);
    fireEvent.click(r.getByTestId('tl-agent-toggle'));
    const rows = () => r.getByTestId('tl-agent-body').querySelectorAll('[data-testid=tl-tool-row]').length;
    expect(rows()).toBe(AGENT_STEP_CAP);
    const earlier = r.getByTestId('tl-agent-earlier');
    expect(earlier.textContent).toContain('Show 50 earlier steps');
    fireEvent.click(earlier);
    expect(rows()).toBe(AGENT_STEP_CAP + 50);
    expect(r.queryByTestId('tl-agent-earlier')).toBeNull();
  });

  it('a child progress update re-renders only its own card (other cards keep their DOM)', () => {
    const { es, live } = parallel();
    const [entries, setEntries] = createSignal(es);
    const [tools, setTools] = createSignal<Record<string, LiveTool>>(live);
    const ctx = fakeCtx([]);
    const r = render(() => <WorkNodes nodes={nodesOf(entries(), tools())} ctx={ctx} live />);
    const before = r.getAllByTestId('tl-agent');
    const otherLine = before[1]!.querySelector('[data-testid=tl-agent-line]')!;
    setEntries([...es, step('a1', 'Provision sandbox', 'bash', 'make deploy')]);
    setTools({ ...live, c1: liveRun('c1', 'a1', 'Provision sandbox', { name: 'bash', arg: 'make deploy' }) });
    const after = r.getAllByTestId('tl-agent');
    expect(after).toHaveLength(3);
    // Same card elements (keyed), the untouched card's line node is the very same node.
    after.forEach((c, i) => expect(c).toBe(before[i]));
    expect(after[1]!.querySelector('[data-testid=tl-agent-line]')).toBe(otherLine);
    expect(after[0]!.querySelector('[data-testid=tl-agent-line]')!.textContent).toBe('bashmake deploy');
    expect(after[0]!.querySelector('[data-testid=tl-agent-counters]')!.textContent).toBe('3 tools');
  });
});
