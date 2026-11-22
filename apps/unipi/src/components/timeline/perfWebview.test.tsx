// @vitest-environment jsdom
/**
 * perf-webview (v3-perf P5, spec §3 T9 / §4.7): stream deltas append to the live tail without
 * rebuilding the turn; the markdown cache is keyed by entry id + content hash; hidden chats
 * never build turns; long work lists mount a window.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { createRoot, createSignal } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setStreamSmoothing } from '../../lib/timeline/smoothStream';
import { buildTurns, createTurnsIndex, type ToolRun, type TurnsInput, type WorkNode } from '../../lib/session/turns';
import type { LiveTool, SessionEntry } from '../../lib/session/types';
import { clearMdCache, contentHash, mdBlocks, mdCacheSize } from '../../lib/timeline/mdCache';
import * as markdown from '../../lib/chat/markdown';
import type { TimelineCtx } from './context';
import { TurnRow } from './TurnRow';
import { WORK_WINDOW, WorkNodes } from './tools/WorkItem';
import { lineWindow, OUT_LINES, RunBody } from './tools/bodies';
import { fakeCtx } from './tools/testutil';

afterEach(() => {
  cleanup();
  setStreamSmoothing(undefined);
  vi.restoreAllMocks();
});

const T0 = Date.UTC(2026, 10, 14, 9, 0, 0);
const user = (id: string, parentId: string | null, text: string): SessionEntry =>
  ({ type: 'message', id, parentId, timestamp: new Date(T0).toISOString(), message: { role: 'user', content: [{ type: 'text', text }], timestamp: T0 } }) as SessionEntry;
const answer = (id: string, parentId: string, text: string): SessionEntry =>
  ({ type: 'message', id, parentId, timestamp: new Date(T0 + 1000).toISOString(), message: { role: 'assistant', content: [{ type: 'text', text }], stopReason: 'stop', timestamp: T0 + 1000 } }) as SessionEntry;

const history: SessionEntry[] = [user('u0', null, 'first'), answer('a0', 'u0', 'one'), user('u1', 'a0', 'second')];

/** A live input over the same `entries` array (the store appends in place; streaming parts are patched in place). */
function live(parts: unknown[], liveTools: Record<string, LiveTool> = {}): TurnsInput {
  return {
    entries: history,
    leafId: 'u1',
    entriesRev: 0,
    streaming: { message: { role: 'assistant', content: parts } as never, startedAt: T0 },
    liveTools,
    run: { running: true, startedAt: T0 },
  };
}

describe('turns index: stream fast path (tail append)', () => {
  it('a text delta patches the live turn instead of rebuilding it, sharing every untouched object', () => {
    const index = createTurnsIndex();
    const think = { type: 'thinking', thinking: 'plan' };
    const call = { type: 'toolCall', id: 't1', name: 'read', arguments: { path: 'a.ts' } };
    const first = index.update(live([think, call, { type: 'text', text: 'Hel' }]));
    const second = index.update(live([think, call, { type: 'text', text: 'Hello wor' }]));
    expect(index.lastMode).toBe('stream');
    // Settled turns keep identity; the live turn is a new object (Solid sees the change)…
    expect(second[0]).toBe(first[0]);
    expect(second[1]).not.toBe(first[1]);
    const [a, b] = [first[1]!.segments[0]!, second[1]!.segments[0]!];
    // …but only the streamed item changed: the tool / thinking items and the user entry are shared.
    expect(b.answer).toMatchObject({ text: 'Hello wor', live: true });
    expect(b.work).toBe(a.work);
    expect(b.nodes).toBe(a.nodes);
    expect(b.fold).toBe(a.fold);
    expect(second[1]!.user).toBe(first[1]!.user);
    // Same result as a full build.
    expect(second).toEqual(buildTurns(live([think, call, { type: 'text', text: 'Hello wor' }])));
  });

  it('a thinking delta inside the fold patches that item only', () => {
    const index = createTurnsIndex();
    index.update(live([{ type: 'thinking', thinking: 'a' }]));
    const next = index.update(live([{ type: 'thinking', thinking: 'ab' }]));
    expect(index.lastMode).toBe('stream');
    expect(next).toEqual(buildTurns(live([{ type: 'thinking', thinking: 'ab' }])));
  });

  it('a structural change (new part, tool update, prose appearing) rebuilds the tail', () => {
    const index = createTurnsIndex();
    index.update(live([{ type: 'text', text: '' }]));
    // Whitespace-only text renders no prose: the first visible text is a structure change.
    index.update(live([{ type: 'text', text: 'x' }]));
    expect(index.lastMode).toBe('tail');
    index.update(live([{ type: 'text', text: 'x' }, { type: 'toolCall', id: 't', name: 'bash', arguments: {} }]));
    expect(index.lastMode).toBe('tail');
    const tool: LiveTool = { id: 't', name: 'bash', args: {}, startedAt: T0 };
    index.update(live([{ type: 'text', text: 'x' }, { type: 'toolCall', id: 't', name: 'bash', arguments: {} }], { t: tool }));
    expect(index.lastMode).toBe('tail');
    // In-place partial output (the reducer mutates the live tool) is seen too.
    tool.partial = 'out';
    const out = index.update(live([{ type: 'text', text: 'x' }, { type: 'toolCall', id: 't', name: 'bash', arguments: {} }], { t: tool }));
    expect(index.lastMode).toBe('tail');
    const node = out.at(-1)!.segments[0]!.nodes.find((n) => n.kind === 'tool') as Extract<WorkNode, { kind: 'tool' }>;
    expect(node.tool.live).toBe('out');
  });

  it('many deltas in a row stay equal to a full build', () => {
    const index = createTurnsIndex();
    let text = '';
    for (const piece of ['# Ti', 'tle\n\n', 'para', 'graph\n\n```ts\nconst', ' x = 1;\n```\n', 'end']) {
      text += piece;
      const turns = index.update(live([{ type: 'text', text }]));
      expect(turns).toEqual(buildTurns(live([{ type: 'text', text }])));
    }
    expect(index.lastMode).toBe('stream');
  });
});

