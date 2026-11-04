/**
 * Status-pane section scenarios (WP-A16):
 *
 *   status-lh-graph  the LH section over a running agent graph (mode chip,
 *                    task, counts bar, Stop/History) + the capsule's LH chip
 *   status-todos     the Plan & todos section: plan mode on, a plan file and
 *                    a todowrite checklist mid-run
 *   status-running   the compact Running summary (bash + agents + sidekick +
 *                    work-tray rows) over the same data as the Session modal
 *   capsule          the collapsed capsule alone (pane closed): branch ±,
 *                    LH progress, running count
 *
 * The session seeds its sticky topics (LH_STATE / LH_PROGRESS /
 * WORKFLOW_STATUS / bg + subagents) the way the host's snapshot would, and
 * `_oino/topics` re-answers them for late subscribers.
 */
import type { MockGitFile, MockScenario, MockSession } from '../host';
import type { RpcCommand } from '../../../lib/host/client';
import { genCards, genSession } from '../gen';
import { desktopShell } from '../../../lib/layout';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';

export const topic = (name: string, data: unknown) => ({ topic: name, data, sticky: true, publisher: 'unipi.long-horizon' });

export const LH_EXTENSIONS = [
  { id: 'unipi.long-horizon', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/lh.wasm' },
  { id: 'unipi.workflow', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/wf.wasm' },
  { id: 'unipi.background-tasks', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/bg.wasm' },
  { id: 'unipi.subagents', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/sa.wasm' },
  { id: 'unipi.fusion', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/fusion.wasm' },
  { id: 'unipi.dream', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/dream.wasm' },
];

const GRAPH_COUNTS = { queued: 1, running: 2, blocked: 0, completed: 3, failed: 1, stopped: 0, cancelled: 0, superseded: 0 };

const TODOS = [
  { content: 'Re-home the LH section into the status pane', status: 'completed' },
  { content: 'Feed it from the sticky topics', status: 'in_progress' },
  { content: 'Capsule: branch ±, LH progress, running count', status: 'pending' },
  { content: 'Delete the Progress sheet', status: 'pending', priority: 'high' },
  { content: 'Phone layer: sections under Git', status: 'pending' },
];

/** A session whose assistant called `todowrite` (the Plan section's source). */
const TODO_ENTRIES = [
  { type: 'message', id: 'e1', message: { role: 'user', content: 'Port the status sections.', timestamp: 1_759_999_000 } },
  { type: 'message', id: 'e2', customType: 'unipi:plan-mode', data: { active: true, planFile: `${CWD}/plans/2026-02-14-a16status.md` } },
  {
    type: 'message',
    id: 'e3',
    message: {
      role: 'assistant',
      stopReason: 'toolUse',
      timestamp: 1_759_999_100,
      content: [{ type: 'toolCall', id: 't1', name: 'todowrite', arguments: { todos: TODOS } }],
    },
  },
  { type: 'message', id: 'e4', message: { role: 'toolResult', toolCallId: 't1', toolName: 'todowrite', isError: false, content: [{ type: 'text', text: 'Todos updated.' }] } },
];

const BG_BODY = {
  t: 'Layout',
  direction: 'Vertical',
  spacing: 1,
  children: [
    {
      t: 'List',
      onActivate: 'bg:open',
      items: [
        { key: 'bg-1', content: { lines: [[{ content: 'Shell', style: { fg: 'dim' } }, ' ', { content: 'cargo test -p unipi', style: { addModifier: ['BOLD'] } }]] }, badge: '2m 12s · wakes agent' },
      ],
    },
  ],
};

const DREAM_BODY = {
  t: 'List',
  onActivate: 'dream:open',
  items: [{ key: 'd-1', content: { lines: [[{ content: 'Dream', style: { fg: 'dim' } }, ' ', { content: 'Tidy the docs folder', style: { addModifier: ['BOLD'] } }]] } }],
};

function serviceCall(cmd: RpcCommand): unknown {
  const name = String(cmd.name ?? '');
  const target = typeof cmd.target === 'string' ? cmd.target : undefined;
  if (name === 'work-tray.tab') {
    if (target === 'unipi.background-tasks') return [{ id: 'bg', label: 'Background tasks', order: 10, counts: { total: 1, running: 1 }, visible: true, body: BG_BODY, owner: 'unipi.background-tasks' }];
    if (target === 'unipi.dream') return [{ id: 'dream', label: 'Dream', order: 30, counts: { total: 1, running: 0 }, visible: true, body: DREAM_BODY, owner: 'unipi.dream' }];
  } else if (name === 'fusion.status' && target === 'unipi.fusion') {
    return { leadName: 'GLM 5.3', sidekickName: 'Flash', busy: true };
  } else if (name === 'work-tray.action') {
    return { close: false, refresh: true };
  }
  throw Object.assign(new Error(`Unknown service: ${target ?? ''}/${name}`), { code: 'E_NOT_FOUND' });
}

const RUNNING_CMDS: MockSession['cmd'] = {
  '_oino/extensions': () => ({ extensions: LH_EXTENSIONS }),
  '_oino/running_bash': () => ({ calls: [{ toolCallId: 'tc-9', command: 'cargo test -p unipi-host', startedAt: Date.now() - 38_000, pid: 4242 }] }),
  '_oino/agents': () => ({
    agents: [
      { id: 'ag-1', label: 'Subagent · map the status topics', owner: 'unipi.subagents', role: 'subagent_explore', state: 'running', depth: 1 },
      { id: 'ag-2', parentId: 'ag-1', label: 'read lh.rs', owner: 'unipi.subagents', state: 'idle', depth: 2 },
    ],
  }),
  '_oino/service_call': (cmd) => serviceCall(cmd),
  '_oino/invoke_shortcut': () => ({}),
  '_oino/agent_abort': () => ({}),
  abort: () => ({}),
};

const FILES: MockGitFile[] = [
  { path: 'src/components/status/LhSection.tsx', area: 'unstaged', status: 'M', add: 96, del: 12 },
  { path: 'src/components/status/StatusPane.tsx', area: 'unstaged', status: 'M', add: 44, del: 6 },
  { path: 'src/lib/status/lh.ts', area: 'untracked', status: '?', add: 128, del: 0 },
];

const git = { repo: true, root: CWD, branch: 'v2/status', head: 'f3d2a1b', upstream: 'origin/v2/status', ahead: 3, behind: 0, remotes: ['origin'], stashes: 0, base: 'origin/v2/status', files: FILES, commits: [] };

export function host(topics: unknown[], cmd?: MockSession['cmd'], runState: 'idle' | 'working' = 'working') {
  const base = genSession('s001', { seed: 9, turns: 3, toolsPerTurn: 1, cwd: CWD });
  const session: MockSession = {
    ...base,
    snapshot: {
      ...base.snapshot,
      state: { ...(base.snapshot.state as object), isStreaming: runState === 'working' },
      topics,
      commands: [...(base.snapshot.commands ?? []), { name: 'goal', description: 'Goal mode' }, { name: 'graph', description: 'Graph mode' }, { name: 'ralph', description: 'Ralph loop' }, { name: 'regular', description: 'Plain mode' }],
    },
    ...(cmd ? { cmd } : {}),
  };
  return {
    id: HOST,
    name: 'workstation',
    transport: 'local' as const,
    conn: { state: 'connected' as const },
    cards: genCards(3, { seed: 4 }).map((c) => ({ ...c, cwd: CWD, run_state: c.session_id === 's001' ? runState : c.run_state })),
    sessions: { s001: session },
    git,
  };
}

/** Seeds the desktop side pane on the `git` tab (+ the last route). */
function seedPane(): void {
  localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: true, sideW: 480, bottomOpen: false, bottomH: 220 }));
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
  localStorage.setItem(`unipi.panes:${HOST}|${CWD}`, JSON.stringify({
    side: { tabs: [{ id: 'tg', kind: 'git', params: { path: CWD } }], active: { s001: 'tg' }, closed: [] },
    bottom: { tabs: [], active: {}, closed: [] },
  }));
}

/** The phone has no side pane: the last route + the pane closed (the capsule opens the layer). */
function seedNoPane(): void {
  localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 480, bottomOpen: false, bottomH: 220 }));
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
}

