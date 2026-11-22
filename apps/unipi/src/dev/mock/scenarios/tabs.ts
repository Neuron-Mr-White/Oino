/**
 * Session side-tab scenarios (WP-A10): `tab-diff` (Z-40), `tab-tree` (Z-18),
 * `tab-btw` (Z-31) and `tab-subagent` (Z-42) on the desktop side pane, and
 * their phone surfaces (`tab-diff-phone`, `tab-tree-phone`, `tab-btw-phone`,
 * `tab-subagent-phone`: the diff viewer as a full-screen layer, the others as
 * bottom sheets). One session per scenario so the tab is the only thing the
 * capture has to look at.
 */
import type { MockScenario } from '../host';
import { BASE_TIME, genCards, genFsTree, genSession } from '../gen';
import { layout } from '../../../lib/layout';
import { panes } from '../../../lib/panes';
import { seedPhoneSessionTab } from '../../../components/panes/tabs/sessionTabs';
import type { SessionEntry } from '../../../lib/session/types';

const HOST = 'pc-coffee';
const CWD = '/home/you/projects/unipi';
const S = 's-tabs';

const text = (t: string) => [{ type: 'text', text: t }];
const call = (id: string, name: string, args: Record<string, unknown>) => ({ type: 'toolCall', id, name, arguments: args });

/** A small branchy session: an edit with two hunks, a write, a subagent step and a completion card. */
function entries(): SessionEntry[] {
  let t = BASE_TIME;
  const out: SessionEntry[] = [];
  const add = (e: Record<string, unknown>) => {
    t += 3000;
    const entry = { ...e, timestamp: new Date(t).toISOString() } as SessionEntry;
    out.push(entry);
    return entry;
  };
  add({ type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: text('Fix add() and tidy the docs') } });
  add({
    type: 'message',
    id: 'a1',
    parentId: 'u1',
    message: {
      role: 'assistant',
      content: [
        { type: 'thinking', thinking: 'The test expects a + b; the body subtracts.' },
        call('tc-edit', 'edit', {
          path: 'src/math.ts',
          edits: [
            { oldText: 'export function add(a: number, b: number) {\n  return a - b;\n}', newText: 'export function add(a: number, b: number) {\n  return a + b;\n}' },
            { oldText: 'export const VERSION = "1.0.0";', newText: 'export const VERSION = "1.0.1";' },
          ],
        }),
      ],
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      stopReason: 'toolUse',
      usage: { input: 2000, output: 90, totalTokens: 2090, cost: { total: 0.006 } },
    },
  });
  add({ type: 'message', id: 'r1', parentId: 'a1', message: { role: 'toolResult', toolCallId: 'tc-edit', toolName: 'edit', content: text('Successfully replaced 2 blocks in src/math.ts.'), details: { firstChangedLine: 2 } } });
  add({
    type: 'message',
    id: 'a2',
    parentId: 'r1',
    message: { role: 'assistant', content: [call('tc-sub', 'task', { description: 'Map the reducer', prompt: 'Read src/lib/session/reducer.ts and list the slices it owns.' })], provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'toolUse' },
  });
  add({
    type: 'custom',
    id: 'st1',
    parentId: 'a2',
    customType: 'subagent-step',
    data: { kind: 'subagent-step', agentId: 'ag-1', id: 'ag-1', title: 'Map the reducer', profile: 'subagent_explore', name: 'read', arg: 'src/lib/session/reducer.ts' },
  });
  add({
    type: 'custom_message',
    id: 'cm1',
    parentId: 'st1',
    customType: 'subagent-completion',
    data: { title: 'Map the reducer', status: 'completed' },
    message: { role: 'custom', content: text('Subagent · Map the reducer — 4 slices: entries, run, queue, topics.'), details: { owner: 'subagents', id: 'ag-1', title: 'Map the reducer', profile: 'subagent_explore', status: 'completed', toolCalls: 3, durationMs: 42_000 } },
  });
  add({
    type: 'message',
    id: 'a3',
    parentId: 'cm1',
    message: {
      role: 'assistant',
      content: [call('tc-write', 'write', { path: 'docs/NOTES.md', content: '# Notes\n\n- `add` adds (was subtracting)\n- version bump\n' })],
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      stopReason: 'toolUse',
    },
  });
  add({ type: 'message', id: 'r3', parentId: 'a3', message: { role: 'toolResult', toolCallId: 'tc-write', toolName: 'write', content: text('Wrote docs/NOTES.md.') } });
  add({ type: 'message', id: 'u2', parentId: 'r3', message: { role: 'user', content: text('Also check the cli') } });
  add({
    type: 'message',
    id: 'a4',
    parentId: 'u2',
    message: { role: 'assistant', content: text('`add` adds now, the version is bumped and NOTES.md records it.'), provider: 'anthropic', model: 'claude-sonnet-4-5', stopReason: 'stop' },
  });
  return out;
}

