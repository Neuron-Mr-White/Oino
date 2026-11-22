import type { MockScenario } from '../host';
import { genCards, genSession } from '../gen';
import { openSessionModal } from '../../../components/session/modalTarget';

const host = 'this-pc';
const records = Array.from({ length: 5 }, (_, i) => ({ id: `run-${i}`, title: ['Explore layout', 'Inspect status', 'Review tests', 'Audit session feed', 'Check ownership'][i], parentSessionId: 's001', profile: 'subagent_explore', status: i < 3 ? 'running' : 'completed', background: true, startedAt: Date.now() - 60000 - i * 1000, toolCalls: i + 1, depth: 1 }));
const session = (id: string) => {
  const base = genSession(id, { seed: 4, turns: 2, cwd: '/home/you/projects/unipi' });
  return {
    ...base,
    snapshot: { ...base.snapshot, topics: [{ topic: 'unipi:subagents:list', data: { parentSessionId: id, records: id === 's001' ? records : [] }, sticky: true, publisher: 'unipi.subagents' }] },
    cmd: {
      '_oino/running_bash': () => ({ calls: [] }),
      '_oino/agents': () => ({ agents: [] }),
      '_oino/extensions': () => ({ extensions: [{ id: 'unipi.subagents', loaded: true, enabled: true }] }),
      '_oino/service_call': () => id === 's001' ? records : [],
    },
  };
};
export const oin42: MockScenario = {
  name: 'oin42',
  hosts: [{ id: host, name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: genCards(2, { seed: 5 }), sessions: { s001: session('s001'), s002: session('s002') } }],
  setup: () => {
    const selected = new URLSearchParams(location.search).get('session') === 'B' ? 's002' : 's001';
    localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: host, sessionId: selected }));
    openSessionModal({ hostId: host, sessionId: selected, tab: 'running' });
  },
};
