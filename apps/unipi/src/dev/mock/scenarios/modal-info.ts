/**
 * `modal-info` (WP-A17, G-13): the `/info` path — the reserved
 * `unipi.info-screen/dashboard` overlay entry rides the session snapshot's
 * registry, so the modal opens through the reserved route (no `setup` open),
 * at the Info tab. `caps.get` answers, so the Limits section shows too.
 */
import type { MockScenario } from '../host';
import { genCards, genSession } from '../gen';
import { sessions } from '../../../lib/session/store';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';
const base = genSession('s001', { seed: 4, turns: 5, toolsPerTurn: 1, thinking: true, cwd: CWD });

const COMMANDS = [
  { name: 'compact', description: 'Compact the session', source: 'builtin' },
  { name: 'model', description: 'Pick the model', source: 'builtin' },
  { name: 'info', description: 'Open the info screen', source: 'extension:unipi.info-screen' },
  { name: 'bg', description: 'Run a command in the background', source: 'extension:unipi.background-tasks' },
  { name: 'skill:refactor-plan', description: 'Draft a refactor plan before touching code', source: 'skill', sourceInfo: { scope: 'project' } },
  { name: 'skill:tidy-readme', description: 'Rewrite a README from the code', source: 'skill', sourceInfo: { path: '/home/you/.oino/skills/tidy-readme' } },
  { name: 'skill:release-check', description: 'Run the pre-release checklist', source: 'skill', sourceInfo: { path: '/opt/unipi/bundled/unipi.release-check', origin: 'package' } },
];

export const modalInfo: MockScenario = {
  name: 'modal-info',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(6, { seed: 8 }).map((c) => ({ ...c, cwd: CWD })),
      sessions: {
        s001: {
          snapshot: {
            ...base.snapshot,
            // `/info usage`: the reserved overlay entry (ui-protocol §8) with its page param.
            ui: {
              scope: 'session',
              sessionId: 's001',
              rev: 3,
              dialogs: [],
              entries: [
                {
                  id: 'unipi.info-screen/dashboard',
                  extensionId: 'unipi.info-screen',
                  scope: 'session',
                  slot: 'overlay',
                  order: 0,
                  seq: 1,
                  title: 'Info',
                  focus: true,
                  closeOnEscape: true,
                  rev: 1,
                  view: { kind: 'reserved', component: 'sessionInfo', params: { page: 'usage' } },
                },
              ],
            },
          },
          cmd: {
            get_session_stats: () => ({
              sessionFile: '/home/you/.oino/sessions/s001.jsonl',
              sessionId: 's001',
              userMessages: 6,
              assistantMessages: 5,
              toolCalls: 11,
              toolResults: 11,
              totalMessages: 28,
              tokens: { input: 9_800, output: 6_400, cacheRead: 31_000, cacheWrite: 4_100, total: 51_300 },
              cost: 0.8421,
              contextUsage: { tokens: 51_300, contextWindow: 200_000, percent: 25.7 },
            }),
            get_commands: () => ({ commands: COMMANDS }),
            '_oino/extensions': () => ({
              extensions: [
                { id: 'unipi.info-screen', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/info.wasm' },
                { id: 'unipi.background-tasks', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/bg.wasm' },
                { id: 'unipi.watchdog', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/wd.wasm' },
                { id: 'unipi.notify', kind: 'extension', enabled: true, loaded: false },
              ],
            }),
            '_oino/service_call': (cmd) => {
              if (cmd.name === 'caps.get') {
                return { caps: { maxTurns: null, maxCostUsd: null, maxWallMinutes: null, maxConsecutiveToolErrors: null, summaryTurn: true }, pinned: false };
              }
              throw Object.assign(new Error(`Unknown service: ${String(cmd.target ?? '')}/${String(cmd.name ?? '')}`), { code: 'E_NOT_FOUND' });
            },
          },
        },
      },
    },
  ],
  // Nothing to open: the snapshot's reserved entry opens the modal. But the
  // session must be streaming for the registry to see it — nothing on the
  // `/s/:host/:session` route ensures it yet (A4/A6), so the scenario does.
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    // Deferred: this file is reached from `lib/host/factory` (the capture plugin
    // injects the mock boot there), which is still initializing at this point.
    queueMicrotask(() => {
      const handle = sessions().ensure(HOST, 's001');
      handle.acquire();
    });
  },
};
