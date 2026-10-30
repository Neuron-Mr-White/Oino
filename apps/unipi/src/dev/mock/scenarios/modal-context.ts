/**
 * `modal-context` (WP-A17): a quiet session whose context numbers are worth
 * reading — 42 % of the window, a healthy cache-hit rate, a real cost.
 * Setup opens the modal on the Context tab.
 */
import type { MockScenario } from '../host';
import { genCards, genSession } from '../gen';
import { openSessionModal } from '../../../components/session/modalTarget';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';
const base = genSession('s001', { seed: 4, turns: 6, toolsPerTurn: 2, thinking: true, cwd: CWD });

export const modalContext: MockScenario = {
  name: 'modal-context',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(5, { seed: 3 }).map((c) => ({ ...c, cwd: CWD })),
      sessions: {
        s001: {
          ...base,
          cmd: {
            '_oino/get_context_usage': () => ({ tokens: 84_213, contextWindow: 200_000, percent: 42.1 }),
            get_session_stats: () => ({
              sessionFile: '/home/you/.oino/sessions/s001.jsonl',
              sessionId: 's001',
              userMessages: 7,
              assistantMessages: 6,
              toolCalls: 23,
              toolResults: 23,
              totalMessages: 36,
              tokens: { input: 12_400, output: 9_100, cacheRead: 52_300, cacheWrite: 8_400, total: 82_200 },
              cost: 1.2345,
              contextUsage: { tokens: 84_213, contextWindow: 200_000, percent: 42.1 },
            }),
            compact: () => ({}),
            '_oino/service_call': (cmd) => {
              throw Object.assign(new Error(`Unknown service: ${String(cmd.target ?? '')}/${String(cmd.name ?? '')}`), { code: 'E_NOT_FOUND' });
            },
          },
        },
      },
    },
  ],
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    openSessionModal({ hostId: HOST, sessionId: 's001', tab: 'context' });
  },
};
