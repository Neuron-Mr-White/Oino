/**
 * `modal-running` (WP-A17): the full Running view — a foreground bash call,
 * two nested subagents (one running, one waiting), a live Fusion handoff,
 * and the work-tray tabs (background tasks + dream) as trees. Actions are
 * recorded on the mock log; `window.__UNIPI_MOCK_LOG__` shows what the app
 * sends (`abort`, `prompt` = `/bg-detach`, `_oino/agent_abort`,
 * `_oino/invoke_shortcut`, `_oino/service_call` → `work-tray.action`).
 */
import type { MockScenario } from '../host';
import type { RpcCommand } from '../../../lib/host/client';
import { genCards, genSession } from '../gen';
import { openSessionModal } from '../../../components/session/modalTarget';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';
const base = genSession('s001', { seed: 4, turns: 4, toolsPerTurn: 2, thinking: true, cwd: CWD });

/** The bg tab's tree: rows + per-row actions, exactly as the extension builds them. */
const bgBody = {
  t: 'Layout',
  direction: 'Vertical',
  spacing: 1,
  children: [
    {
      t: 'List',
      onActivate: 'bg:open',
      items: [
        { key: 'bg-1', content: { lines: [[{ content: 'Shell', style: { fg: 'dim' } }, ' ', { content: 'cargo test', style: { addModifier: ['BOLD'] } }]] }, detail: 'cargo test --workspace', badge: '2m 12s · 1.2 MB · wakes agent' },
        { key: 'bg-2', content: { lines: [[{ content: 'Shell', style: { fg: 'dim' } }, ' ', { content: 'npm run build', style: { addModifier: ['BOLD'] } }]] }, detail: 'npm run build', badge: 'exit 0' },
      ],
    },
    {
      t: 'Layout',
      direction: 'Horizontal',
      spacing: 2,
      children: [
        { t: 'Button', label: 'Stop', action: 'bg:stop:bg-1' },
        { t: 'Button', label: 'Kill', action: 'bg:kill:bg-1', variant: 'Danger', confirm: 'Kill this task?' },
        { t: 'Button', label: 'Re-run', action: 'bg:rerun:bg-2' },
      ],
    },
  ],
};

const dreamBody = {
  t: 'List',
  onActivate: 'dream:open',
  items: [
    { key: 'd-1', content: { lines: [[{ content: 'Dream', style: { fg: 'dim' } }, ' ', { content: 'Tidy the docs folder', style: { addModifier: ['BOLD'] } }]] }, badge: '3 runs · last 14:02' },
  ],
};

const EXTENSIONS = [
  { id: 'unipi.background-tasks', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/bg.wasm' },
  { id: 'unipi.subagents', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/sa.wasm' },
  { id: 'unipi.dream', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/dream.wasm' },
  { id: 'unipi.fusion', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/fusion.wasm' },
  { id: 'unipi.watchdog', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/wd.wasm' },
  { id: 'unipi.info-screen', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/info.wasm' },
];

/** Answers `_oino/service_call` the way the host would: only the extensions that
 * actually serve a name answer; everyone else errors `E_NOT_FOUND`. */
function serviceCall(cmd: RpcCommand): unknown {
  const name = String(cmd.name ?? '');
  const target = typeof cmd.target === 'string' ? cmd.target : undefined;
  if (name === 'work-tray.tab') {
    if (target === 'unipi.background-tasks') {
      return [{ id: 'bg', label: 'Background tasks', order: 10, counts: { total: 2, running: 1 }, visible: true, body: bgBody, owner: 'unipi.background-tasks' }];
    }
    if (target === 'unipi.dream') {
      return [{ id: 'dream', label: 'Dream', order: 30, counts: { total: 1, running: 0 }, visible: true, body: dreamBody, owner: 'unipi.dream' }];
    }
  } else if (name === 'fusion.status' && target === 'unipi.fusion') {
    return { leadName: 'GLM 5.3', sidekickName: 'Flash', leadEffort: 'high', sidekickEffort: 'high', busy: true, savedUsd: 0.42 };
  } else if (name === 'caps.get' && target === 'unipi.watchdog') {
    return { caps: { maxTurns: null, maxCostUsd: null, maxWallMinutes: null, maxConsecutiveToolErrors: null, summaryTurn: true }, pinned: false };
  } else if (name === 'work-tray.action') {
    return { close: false, refresh: true };
  }
  throw Object.assign(new Error(`Unknown service: ${target ?? ''}/${name}`), { code: 'E_NOT_FOUND' });
}

export const modalRunning: MockScenario = {
  name: 'modal-running',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(4, { seed: 5 }).map((c) => ({ ...c, cwd: CWD, run_state: c.session_id === 's001' ? 'working' : c.run_state })),
      sessions: {
        s001: {
          ...base,
          // The watchdog's detach command is registered (its availability gates
          // the "Send to background" row action).
          snapshot: {
            ...base.snapshot,
            commands: [{ name: 'bg-detach', description: 'Move the running bash call to a background task', source: 'extension:unipi.watchdog' }],
          },
          cmd: {
            '_oino/running_bash': () => ({
              calls: [{ toolCallId: 'tc-9', command: 'cargo test -p unipi-watchdog', startedAt: Date.now() - 45_000, pid: 4242 }],
            }),
            '_oino/agents': () => ({
              agents: [
                { id: 'ag-1', label: 'Subagent · explore the layout code', owner: 'unipi.subagents', role: 'subagent_explore', state: 'running', depth: 1 },
                { id: 'ag-2', parentId: 'ag-1', label: 'read files', owner: 'unipi.subagents', state: 'idle', depth: 2 },
              ],
            }),
            '_oino/extensions': () => ({ extensions: EXTENSIONS }),
            '_oino/service_call': (cmd) => serviceCall(cmd),
            '_oino/agent_abort': () => ({}),
            '_oino/invoke_shortcut': () => ({}),
            abort: () => ({}),
          },
        },
      },
    },
  ],
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    openSessionModal({ hostId: HOST, sessionId: 's001', tab: 'running' });
  },
};