describe('session store: hidden chats build no turns', () => {
  it('turns() runs the index only when read', async () => {
    const { createSession } = await import('../../lib/session/store');
    const { createMockHost } = await import('../../dev/mock/host');
    const { genSession } = await import('../../dev/mock/gen');
    const world = createMockHost({ name: 't', hosts: [{ id: 'h', name: 'h', sessions: { s: genSession('s', { turns: 3 }) } }] });
    let flush!: () => void;
    const handle = createRoot(() => createSession('h', 's', { client: world.client('h'), schedule: (f) => (flush = f) }));
    await new Promise((r) => setTimeout(r, 0));
    // Deltas into the store, nobody reading turns: no build (the index's lastMode stays put).
    const before = handle.turns();
    const builds = () => handle.turnsBuilds!();
    const built = builds();
    let seq = handle.state.seq;
    const push = (event: Record<string, unknown>) => world.push('h', 's', [{ session: 's', seq: ++seq, event } as never]);
    push({ type: 'agent_start' });
    push({ type: 'message_start', message: { role: 'assistant', content: [], timestamp: T0 } });
    for (const t of ['a', 'ab', 'abc']) push({ type: 'message_update', message: { role: 'assistant', content: [{ type: 'text', text: t }], timestamp: T0 } });
    flush();
    expect(handle.state.streaming?.message.content).toEqual([{ type: 'text', text: 'abc' }]);
    expect(builds()).toBe(built);
    // A read builds once and reflects the store; a second read without changes is the same array.
    const after = handle.turns();
    expect(after).not.toBe(before);
    expect(after.at(-1)!.segments.at(-1)!.answer).toMatchObject({ text: 'abc', live: true });
    expect(builds()).toBe(built + 1);
    expect(handle.turns()).toBe(after);
    expect(builds()).toBe(built + 1);
    handle.close();
  });
});

const ctx = {
  session: { state: { byId: {}, entries: [] }, key: 'k', turns: () => [] },
  hostId: 'h',
  sessionId: () => 's',
  cwd: () => '/',
  refs: { hostId: 'h', cwd: () => '/' },
  commands: () => [],
  columnWidth: () => 720,
  readOnly: true,
} as unknown as TimelineCtx;

describe('live turn render', () => {
  it('stream flushes keep the user bubble, the settled blocks and the prose node; only the tail block changes', () => {
    setStreamSmoothing(false);
    const index = createTurnsIndex();
    const [turn, setTurn] = createSignal(index.update(live([{ type: 'text', text: 'Para one.\n\nPara' }])).at(-1)!);
    const r = render(() => <TurnRow turn={turn()} ctx={ctx} />);
    const bubble = r.container.querySelector('.tl-user');
    const prose = r.container.querySelector('.tl-prose');
    const blocks = [...r.container.querySelectorAll('.tl-prose .chat-block')];
    expect(blocks).toHaveLength(2);
    const firstHtml = blocks[0]!.innerHTML;
    setTurn(index.update(live([{ type: 'text', text: 'Para one.\n\nPara two grows' }])).at(-1)!);
    expect(index.lastMode).toBe('stream');
    expect(r.container.querySelector('.tl-user')).toBe(bubble);
    expect(r.container.querySelector('.tl-prose')).toBe(prose);
    const now = [...r.container.querySelectorAll('.tl-prose .chat-block')];
    expect(now[0]).toBe(blocks[0]);
    expect(now[0]!.innerHTML).toBe(firstHtml);
    expect(now[1]).toBe(blocks[1]);
    expect(now[1]!.textContent).toContain('Para two grows');
  });
});