/** The app's `/` route reopens the last session (`unipi.shell.last`). */
export function seedSessionRoute(): void {
  if (desktopShell()) seedPane();
  else seedNoPane();
}

export const statusLhGraph: MockScenario = {
  name: 'status-lh-graph',
  hosts: [
    host(
      [
        topic('unipi:long-horizon:state', { mode: 'graph', owner: { kind: 'graph', status: 'active' } }),
        topic('unipi:long-horizon:progress', {
          mode: 'graph',
          graph: { graphId: 'g-77', mode: 'graph', task: 'Land the status pane sections (WP-A16)', counts: GRAPH_COUNTS },
          todos: '1/5 \u25b6 Feed it from the sticky topics',
        }),
      ],
      { '_oino/extensions': () => ({ extensions: LH_EXTENSIONS }), prompt: () => ({}) },
    ),
  ],
  setup: seedSessionRoute,
};

const TODO_TOPICS = [
  topic('unipi:workflow:status', { planMode: true, permissionMode: 'default' }),
  topic('unipi:long-horizon:state', { mode: 'none' }),
  topic('unipi:long-horizon:progress', { mode: 'none', todos: '1/5 \u25b6 Feed it from the sticky topics' }),
];

export const statusTodos: MockScenario = {
  name: 'status-todos',
  hosts: [
    {
      ...host(TODO_TOPICS, undefined, 'idle'),
      sessions: {
        s001: (() => {
          const base = genSession('s001', { seed: 9, turns: 3, toolsPerTurn: 1, cwd: CWD });
          return {
            ...base,
            snapshot: { ...base.snapshot, entries: TODO_ENTRIES, leaf_id: 'e4', topics: TODO_TOPICS },
            entries: { entries: TODO_ENTRIES, leafId: 'e4' },
            cmd: { '_oino/extensions': () => ({ extensions: LH_EXTENSIONS }) },
          } as MockSession;
        })(),
      },
    },
  ],
  setup: seedSessionRoute,
};

export const statusRunning: MockScenario = {
  name: 'status-running',
  hosts: [
    host(
      [
        topic('unipi:long-horizon:state', { mode: 'none' }),
        topic('unipi:bg:tasks', { tasks: [{ id: 'bg-1', command: 'cargo test -p unipi-host', status: 'running' }] }),
        topic('unipi:subagents:list', { records: [{ id: 'ag-1', title: 'map the status topics', status: 'running' }] }),
      ],
      RUNNING_CMDS,
    ),
  ],
  setup: seedSessionRoute,
};

export const capsuleScenario: MockScenario = {
  name: 'capsule',
  hosts: [
    host(
      [
        topic('unipi:long-horizon:state', { mode: 'graph', owner: { kind: 'graph', status: 'active' } }),
        topic('unipi:long-horizon:progress', { mode: 'graph', graph: { graphId: 'g-77', mode: 'graph', task: 'Land the status pane sections', counts: GRAPH_COUNTS } }),
        topic('unipi:bg:tasks', { tasks: [{ id: 'bg-1', command: 'cargo test', status: 'running' }] }),
      ],
      { '_oino/extensions': () => ({ extensions: LH_EXTENSIONS }) },
    ),
  ],
  setup: seedNoPane,
};
