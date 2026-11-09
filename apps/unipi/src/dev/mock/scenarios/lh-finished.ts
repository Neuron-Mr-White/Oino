import type { MockScenario } from '../host';
import { lhService, lhStatusWorld } from './lh';
import { host, LH_EXTENSIONS, seedSessionRoute, topic } from './status';

const world = lhStatusWorld();
const graph = world.current!;
graph.graphId = 'graph-oin57';
graph.task = 'Review the finished graph';
graph.state = 'finished';
graph.work.forEach((w, i) => { w.status = i < 5 ? 'stopped' : i === 5 ? 'superseded' : 'blocked'; });
graph.counts = { queued: 0, running: 0, blocked: 1, errored: 0, completed: 0, failed: 0, stopped: 5, cancelled: 0, superseded: 1 };

export const lhFinished: MockScenario = {
  name: 'lh-finished',
  hosts: [host([
    topic('unipi:long-horizon:state', { mode: 'graph', owner: { kind: 'graph', status: 'active' } }),
    topic('unipi:long-horizon:progress', { mode: 'graph', graph: { graphId: graph.graphId, mode: 'graph', task: graph.task, counts: graph.counts } }),
  ], {
    '_oino/extensions': () => ({ extensions: LH_EXTENSIONS }),
    '_oino/agents': () => ({ agents: [] }),
    '_oino/service_call': (cmd) => lhService(world)(cmd),
    prompt: () => ({}),
  })],
  setup: () => {
    seedSessionRoute();
    localStorage.setItem('unipi.panes:this-pc|/home/you/projects/unipi', JSON.stringify({
      side: { tabs: [{ id: 'tl', kind: 'status', params: { section: 'lh', hostId: 'this-pc', path: '/home/you/projects/unipi' } }], active: { s001: 'tl' }, closed: [] },
      bottom: { tabs: [], active: {}, closed: [] },
    }));
  },
};