/** The `get_tree` reply: the leaf branch plus one abandoned branch. */
const TREE = [
  {
    entry: { type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: text('Fix add() and tidy the docs') } },
    children: [
      {
        entry: { type: 'message', id: 'a1', parentId: 'u1', message: { role: 'assistant', content: text('The test expects a + b; the body subtracts.') } },
        children: [
          {
            entry: { type: 'message', id: 'r1', parentId: 'a1', message: { role: 'toolResult', content: text('Successfully replaced 2 blocks in src/math.ts.') } },
            children: [
              { entry: { type: 'custom', id: 'st1', parentId: 'r1', customType: 'subagent-step', data: { name: 'read', arg: 'reducer.ts' } } },
              { entry: { type: 'message', id: 'u2', parentId: 'r1', message: { role: 'user', content: text('Also check the cli') } } },
            ],
          },
        ],
      },
      { entry: { type: 'message', id: 'b1', parentId: 'u1', message: { role: 'assistant', content: text('Bump the version instead') } } },
    ],
  },
];

const BTW_PAGES = [
  { id: 'p1', question: 'Why is the build slow?', answer: 'A cold cache: the first `vite` run re-bundles the whole app. Later runs reuse it.', done: true },
  { id: 'p2', question: 'Does the reducer re-run per event?', answer: 'Only the tail turn recomputes; settled turns keep their identity (`lib/session/turns.ts`).', done: true },
  { id: 'p3', question: 'What owns the pane tabs?', answer: '', done: false, running: true, thinking: 'lib/panes.ts keys tabs by (hostId, projectKey)…', toolLines: ['read lib/panes.ts'] },
];

const SUBAGENT = [
  { role: 'user', content: text('Read src/lib/session/reducer.ts and list the slices it owns.') },
  { role: 'assistant', content: [{ type: 'thinking', thinking: 'Start at the state type, then the snapshot writer.' }, text('Reading the reducer and the state type.')[0], call('s1', 'read', { path: 'src/lib/session/reducer.ts' })] },
  { role: 'toolResult', toolCallId: 's1', toolName: 'read', content: text('export type SessionState = { seq, entries, run, queue, topics, … }') },
  { role: 'assistant', content: [text('The reducer owns **entries**, **run**, **queue** and **topics** — the turns index is derived.'), call('s2', 'grep', { pattern: 'export function apply', path: 'src/lib/session' })] },
  { role: 'toolResult', toolCallId: 's2', toolName: 'grep', content: text('reducer.ts: 4 hits\napplySnapshot, applyEvents, initial…') },
];

const base = () => genSession(S, { seed: 5, turns: 2, toolsPerTurn: 1, cwd: CWD });

