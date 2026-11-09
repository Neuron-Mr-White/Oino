/** Three concurrent Superagent sessions: two dispatched leads and one manual chat. */
import type { MockScenario } from '../host';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
const started = Date.now() - 12 * 60_000;
export const superagentConcurrent: MockScenario = {
  name: 'superagent-concurrent',
  hosts: [{
    ...base,
    sessions: { ...base.sessions, 'superagent-chat': { ...base.sessions!['superagent-chat']!, snapshot: { ...base.sessions!['superagent-chat']!.snapshot, state: { ...base.sessions!['superagent-chat']!.snapshot.state as object, isStreaming: true } } } },
    ops: {
      ...base.ops,
      'superagent.sessions': () => [
        { sessionId: 'superagent-chat', title: 'Manual investigation', project: 'Website', createdAt: started, updatedAt: started, path: '/mock/superagent-chat', state: { kind: 'working' } },
        { sessionId: 'lead-hp', title: 'Lead · Heartpick', project: 'Heartpick', createdAt: started, updatedAt: started, path: '/mock/lead-hp', state: { kind: 'working' } },
        { sessionId: 'old-chat', title: 'Previous investigation', project: 'Website', createdAt: started - 86400000, updatedAt: started - 80000000, path: '/mock/old', state: { kind: 'idle' } },
      ],
    },
  }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
