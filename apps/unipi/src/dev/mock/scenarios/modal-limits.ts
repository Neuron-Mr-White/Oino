/**
 * `modal-limits` (WP-A17, G-16): the watchdog Limits section with a live
 * store — `caps.get`/`caps.set` mutate in memory, so a capture can save a
 * cap and see "set for this session". Setup opens the modal on Info.
 */
import type { MockScenario } from '../host';
import { genCards, genSession } from '../gen';
import { openSessionModal } from '../../../components/session/modalTarget';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';
const base = genSession('s001', { seed: 4, turns: 3, cwd: CWD });

const OFF = { maxTurns: null, maxCostUsd: null, maxWallMinutes: null, maxConsecutiveToolErrors: null };
const pins: { caps: Record<string, unknown> } = { caps: { ...OFF, summaryTurn: true } };

const num = (v: unknown): number | null => {
  const n = typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : NaN;
  return Number.isFinite(n) && n > 0 ? n : null;
};

export const modalLimits: MockScenario = {
  name: 'modal-limits',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(3, { seed: 2 }).map((c) => ({ ...c, cwd: CWD })),
      sessions: {
        s001: {
          ...base,
          cmd: {
            get_session_stats: () => ({
              sessionFile: '/home/you/.oino/sessions/s001.jsonl',
              sessionId: 's001',
              userMessages: 3,
              assistantMessages: 3,
              toolCalls: 4,
              toolResults: 4,
              totalMessages: 12,
              tokens: { input: 4_100, output: 2_900, cacheRead: 12_000, cacheWrite: 900, total: 19_900 },
              cost: 0.2144,
              contextUsage: { tokens: 19_900, contextWindow: 200_000, percent: 10.0 },
            }),
            get_commands: () => ({ commands: [] }),
            '_oino/extensions': () => ({ extensions: [{ id: 'unipi.watchdog', kind: 'extension', enabled: true, loaded: true }] }),
            '_oino/service_call': (cmd) => {
              if (cmd.name === 'caps.get') return { caps: { ...pins.caps }, pinned: pins.caps.maxTurns != null || pins.caps.maxCostUsd != null };
              if (cmd.name === 'caps.set') {
                const p = (cmd.payload ?? {}) as Record<string, unknown>;
                pins.caps = {
                  maxTurns: num(p.maxTurns),
                  maxCostUsd: num(p.maxCostUsd),
                  maxWallMinutes: num(p.maxWallMinutes),
                  maxConsecutiveToolErrors: num(p.maxConsecutiveToolErrors),
                  summaryTurn: typeof p.summaryTurn === 'boolean' ? p.summaryTurn : true,
                };
                return { caps: { ...pins.caps }, pinned: true };
              }
              throw Object.assign(new Error(`Unknown service: ${String(cmd.name ?? '')}`), { code: 'E_NOT_FOUND' });
            },
          },
        },
      },
    },
  ],
  setup: () => {
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
    openSessionModal({ hostId: HOST, sessionId: 's001', tab: 'info' });
  },
};
