/**
 * `stream-basic` (WP-A2): one PC with a few session cards; `s001` opens with
 * a 4-turn snapshot and replays a live run (thinking, a tool, a streamed
 * answer) on the virtual clock (`speed=0` settles at once, `live=1` streams).
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

const base = genSession('s001', { seed: 4, turns: 4, toolsPerTurn: 2, thinking: true, diffs: true, cwd: '/home/you/projects/unipi' });

export const streamBasic: MockScenario = {
  name: 'stream-basic',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(6),
      fs: genFsTree(),
      sessions: { s001: { ...base, events: genLiveRun('s001', { seq: 1 }) } },
    },
  ],
};