function host(session: ReturnType<typeof genSession>, cmd: Record<string, (c: { type: string }) => unknown> = {}) {
  return {
    ...session,
    snapshot: { ...session.snapshot, entries: entries(), leaf_id: 'a4' },
    cmd: {
      ...(session.cmd ?? {}),
      get_tree: () => ({ tree: TREE, leafId: 'u2' }),
      '_oino/navigate_tree': () => ({ editorText: '> Fix add() and tidy the docs\n' }),
      '_oino/service_call': (c: { type: string }) => (String((c as { name?: string }).name) === 'btw.list' ? BTW_PAGES : BTW_PAGES[0]),
      '_oino/agents': () => ({ agents: [{ id: 'ag-1', label: 'Subagent · Map the reducer', owner: 'unipi.subagents', role: 'subagent_explore', state: 'running', depth: 1 }] }),
      '_oino/agent_messages': () => ({ messages: SUBAGENT }),
      '_oino/agent_abort': () => ({}),
      ...cmd,
    },
  };
}

function scenario(name: string, kind: 'diff' | 'tree' | 'btw' | 'subagent', phone: boolean, params: Record<string, unknown>): MockScenario {
  return {
    name,
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        conn: { state: 'connected' },
        // The shown session's card carries its cwd — the pane scope is that project.
        cards: [{ session_id: S, title: 'Fix add() and tidy the docs', cwd: CWD, last_activity: BASE_TIME }, ...genCards(4, { seed: 7, root: CWD })] as never[],
        fs: genFsTree(CWD),
        sessions: { [S]: host(base()) },
      },
    ],
    setup: () => {
      localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: S }));
      const titles = { diff: 'Diff · math.ts', tree: 'Tree', btw: 'Side chat', subagent: 'Subagent · Map the reducer' } as const;
      if (phone) {
        seedPhoneSessionTab({ kind, base: { hostId: HOST, sessionId: S }, params: { hostId: HOST, sessionId: S, ...params }, title: titles[kind] });
        return;
      }
      layout().toggleSide(true);
      panes().open({ hostId: HOST, projectKey: CWD }, 'side', { sessionId: S }, { kind, params: { hostId: HOST, sessionId: S, ...params }, title: titles[kind] });
    },
  };
}

const DIFF = { toolId: 'tc-edit', path: 'src/math.ts' };
const DIFF_SPLIT = { ...DIFF, split: true };
const SUB = { agentId: 'ag-1', name: 'Subagent · Map the reducer' };

export const tabDiff = scenario('tab-diff', 'diff', false, DIFF);
export const tabTree = scenario('tab-tree', 'tree', false, {});
export const tabBtw = scenario('tab-btw', 'btw', false, {});
export const tabSubagent = scenario('tab-subagent', 'subagent', false, SUB);
export const tabDiffPhone = scenario('tab-diff-phone', 'diff', true, DIFF);
export const tabDiffSplit = scenario('tab-diff-split', 'diff', false, DIFF_SPLIT);
export const tabTreePhone = scenario('tab-tree-phone', 'tree', true, {});
export const tabBtwPhone = scenario('tab-btw-phone', 'btw', true, {});
export const tabSubagentPhone = scenario('tab-subagent-phone', 'subagent', true, SUB);

/** `tab-tree-long`: a 3000-entry chat on the Tree tab's "All" filter (windowed list, virtualizer regression). */
const LONG = Array.from({ length: 3000 }, (_, i) => ({
  entry: {
    type: 'message',
    id: `n${i}`,
    parentId: i === 0 ? null : `n${i - 1}`,
    timestamp: new Date(BASE_TIME + i * 1000).toISOString(),
    message: i % 2 === 0 ? { role: 'user', content: text(`Row ${i}: user question`) } : { role: 'assistant', content: text(`Row ${i}: assistant answer`) },
  },
}));
const longBase = scenario('tab-tree-long', 'tree', false, {});
export const tabTreeLong: MockScenario = {
  ...longBase,
  hosts: longBase.hosts.map((h) => ({ ...h, sessions: { [S]: host(base(), { get_tree: () => ({ tree: LONG, leafId: 'n2999' }) }) } })),
  setup: () => {
    localStorage.setItem('unipi.chat.treeFilter', 'all');
    longBase.setup?.();
  },
};
