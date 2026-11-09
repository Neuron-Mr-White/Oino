/** Three concurrent Superagent sessions: two dispatched leads and one manual chat. */
import type { MockScenario } from '../host';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
const started = Date.now() - 12 * 60_000;
// Capture variants: 0 = idle, 1 = manual chat, 3 = manual plus two leads.
const count = typeof location === 'undefined' ? 3 : Number(new URLSearchParams(location.search).get('runs') ?? 3);
export const superagentConcurrent: MockScenario = {
  name: 'superagent-concurrent',
  hosts: [{
    ...base,
    cards: count >= 3 ? base.cards : [],
    sessions: { ...base.sessions, 'superagent-chat': { ...base.sessions!['superagent-chat']!, snapshot: { ...base.sessions!['superagent-chat']!.snapshot, state: { ...base.sessions!['superagent-chat']!.snapshot.state as object, isStreaming: count > 0 }, ...(count < 3 ? { entries: [
      { type: 'custom', id: 'sa-kind', parentId: null, customType: 'oino:superagent', data: { workspace: null } },
      { type: 'message', id: 'sa-user', parentId: 'sa-kind', message: { role: 'user', content: 'Check the Website project.' } },
      { type: 'message', id: 'sa-answer', parentId: 'sa-user', message: { role: 'assistant', content: [{ type: 'text', text: count === 0 ? 'The investigation is complete.' : 'I am investigating the Website project.' }], stopReason: 'stop' } },
    ] as never } : {}) } } },
    ops: {
      ...base.ops,
      'superagent.status': (args, host) => {
        const status = base.ops!['superagent.status']!(args, host) as { leads: unknown[] };
        return { ...status, leads: count >= 3 ? status.leads : [] };
      },
      'superagent.sessions': () => [
        { sessionId: 'superagent-chat', title: 'Manual investigation', project: 'Website', createdAt: started, updatedAt: started, path: '/mock/superagent-chat', state: { kind: count > 0 ? 'working' : 'idle' } },
        ...(count >= 3 ? [{ sessionId: 'lead-hp', title: 'Lead · Heartpick', project: 'Heartpick', createdAt: started, updatedAt: started, path: '/mock/lead-hp', state: { kind: 'working' } }] : []),
        { sessionId: 'old-chat', title: 'Previous investigation', project: 'Website', createdAt: started - 86400000, updatedAt: started - 80000000, path: '/mock/old', state: { kind: 'idle' } },
      ],
    },
  }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