describe('markdown memo: entry id + content hash', () => {
  it('renders once per (entry, content); a changed text under the same id renders again', () => {
    clearMdCache();
    const spy = vi.spyOn(markdown, 'renderBlocks');
    const a = mdBlocks('e1-0', 720, 'hello **world**');
    const b = mdBlocks('e1-0', 720, 'hello **world**');
    expect(b).toBe(a);
    expect(spy).toHaveBeenCalledTimes(1);
    const c = mdBlocks('e1-0', 720, 'hello **there**');
    expect(c).not.toBe(a);
    expect(c.join('')).toContain('there');
    expect(spy).toHaveBeenCalledTimes(2);
    // Same width class shares; another class renders.
    mdBlocks('e1-0', 730, 'hello **there**');
    expect(spy).toHaveBeenCalledTimes(2);
    mdBlocks('e1-0', 300, 'hello **there**');
    expect(spy).toHaveBeenCalledTimes(3);
    expect(mdCacheSize()).toBe(3);
  });

  it('contentHash differs for different texts and is stable', () => {
    expect(contentHash('abc')).toBe(contentHash('abc'));
    expect(contentHash('abc')).not.toBe(contentHash('abd'));
    expect(contentHash('')).not.toBe(contentHash(' '));
  });
});

describe('long work lists: windowed', () => {
  const tool = (i: number): WorkNode => ({ kind: 'tool', key: `k${i}`, tool: { id: `t${i}`, name: 'read', args: { path: `f${i}.ts` }, state: 'ok', output: 'x' } as ToolRun });

  it(`mounts the newest ${WORK_WINDOW} nodes and pages older ones in`, () => {
    const nodes = Array.from({ length: WORK_WINDOW + 130 }, (_, i) => tool(i));
    const r = render(() => <WorkNodes nodes={nodes} ctx={ctx} />);
    const rows = () => r.container.querySelectorAll('[data-testid="tl-tool"], .tl-tool').length;
    const first = rows();
    expect(first).toBeLessThanOrEqual(WORK_WINDOW);
    expect(first).toBeGreaterThan(0);
    const more = r.getByTestId('tl-work-more');
    expect(more.textContent).toBe('Show 100 earlier steps');
    fireEvent.click(more);
    expect(rows()).toBe(first + 100);
    expect(r.getByTestId('tl-work-more').textContent).toBe('Show 30 earlier steps');
    fireEvent.click(r.getByTestId('tl-work-more'));
    expect(r.queryByTestId('tl-work-more')).toBeNull();
  });

  it('a short list mounts whole, no pager', () => {
    const r = render(() => <WorkNodes nodes={[tool(1), tool(2)]} ctx={ctx} />);
    expect(r.queryByTestId('tl-work-more')).toBeNull();
  });
});

describe('long tool output: line window', () => {
  it('lineWindow splits at a line boundary and counts the rest', () => {
    expect(lineWindow('a\nb\nc', 5)).toEqual({ text: 'a\nb\nc', more: 0 });
    expect(lineWindow('a\nb\nc\nd\ne', 2)).toEqual({ text: 'a\nb', more: 3 });
    expect(lineWindow('a\nb\n', 2)).toEqual({ text: 'a\nb\n', more: 0 });
  });

  it('the expanded full output mounts OUT_LINES lines, then pages 4x', () => {
    const total = OUT_LINES * 6;
    const output = Array.from({ length: total }, (_, i) => `line ${i}`).join('\n');
    const tool = { id: 't', name: 'bash', args: { command: 'x' }, state: 'ok', output } as unknown as ToolRun;
    const r = render(() => <RunBody tool={tool} ctx={fakeCtx([])} />);
    fireEvent.click(r.getByTestId('tool-full-output'));
    const lines = () => r.container.querySelector('.tool-term-out')!.textContent!.split('\n').length;
    expect(lines()).toBe(OUT_LINES);
    expect(r.getByTestId('tool-more-lines').textContent).toBe(`Show ${OUT_LINES * 4} more lines`);
    fireEvent.click(r.getByTestId('tool-more-lines'));
    expect(lines()).toBe(OUT_LINES * 5);
    fireEvent.click(r.getByTestId('tool-more-lines'));
    expect(lines()).toBe(total);
    expect(r.queryByTestId('tool-more-lines')).toBeNull();
  });
});
