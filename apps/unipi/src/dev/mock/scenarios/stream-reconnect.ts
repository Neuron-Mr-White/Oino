/**
 * `stream-reconnect` (WP-A2, Z-50): the stream drops mid-run and the host
 * stays unreachable — the session store retries with `sinceSeq` (0.5/1/2/5 s)
 * and shows "Lost the connection · Retry"; prompts typed meanwhile wait in the
 * outbox ("Waiting for connection… ✕"). With `live=1` the drop comes after
 * the first tool call; `down` keeps the host away for good.
 */
import type { MockScenario, MockStep } from '../host';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

const base = genSession('s001', { seed: 6, turns: 3, toolsPerTurn: 2, thinking: true, cwd: '/home/you/projects/unipi' });
const run = genLiveRun('s001', { seq: 1, running: true });
const events: MockStep[] = [...run.slice(0, 9), { at: run[8]!.at + 50, drop: { message: 'The host stopped answering', down: Infinity } }];

export const streamReconnect: MockScenario = {
  name: 'stream-reconnect',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3).map((c, i) => (i === 0 ? { ...c, run_state: 'working' as const } : c)),
      fs: genFsTree(),
      sessions: { s001: { ...base, events } },
    },
  ],
};
