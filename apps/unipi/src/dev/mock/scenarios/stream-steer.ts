/**
 * `stream-steer` (WP-A2, Z-54): a run where a steer arrives after the first
 * tool result — the turn model opens a second segment in the same turn; the
 * queue strip sees it as a steering message before pi takes it.
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

const base = genSession('s001', { seed: 5, turns: 2, toolsPerTurn: 1, cwd: '/home/you/projects/unipi' });

export const streamSteer: MockScenario = {
  name: 'stream-steer',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3).map((c, i) => (i === 0 ? { ...c, run_state: 'working' as const } : c)),
      fs: genFsTree(),
      sessions: { s001: { ...base, events: genLiveRun('s001', { seq: 1, steer: 'Also keep the public API unchanged.' }) } },
    },
  ],
};
