/**
 * `timeline-many` (WP-A6): an 80-turn session (virtualized history) with the
 * full row mix — thinking, tools, diffs, a compaction divider, a steer, an
 * image — and a *running* live tail streaming at the bottom (speed=0 lands
 * it at once, live=1 streams). `s002` is a short session for scroll-memory
 * switching shots.
 */
import type { MockScenario } from '../host';
import { genCards, genFsTree, genLiveRun, genSession } from '../gen';

const long = genSession('s001', {
  seed: 11,
  turns: 80,
  toolsPerTurn: 2,
  thinking: true,
  diffs: true,
  compaction: true,
  steers: true,
  images: true,
  cwd: '/home/you/projects/unipi',
});

const short = genSession('s002', { seed: 12, turns: 6, toolsPerTurn: 1, cwd: '/home/you/projects/oino' });

export const timelineMany: MockScenario = {
  name: 'timeline-many',
  hosts: [
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(8),
      fs: genFsTree(),
      sessions: {
        s001: { ...long, events: genLiveRun('s001', { seq: 1, running: true }) },
        s002: short,
      },
    },
  ],
};
